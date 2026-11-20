//! Partie du stockage propre à la console d'admin (ADR 0036) : suspension des
//! comptes, réglages, codes d'invitation, journal de la console et liste des joueurs.
//!
//! Ce module est un enfant de `store` : il voit les champs privés de [`Store`]
//! (le pool) et ses fonctions privées. Aucune page HTML ici, seulement des données.

use std::collections::BTreeMap;

use serde::Serialize;
use sqlx::any::AnyRow;
use sqlx::{Any, Row, Transaction};

use super::{Store, DISCORD_PROVIDER, EMAIL_PROVIDER};
use crate::error::HubError;

/// Nombre maximal de lignes par page (liste des joueurs, journal, invitations).
pub const MAX_PAGE_SIZE: i64 = 200;

/// Une page d'une liste : les lignes, le total (pour « page 2 sur 5 ») et les
/// bornes réellement appliquées.
#[derive(Clone, Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

/// Bornes de pagination raisonnables : 1 à [`MAX_PAGE_SIZE`] lignes, décalage ≥ 0.
fn page_bounds(limit: i64, offset: i64) -> (i64, i64) {
    (limit.clamp(1, MAX_PAGE_SIZE), offset.max(0))
}

/// Ligne du journal de la console.
///
/// `actor_name` et `target_name` sont lus au moment de l'affichage (jointure sur
/// `users`) : un compte supprimé n'y laisse pas son pseudo.
#[derive(Clone, Debug, Serialize)]
pub struct AdminEvent {
    pub id: i64,
    pub created_at: i64,
    pub actor_id: Option<i64>,
    pub actor_name: Option<String>,
    pub kind: String,
    pub target_id: Option<i64>,
    pub target_name: Option<String>,
    pub detail: String,
}

/// Code d'invitation, tel que la console le montre (jamais le code lui-même : la
/// base n'en garde que l'empreinte).
#[derive(Clone, Debug, Serialize)]
pub struct InviteRow {
    pub id: i64,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_by: Option<String>,
    pub used_at: Option<i64>,
}

/// Filtre de la page « Joueurs ».
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerFilter {
    #[default]
    All,
    /// Comptes dont l'adresse e-mail n'est pas encore vérifiée.
    Unverified,
    Suspended,
}

impl PlayerFilter {
    /// Lit le filtre tel qu'il arrive d'un formulaire (`all`, `unverified`, `suspended`).
    pub fn parse(value: &str) -> Option<PlayerFilter> {
        match value {
            "all" | "" => Some(PlayerFilter::All),
            "unverified" => Some(PlayerFilter::Unverified),
            "suspended" => Some(PlayerFilter::Suspended),
            _ => None,
        }
    }

    /// Condition SQL ajoutée à la requête. Texte fixe, jamais tiré d'une saisie.
    fn condition(self) -> &'static str {
        match self {
            PlayerFilter::All => "1 = 1",
            PlayerFilter::Unverified => "e.verified = 0",
            PlayerFilter::Suspended => "u.suspended_at IS NOT NULL",
        }
    }
}

/// Un joueur dans la liste de la console.
#[derive(Clone, Debug, Serialize)]
pub struct PlayerSummary {
    pub id: i64,
    pub username: String,
    /// Adresse e-mail complète : la console la montre à l'admin (jamais dans le journal).
    pub email: Option<String>,
    pub email_verified: bool,
    pub discord_linked: bool,
    pub created_at: i64,
    pub suspended_at: Option<i64>,
}

/// Compteurs du tableau de bord tirés de la base (la présence, en mémoire, est
/// ajoutée par l'état du hub).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DashboardCounts {
    pub accounts: i64,
    pub unverified: i64,
    pub suspended: i64,
    /// Comptes créés pendant les 7 derniers jours.
    pub created_last_7_days: i64,
    /// Comptes qui ont au moins une session encore valide.
    pub with_active_session: i64,
}

/// Échappe `%`, `_` et `\` pour qu'une recherche soit lue mot pour mot par `LIKE`.
fn like_pattern(search: &str) -> String {
    let mut pattern = String::from("%");
    for c in search.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Consomme un code d'invitation à l'intérieur de la transaction d'inscription.
///
/// `&mut Transaction` : on emprunte la transaction de l'appelant sans la prendre ;
/// s'il y a une erreur, l'appelant ne valide pas et tout est annulé.
pub(super) async fn consume_invite(
    backend: super::Backend,
    tx: &mut Transaction<'static, Any>,
    code_hash: &str,
    user_id: i64,
    now: i64,
) -> Result<(), HubError> {
    // Une seule requête : deux inscriptions simultanées avec le même code ne
    // peuvent pas réussir toutes les deux (la seconde ne trouve plus `used_at IS NULL`).
    let result = super::query_for(
        backend,
        "UPDATE invites SET used_by = $1, used_at = $2 \
         WHERE code_hash = $3 AND used_at IS NULL AND expires_at > $4",
    )
    .bind(user_id)
    .bind(now)
    .bind(code_hash)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(HubError::InvalidInvite);
    }
    Ok(())
}

// ---------- Suspension ----------

impl Store {
    /// Suspend un compte et ferme toutes ses sessions, en une transaction.
    /// Suspendre deux fois garde la première date. `false` si le compte n'existe pas.
    pub async fn suspend_user(&self, user_id: i64, now: i64) -> Result<bool, HubError> {
        let mut tx = self.pool.begin().await?;
        let result = self
            .q("UPDATE users SET suspended_at = COALESCE(suspended_at, $1) WHERE id = $2")
            .bind(now)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        self.q("DELETE FROM sessions WHERE user_id = $1")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Lève la suspension (le joueur devra se reconnecter). `false` si le compte
    /// n'existe pas.
    pub async fn reactivate_user(&self, user_id: i64) -> Result<bool, HubError> {
        let result = self
            .q("UPDATE users SET suspended_at = NULL WHERE id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Le compte est-il suspendu ? (`false` s'il n'existe pas)
    pub async fn is_suspended(&self, user_id: i64) -> Result<bool, HubError> {
        let row = self
            .q("SELECT suspended_at FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => {
                let at: Option<i64> = row.try_get("suspended_at")?;
                Ok(at.is_some())
            }
            None => Ok(false),
        }
    }
}

// ---------- Réglages ----------

impl Store {
    /// Tous les réglages écrits en base, par nom (ordre stable).
    pub async fn all_settings(&self) -> Result<BTreeMap<String, String>, HubError> {
        let rows = self
            .q("SELECT name, value FROM settings ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        let mut map = BTreeMap::new();
        for row in rows {
            map.insert(row.try_get("name")?, row.try_get("value")?);
        }
        Ok(map)
    }

    /// Écrit (ou remplace) un réglage. La valeur doit déjà être vérifiée
    /// (voir [`crate::settings::SettingKey::validate`]).
    pub async fn put_setting(&self, name: &str, value: &str, now: i64) -> Result<(), HubError> {
        // Écrire ou remplacer : MySQL a sa propre forme (`ON DUPLICATE KEY UPDATE`).
        let upsert = if self.backend == super::Backend::Mysql {
            "INSERT INTO settings (name, value, updated_at) VALUES ($1, $2, $3) \
             ON DUPLICATE KEY UPDATE value = VALUES(value), updated_at = VALUES(updated_at)"
        } else {
            "INSERT INTO settings (name, value, updated_at) VALUES ($1, $2, $3) \
             ON CONFLICT (name) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at"
        };
        self.q(upsert)
            .bind(name)
            .bind(value)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Retire un réglage de la base (on revient à l'environnement ou au défaut).
    /// `false` s'il n'y était pas.
    pub async fn delete_setting(&self, name: &str) -> Result<bool, HubError> {
        let result = self
            .q("DELETE FROM settings WHERE name = $1")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

// ---------- Invitations ----------

impl Store {
    /// Enregistre un code d'invitation (seule son empreinte est gardée).
    pub async fn insert_invite(
        &self,
        code_hash: &str,
        created_by: Option<i64>,
        now: i64,
        expires_at: i64,
    ) -> Result<i64, HubError> {
        const INSERT: &str = "INSERT INTO invites (code_hash, created_by, created_at, expires_at) \
             VALUES ($1, $2, $3, $4)";
        // MySQL ne connaît pas `RETURNING` : l'identifiant vient du résultat.
        if self.backend == super::Backend::Mysql {
            let result = self
                .q(INSERT)
                .bind(code_hash)
                .bind(created_by)
                .bind(now)
                .bind(expires_at)
                .execute(&self.pool)
                .await?;
            return result
                .last_insert_id()
                .ok_or_else(|| HubError::Internal("identifiant de l'invitation non rendu".into()));
        }
        let row = self
            .q(concat!(
                "INSERT INTO invites (code_hash, created_by, created_at, expires_at) ",
                "VALUES ($1, $2, $3, $4) RETURNING id"
            ))
            .bind(code_hash)
            .bind(created_by)
            .bind(now)
            .bind(expires_at)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get("id")?)
    }

    /// Le code existe-t-il, sans être expiré ni déjà utilisé ? Vérification rapide
    /// avant le hachage du mot de passe ; la vraie consommation se fait à la création
    /// du compte.
    pub async fn invite_usable(&self, code_hash: &str, now: i64) -> Result<bool, HubError> {
        let row = self
            .q("SELECT 1 AS found FROM invites \
             WHERE code_hash = $1 AND used_at IS NULL AND expires_at > $2")
            .bind(code_hash)
            .bind(now)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.is_some())
    }

    /// Codes d'invitation, du plus récent au plus ancien.
    pub async fn list_invites(&self, limit: i64, offset: i64) -> Result<Page<InviteRow>, HubError> {
        let (limit, offset) = page_bounds(limit, offset);
        let total: i64 = self
            .q("SELECT COUNT(*) AS total FROM invites")
            .fetch_one(&self.pool)
            .await?
            .try_get("total")?;
        let rows = self
            .q(
                "SELECT i.id, c.username AS created_by, i.created_at, i.expires_at, \
             u.username AS used_by, i.used_at FROM invites i \
             LEFT JOIN users c ON c.id = i.created_by \
             LEFT JOIN users u ON u.id = i.used_by \
             ORDER BY i.created_at DESC, i.id DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let mut items = Vec::new();
        for row in rows {
            items.push(InviteRow {
                id: row.try_get("id")?,
                created_by: row.try_get("created_by")?,
                created_at: row.try_get("created_at")?,
                expires_at: row.try_get("expires_at")?,
                used_by: row.try_get("used_by")?,
                used_at: row.try_get("used_at")?,
            });
        }
        Ok(Page {
            items,
            total,
            limit,
            offset,
        })
    }

    /// Retire un code (révocation). `false` s'il n'existait pas.
    pub async fn delete_invite(&self, id: i64) -> Result<bool, HubError> {
        let result = self
            .q("DELETE FROM invites WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

// ---------- Journal de la console ----------

impl Store {
    /// Écrit une ligne du journal. Le détail doit déjà être nettoyé
    /// (voir [`crate::console::clean_detail`]).
    pub async fn insert_event(
        &self,
        now: i64,
        actor_id: Option<i64>,
        kind: &str,
        target_id: Option<i64>,
        detail: &str,
    ) -> Result<(), HubError> {
        self.q(
            "INSERT INTO admin_events (created_at, actor_id, kind, target_id, detail) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(now)
        .bind(actor_id)
        .bind(kind)
        .bind(target_id)
        .bind(detail)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Journal, du plus récent au plus ancien.
    pub async fn list_events(&self, limit: i64, offset: i64) -> Result<Page<AdminEvent>, HubError> {
        let (limit, offset) = page_bounds(limit, offset);
        let total: i64 = self
            .q("SELECT COUNT(*) AS total FROM admin_events")
            .fetch_one(&self.pool)
            .await?
            .try_get("total")?;
        let rows = self
            .q(
                "SELECT e.id, e.created_at, e.actor_id, a.username AS actor_name, e.kind, \
             e.target_id, t.username AS target_name, e.detail FROM admin_events e \
             LEFT JOIN users a ON a.id = e.actor_id \
             LEFT JOIN users t ON t.id = e.target_id \
             ORDER BY e.created_at DESC, e.id DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let items = rows.iter().map(event_row).collect::<Result<_, _>>()?;
        Ok(Page {
            items,
            total,
            limit,
            offset,
        })
    }
}

fn event_row(row: &AnyRow) -> Result<AdminEvent, HubError> {
    Ok(AdminEvent {
        id: row.try_get("id")?,
        created_at: row.try_get("created_at")?,
        actor_id: row.try_get("actor_id")?,
        actor_name: row.try_get("actor_name")?,
        kind: row.try_get("kind")?,
        target_id: row.try_get("target_id")?,
        target_name: row.try_get("target_name")?,
        detail: row.try_get("detail")?,
    })
}

/// Filtre de la page « Journaux ». Chaque champ vide (`None` ou texte vide)
/// laisse passer toutes les lignes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventFilter {
    /// Type exact (`account_created`…), déjà vérifié par l'appelant.
    pub kind: Option<String>,
    /// Morceau de pseudo, cherché chez l'acteur **ou** la cible.
    pub username: String,
    /// Première date gardée (incluse), en secondes Unix.
    pub from: Option<i64>,
    /// Date de fin (exclue), en secondes Unix.
    pub until: Option<i64>,
}

impl Store {
    /// Journal filtré, du plus récent au plus ancien (date puis identifiant, pour
    /// un ordre stable même quand plusieurs lignes ont la même seconde).
    pub async fn list_events_filtered(
        &self,
        filter: &EventFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Page<AdminEvent>, HubError> {
        let (limit, offset) = page_bounds(limit, offset);
        let username = crate::identity::normalize_username(filter.username.trim());
        let pattern = like_pattern(&username);

        // Comme pour `list_players` : le texte SQL n'est fait que de morceaux fixes,
        // les valeurs saisies passent toutes par les paramètres `$n`.
        let mut condition = String::from("WHERE 1 = 1");
        let mut next = 1;
        if filter.kind.is_some() {
            condition.push_str(&format!(" AND e.kind = ${next}"));
            next += 1;
        }
        if !username.is_empty() {
            // Deux paramètres (acteur, cible) : MySQL n'a que des `?`, un par usage.
            condition.push_str(&format!(
                " AND (a.username_norm LIKE ${} ESCAPE '\\' \
                 OR t.username_norm LIKE ${} ESCAPE '\\')",
                next,
                next + 1
            ));
            next += 2;
        }
        if filter.from.is_some() {
            condition.push_str(&format!(" AND e.created_at >= ${next}"));
            next += 1;
        }
        if filter.until.is_some() {
            condition.push_str(&format!(" AND e.created_at < ${next}"));
            next += 1;
        }
        let from = format!(
            "FROM admin_events e \
             LEFT JOIN users a ON a.id = e.actor_id \
             LEFT JOIN users t ON t.id = e.target_id {condition}"
        );

        // Les deux requêtes reçoivent les mêmes paramètres, dans le même ordre.
        // `sqlx::query` rend une valeur qu'on remplace à chaque `bind` : d'où les
        // `query = query.bind(…)`.
        let count_sql = format!("SELECT COUNT(*) AS total {from}");
        let list_sql = format!(
            "SELECT e.id, e.created_at, e.actor_id, a.username AS actor_name, e.kind, \
             e.target_id, t.username AS target_name, e.detail {from} \
             ORDER BY e.created_at DESC, e.id DESC LIMIT ${} OFFSET ${}",
            next,
            next + 1
        );
        let mut count = self.q(&count_sql);
        let mut list = self.q(&list_sql);
        if let Some(kind) = &filter.kind {
            count = count.bind(kind.as_str());
            list = list.bind(kind.as_str());
        }
        if !username.is_empty() {
            count = count.bind(pattern.as_str()).bind(pattern.as_str());
            list = list.bind(pattern.as_str()).bind(pattern.as_str());
        }
        if let Some(at) = filter.from {
            count = count.bind(at);
            list = list.bind(at);
        }
        if let Some(at) = filter.until {
            count = count.bind(at);
            list = list.bind(at);
        }
        let total: i64 = count.fetch_one(&self.pool).await?.try_get("total")?;
        let rows = list.bind(limit).bind(offset).fetch_all(&self.pool).await?;
        let items = rows.iter().map(event_row).collect::<Result<_, _>>()?;
        Ok(Page {
            items,
            total,
            limit,
            offset,
        })
    }
}

// ---------- Page « Joueurs » et tableau de bord ----------

impl Store {
    /// Liste des joueurs, du plus récent au plus ancien.
    ///
    /// `search` cherche dans le pseudo et l'adresse e-mail (sans tenir compte des
    /// majuscules) ; vide = tout le monde.
    pub async fn list_players(
        &self,
        search: &str,
        filter: PlayerFilter,
        limit: i64,
        offset: i64,
    ) -> Result<Page<PlayerSummary>, HubError> {
        let (limit, offset) = page_bounds(limit, offset);
        // Même forme normalisée que les pseudos en base (NFKC + minuscules) ; les
        // adresses sont gardées en minuscules.
        let search = crate::identity::normalize_username(search.trim());

        // La requête est assemblée à partir de morceaux **fixes** : la saisie ne
        // passe jamais dans le texte SQL, seulement dans les paramètres `$n`.
        // `$1` = email (fournisseur), puis la recherche éventuelle, puis les bornes.
        let mut condition = format!("WHERE {}", filter.condition());
        let mut next = 2;
        if !search.is_empty() {
            // Deux paramètres (pseudo, adresse) : MySQL n'a que des `?`, un par usage.
            condition.push_str(&format!(
                " AND (u.username_norm LIKE ${} ESCAPE '\\' \
                 OR e.subject LIKE ${} ESCAPE '\\')",
                next,
                next + 1
            ));
            next += 2;
        }
        let from = format!(
            "FROM users u LEFT JOIN user_identities e \
             ON e.user_id = u.id AND e.provider = $1 {condition}"
        );
        let pattern = like_pattern(&search);

        let count_sql = format!("SELECT COUNT(*) AS total {from}");
        let mut count = self.q(&count_sql).bind(EMAIL_PROVIDER);
        if !search.is_empty() {
            count = count.bind(pattern.as_str()).bind(pattern.as_str());
        }
        let total: i64 = count.fetch_one(&self.pool).await?.try_get("total")?;

        let list_sql = format!(
            "SELECT u.id, u.username, u.created_at, u.suspended_at, e.subject AS email, \
             e.verified AS email_verified, \
             (SELECT COUNT(*) FROM user_identities d \
              WHERE d.user_id = u.id AND d.provider = '{DISCORD_PROVIDER}') AS discord_count \
             {from} ORDER BY u.created_at DESC, u.id DESC LIMIT ${} OFFSET ${}",
            next,
            next + 1
        );
        let mut list = self.q(&list_sql).bind(EMAIL_PROVIDER);
        if !search.is_empty() {
            list = list.bind(pattern.as_str()).bind(pattern.as_str());
        }
        let rows = list.bind(limit).bind(offset).fetch_all(&self.pool).await?;

        let mut items = Vec::new();
        for row in rows {
            let verified: Option<i64> = row.try_get("email_verified")?;
            let discord: i64 = row.try_get("discord_count")?;
            items.push(PlayerSummary {
                id: row.try_get("id")?,
                username: row.try_get("username")?,
                email: row.try_get("email")?,
                email_verified: verified.unwrap_or(0) != 0,
                discord_linked: discord > 0,
                created_at: row.try_get("created_at")?,
                suspended_at: row.try_get("suspended_at")?,
            });
        }
        Ok(Page {
            items,
            total,
            limit,
            offset,
        })
    }

    /// Un joueur, pour sa fiche dans la console. `None` s'il n'existe pas.
    pub async fn player_summary(&self, user_id: i64) -> Result<Option<PlayerSummary>, HubError> {
        let row = self
            .q(
                "SELECT u.id, u.username, u.created_at, u.suspended_at, e.subject AS email, \
             e.verified AS email_verified FROM users u \
             LEFT JOIN user_identities e ON e.user_id = u.id AND e.provider = $1 \
             WHERE u.id = $2",
            )
            .bind(EMAIL_PROVIDER)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let verified: Option<i64> = row.try_get("email_verified")?;
        let providers = self.identity_providers(user_id).await?;
        Ok(Some(PlayerSummary {
            id: row.try_get("id")?,
            username: row.try_get("username")?,
            email: row.try_get("email")?,
            email_verified: verified.unwrap_or(0) != 0,
            discord_linked: providers.iter().any(|p| p == DISCORD_PROVIDER),
            created_at: row.try_get("created_at")?,
            suspended_at: row.try_get("suspended_at")?,
        }))
    }

    /// Compteurs du tableau de bord, à l'instant `now`.
    pub async fn dashboard_counts(&self, now: i64) -> Result<DashboardCounts, HubError> {
        let week_ago = now.saturating_sub(7 * 24 * 3600);
        let row = self
            .q("SELECT \
             (SELECT COUNT(*) FROM users) AS accounts, \
             (SELECT COUNT(*) FROM user_identities WHERE provider = $1 AND verified = 0) \
               AS unverified, \
             (SELECT COUNT(*) FROM users WHERE suspended_at IS NOT NULL) AS suspended, \
             (SELECT COUNT(*) FROM users WHERE created_at >= $2) AS recent, \
             (SELECT COUNT(DISTINCT user_id) FROM sessions WHERE expires_at > $3) AS active")
            .bind(EMAIL_PROVIDER)
            .bind(week_ago)
            .bind(now)
            .fetch_one(&self.pool)
            .await?;
        Ok(DashboardCounts {
            accounts: row.try_get("accounts")?,
            unverified: row.try_get("unverified")?,
            suspended: row.try_get("suspended")?,
            created_last_7_days: row.try_get("recent")?,
            with_active_session: row.try_get("active")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motif_like_echappe() {
        assert_eq!(like_pattern("ab"), "%ab%");
        assert_eq!(like_pattern("a%b_c\\"), "%a\\%b\\_c\\\\%");
    }

    #[test]
    fn bornes_de_page() {
        assert_eq!(page_bounds(0, -3), (1, 0));
        assert_eq!(page_bounds(10_000, 5), (MAX_PAGE_SIZE, 5));
    }

    #[test]
    fn filtres_lus() {
        assert_eq!(
            PlayerFilter::parse("suspended"),
            Some(PlayerFilter::Suspended)
        );
        assert_eq!(PlayerFilter::parse(""), Some(PlayerFilter::All));
        assert_eq!(PlayerFilter::parse("x"), None);
    }
}
