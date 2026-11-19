//! Opérations de la console d'admin Rafthub (ADR 0036), **sans HTML** : les pages
//! de `/admin` appellent ces fonctions et n'écrivent jamais de SQL elles-mêmes.
//!
//! Chaque action qui change quelque chose (suspension, réglage, invitation…) laisse
//! une ligne dans le journal de la console (table `admin_events`). Le détail d'une
//! ligne est court et ne contient jamais de mot de passe, de jeton ni d'adresse
//! e-mail complète ([`clean_detail`]).
//!
//! `actor` : identifiant du compte qui agit (l'admin connecté, ou le joueur lui-même
//! pour son inscription et le RGPD) ; `None` pour une action sans compte (ligne de
//! commande, système).

use std::time::Duration;

use serde::Serialize;

use crate::error::HubError;
use crate::session::{new_invite_code, normalize_invite_code, token_hash};
use crate::settings::{PublicSettings, SettingKey};
use crate::store::{AdminEvent, DashboardCounts, InviteRow, Page, PlayerFilter, PlayerSummary};
use crate::{now_unix, AppState};

/// Longueur maximale du détail d'une ligne du journal.
const MAX_DETAIL_CHARS: usize = 200;

/// Types de lignes du journal. Le nom (`as_str`) est rangé en base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    AccountCreated,
    AccountSuspended,
    AccountReactivated,
    SettingChanged,
    InviteCreated,
    InviteRevoked,
    GdprExport,
    GdprDelete,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::AccountCreated => "account_created",
            EventKind::AccountSuspended => "account_suspended",
            EventKind::AccountReactivated => "account_reactivated",
            EventKind::SettingChanged => "setting_changed",
            EventKind::InviteCreated => "invite_created",
            EventKind::InviteRevoked => "invite_revoked",
            EventKind::GdprExport => "gdpr_export",
            EventKind::GdprDelete => "gdpr_delete",
        }
    }
}

/// Code d'invitation tout juste créé : le **seul** moment où le code est connu en
/// clair (la base n'en garde que l'empreinte). À montrer une fois à l'admin.
#[derive(Clone, Serialize)]
pub struct CreatedInvite {
    pub id: i64,
    pub code: String,
    pub expires_at: i64,
}

/// `Debug` sans le code.
impl std::fmt::Debug for CreatedInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedInvite")
            .field("id", &self.id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Tableau de bord : compteurs de la base et joueurs en ligne (mémoire).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Dashboard {
    #[serde(flatten)]
    pub counts: DashboardCounts,
    pub online: i64,
}

/// Masque une adresse e-mail : `alice@exemple.org` → `a…@exemple.org`.
pub fn mask_email(email: &str) -> String {
    match email.split_once('@') {
        Some((local, domain)) => {
            let first: String = local.chars().take(1).collect();
            format!("{first}…@{domain}")
        }
        None => email.to_string(),
    }
}

/// Nettoie le détail d'une ligne du journal : chaque mot qui contient `@` est
/// masqué ([`mask_email`]), les caractères de contrôle deviennent des espaces, et
/// le texte est coupé à 200 caractères.
pub fn clean_detail(detail: &str) -> String {
    let words: Vec<String> = detail
        .split(' ')
        .map(|word| {
            if word.contains('@') {
                mask_email(word)
            } else {
                word.to_string()
            }
        })
        .collect();
    words
        .join(" ")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_DETAIL_CHARS)
        .collect()
}

impl AppState {
    /// Écrit une ligne du journal de la console.
    pub async fn record_event(
        &self,
        actor: Option<i64>,
        kind: EventKind,
        target: Option<i64>,
        detail: &str,
    ) -> Result<(), HubError> {
        self.store()
            .insert_event(
                now_unix(),
                actor,
                kind.as_str(),
                target,
                &clean_detail(detail),
            )
            .await
    }

    /// Comme [`AppState::record_event`], pour une action de joueur déjà faite : un
    /// échec du journal ne doit pas faire échouer l'action, il laisse seulement un
    /// avertissement.
    pub(crate) async fn record_event_quietly(
        &self,
        actor: Option<i64>,
        kind: EventKind,
        target: Option<i64>,
        detail: &str,
    ) {
        if let Err(error) = self.record_event(actor, kind, target, detail).await {
            tracing::warn!(%error, kind = kind.as_str(), "journal de la console non écrit");
        }
    }

    /// Suspend un compte : il ne peut plus se connecter (mot de passe ou Discord),
    /// ses sessions sont fermées, il disparaît des amis, des demandes et de la
    /// présence des autres. Suspendre un compte déjà suspendu ne change rien.
    pub async fn suspend_user(&self, actor: Option<i64>, user_id: i64) -> Result<(), HubError> {
        if actor == Some(user_id) {
            return Err(HubError::SelfSuspend);
        }
        if !self.store().suspend_user(user_id, now_unix()).await? {
            return Err(HubError::UserNotFound);
        }
        self.inner.presence.clear(user_id);
        tracing::info!(user_id, "compte suspendu");
        self.record_event(actor, EventKind::AccountSuspended, Some(user_id), "")
            .await
    }

    /// Lève la suspension. Le joueur se reconnecte ensuite normalement.
    pub async fn reactivate_user(&self, actor: Option<i64>, user_id: i64) -> Result<(), HubError> {
        if !self.store().reactivate_user(user_id).await? {
            return Err(HubError::UserNotFound);
        }
        tracing::info!(user_id, "compte réactivé");
        self.record_event(actor, EventKind::AccountReactivated, Some(user_id), "")
            .await
    }

    /// Change un réglage depuis la console. `None` retire la valeur de la base : on
    /// revient à la variable d'environnement, sinon au défaut. Le journal ne garde
    /// que le nom du réglage, jamais la valeur.
    pub async fn update_setting(
        &self,
        actor: Option<i64>,
        name: &str,
        value: Option<&str>,
    ) -> Result<(), HubError> {
        let key = SettingKey::from_name(name).ok_or(HubError::UnknownSetting)?;
        let detail = match value {
            Some(raw) => {
                let value = key.validate(raw)?;
                self.store()
                    .put_setting(key.name(), &value, now_unix())
                    .await?;
                key.name().to_string()
            }
            None => {
                self.store().delete_setting(key.name()).await?;
                format!("{} (valeur retirée)", key.name())
            }
        };
        tracing::info!(setting = key.name(), "réglage changé");
        self.record_event(actor, EventKind::SettingChanged, None, &detail)
            .await
    }

    /// Réglages sans secret, pour la page « Réglages ».
    pub async fn public_settings(&self) -> Result<PublicSettings, HubError> {
        Ok(self.settings().await?.public())
    }

    /// Crée un code d'invitation valable `lifetime`.
    pub async fn create_invite(
        &self,
        actor: Option<i64>,
        lifetime: Duration,
    ) -> Result<CreatedInvite, HubError> {
        let code = new_invite_code();
        let now = now_unix();
        let lifetime = i64::try_from(lifetime.as_secs()).unwrap_or(i64::MAX);
        let expires_at = now.saturating_add(lifetime);
        let id = self
            .store()
            .insert_invite(
                &token_hash(&normalize_invite_code(&code)),
                actor,
                now,
                expires_at,
            )
            .await?;
        self.record_event(actor, EventKind::InviteCreated, None, &format!("n° {id}"))
            .await?;
        Ok(CreatedInvite {
            id,
            code,
            expires_at,
        })
    }

    /// Retire un code d'invitation (qu'il ait servi ou non).
    pub async fn revoke_invite(&self, actor: Option<i64>, invite_id: i64) -> Result<(), HubError> {
        if !self.store().delete_invite(invite_id).await? {
            return Err(HubError::InviteNotFound);
        }
        self.record_event(
            actor,
            EventKind::InviteRevoked,
            None,
            &format!("n° {invite_id}"),
        )
        .await
    }

    /// Codes d'invitation, du plus récent au plus ancien.
    pub async fn invites(&self, limit: i64, offset: i64) -> Result<Page<InviteRow>, HubError> {
        self.store().list_invites(limit, offset).await
    }

    /// Journal de la console, du plus récent au plus ancien.
    pub async fn events(&self, limit: i64, offset: i64) -> Result<Page<AdminEvent>, HubError> {
        self.store().list_events(limit, offset).await
    }

    /// Page « Joueurs » : recherche (pseudo ou e-mail), filtre, pagination.
    pub async fn players(
        &self,
        search: &str,
        filter: PlayerFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Page<PlayerSummary>, HubError> {
        self.store()
            .list_players(search, filter, limit, offset)
            .await
    }

    /// Compteurs du tableau de bord.
    pub async fn dashboard(&self) -> Result<Dashboard, HubError> {
        let counts = self.store().dashboard_counts(now_unix()).await?;
        let online = i64::try_from(self.inner.presence.online_count()).unwrap_or(i64::MAX);
        Ok(Dashboard { counts, online })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_sans_adresse_complete() {
        assert_eq!(mask_email("alice@exemple.org"), "a…@exemple.org");
        let detail = clean_detail("compte bob@exemple.org\ncréé");
        assert!(!detail.contains("bob@"));
        assert!(!detail.contains('\n'));
        assert_eq!(clean_detail(&"x".repeat(500)).chars().count(), 200);
    }
}
