//! Page « Réglages » de la console (ADR 0036).
//!
//! Un formulaire par groupe (général, inscriptions, e-mail, Discord, page publique),
//! tous envoyés en `POST /admin/reglages` avec le jeton anti-CSRF. Le champ `group`
//! dit quel groupe enregistrer ; le champ `reset` (bouton « Revenir au défaut »)
//! efface un réglage de la base ; `action=test_email` envoie un e-mail de test à
//! l'admin connecté.
//!
//! Règles d'enregistrement :
//! - un champ laissé vide ne change rien (pour effacer : « Revenir au défaut ») ;
//! - un champ dont la valeur ne change pas n'est pas réécrit (pas de ligne inutile
//!   au journal, et un réglage de l'environnement ne se fige pas en base) ;
//! - tout le groupe est vérifié avant d'écrire : une seule valeur invalide et rien
//!   n'est enregistré, la page affiche le message d'erreur ;
//! - les secrets (adresse SMTP, secret Discord) ne sont jamais remis dans la page :
//!   leur champ est toujours vide, la page dit seulement « configuré » ou non.

use std::collections::BTreeMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use super::{csrf_refused, internal_error, page, AdminSession, Layout};
use crate::error::HubError;
use crate::mail::Email;
use crate::settings::{PublicSettings, SettingKey, SettingSource};
use crate::AppState;

/// Un choix d'une liste déroulante.
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

/// Un réglage tel que la page l'affiche.
pub struct Field {
    /// Nom du réglage (`hub_name`…), aussi nom du champ du formulaire.
    pub name: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    /// Type du champ `<input>` : `text`, `url` ou `password` (secrets).
    pub input_type: &'static str,
    /// Valeur remise dans le champ : toujours vide pour un secret.
    pub value: String,
    pub placeholder: &'static str,
    /// Liste déroulante si non vide (sinon champ texte).
    pub choices: Vec<Choice>,
    /// `database`, `environment` ou `default` (classe CSS du badge).
    pub source: &'static str,
    pub source_label: &'static str,
    /// Vrai si la valeur vient de la base : le bouton « Revenir au défaut » sert.
    pub from_database: bool,
    pub secret: bool,
    /// Pour un secret : « Configuré · smtp.exemple.org », « Non configuré »…
    pub status: String,
}

/// Un cadre de la page, avec son formulaire.
pub struct Group {
    /// Valeur du champ caché `group`, et ancre de la page.
    pub id: &'static str,
    pub title: &'static str,
    pub intro: String,
    pub fields: Vec<Field>,
}

#[derive(Template)]
#[template(path = "admin/settings.html")]
struct SettingsPage {
    layout: Layout,
    groups: Vec<Group>,
    /// Message vert après un enregistrement réussi.
    notice: Option<&'static str>,
    /// Message rouge (valeur refusée, envoi raté…).
    error: Option<String>,
    smtp_configured: bool,
}

/// Les réglages de chaque groupe, dans l'ordre d'affichage.
const GROUPS: [(&str, &[SettingKey]); 5] = [
    ("general", &[SettingKey::HubName, SettingKey::PublicUrl]),
    ("registration", &[SettingKey::RegistrationMode]),
    ("email", &[SettingKey::SmtpUrl, SettingKey::MailFrom]),
    (
        "discord",
        &[SettingKey::DiscordClientId, SettingKey::DiscordClientSecret],
    ),
    ("download", &[SettingKey::PublicDownload]),
];

/// Les réglages d'un groupe, ou `None` si le groupe n'existe pas. Le groupe
/// « discord » n'existe que dans la version officielle (ADR 0038).
fn group_keys(id: &str, discord_allowed: bool) -> Option<&'static [SettingKey]> {
    GROUPS
        .iter()
        .filter(|(group, _)| discord_allowed || *group != "discord")
        .find(|(group, _)| *group == id)
        .map(|(_, keys)| *keys)
}

/// Libellé, aide et exemple de chaque réglage.
fn describe(key: SettingKey) -> (&'static str, &'static str, &'static str) {
    match key {
        SettingKey::HubName => (
            "Nom du hub",
            "Affiché aux joueurs et dans les e-mails.",
            "Rafthub",
        ),
        SettingKey::PublicUrl => (
            "Adresse publique",
            "Adresse par laquelle les joueurs joignent le hub (liens des e-mails, retours de connexion).",
            "https://hub.exemple.org",
        ),
        SettingKey::RegistrationMode => {
            ("Inscriptions", "Qui peut créer un compte sur ce hub.", "")
        }
        SettingKey::SmtpUrl => (
            "Adresse du serveur SMTP",
            "Contient le mot de passe : jamais réaffichée. Laisser vide pour ne rien changer.",
            "smtps://utilisateur:motdepasse@hote:465",
        ),
        SettingKey::MailFrom => (
            "Expéditeur",
            "Nom et adresse qui envoient les e-mails.",
            "Rafthub <no-reply@exemple.org>",
        ),
        SettingKey::DiscordClientId => (
            "Identifiant de l'application Discord",
            "« Client ID » du portail des développeurs Discord.",
            "123456789012345678",
        ),
        SettingKey::DiscordClientSecret => (
            "Secret de l'application Discord",
            "« Client secret » : jamais réaffiché. Laisser vide pour ne rien changer.",
            "",
        ),
        SettingKey::PublicDownload => (
            "Page publique de téléchargement",
            "Page /telecharger, où chacun récupère le launcher relié à ce hub.",
            "",
        ),
    }
}

fn source_names(source: SettingSource) -> (&'static str, &'static str) {
    match source {
        SettingSource::Database => ("database", "Base"),
        SettingSource::Environment => ("environment", "Environnement"),
        SettingSource::Default => ("default", "Défaut"),
    }
}

/// Choix possibles d'une liste déroulante (vide : champ texte).
fn choices(key: SettingKey, current: &str) -> Vec<Choice> {
    let list: &[(&'static str, &'static str)] = match key {
        SettingKey::RegistrationMode => &[
            ("open", "Ouvertes à tous"),
            ("invite", "Sur invitation"),
            ("closed", "Fermées"),
        ],
        SettingKey::PublicDownload => &[("true", "Activée"), ("false", "Désactivée")],
        _ => &[],
    };
    list.iter()
        .map(|(value, label)| Choice {
            value,
            label,
            selected: *value == current,
        })
        .collect()
}

/// Construit les cadres de la page depuis les réglages sans secret.
fn build_groups(settings: &PublicSettings, discord_allowed: bool) -> Vec<Group> {
    GROUPS
        .iter()
        .filter(|(id, _)| discord_allowed || *id != "discord")
        .map(|(id, keys)| {
            let fields = keys
                .iter()
                .filter_map(|key| {
                    // `entries` contient tous les réglages ; on prend celui-ci.
                    let entry = settings.entries.iter().find(|e| e.name == key.name())?;
                    let (label, help, placeholder) = describe(*key);
                    let (source, source_label) = source_names(entry.source);
                    let value = entry.value.clone().unwrap_or_default();
                    let status = match key {
                        SettingKey::SmtpUrl => match (&settings.smtp_host, entry.configured) {
                            (Some(host), true) => format!("Configurée · serveur {host}"),
                            (None, true) => "Configurée".to_string(),
                            _ => "Non configurée".to_string(),
                        },
                        SettingKey::DiscordClientSecret if entry.configured => {
                            "Configuré".to_string()
                        }
                        SettingKey::DiscordClientSecret => "Non configuré".to_string(),
                        _ => String::new(),
                    };
                    Some(Field {
                        name: entry.name,
                        label,
                        help,
                        input_type: if entry.secret {
                            "password"
                        } else if *key == SettingKey::PublicUrl {
                            "url"
                        } else {
                            "text"
                        },
                        choices: choices(*key, &value),
                        // Un secret n'est jamais remis dans la page (`value` est
                        // déjà `None` pour lui, on vide quand même par prudence).
                        value: if entry.secret { String::new() } else { value },
                        placeholder,
                        source,
                        source_label,
                        from_database: entry.source == SettingSource::Database,
                        secret: entry.secret,
                        status,
                    })
                })
                .collect();
            let (title, intro) = match *id {
                "general" => ("Général", String::new()),
                "registration" => (
                    "Inscriptions",
                    "Sur invitation, un code créé dans la console est demandé à l'inscription."
                        .to_string(),
                ),
                "email" => (
                    "E-mail (SMTP)",
                    if settings.smtp_configured {
                        "Les e-mails partent : vérification d'adresse et mot de passe oublié."
                            .to_string()
                    } else {
                        "Sans adresse SMTP et expéditeur, aucun e-mail ne part.".to_string()
                    },
                ),
                "discord" => (
                    "Connexion Discord",
                    format!(
                        "Adresse de retour à déclarer chez Discord : {}/v1/oauth/discord/callback",
                        settings.public_url
                    ),
                ),
                _ => ("Page publique", String::new()),
            };
            Group {
                id,
                title,
                intro,
                fields,
            }
        })
        .collect()
}

/// Rend la page avec les réglages actuels.
async fn render(
    state: &AppState,
    session: &AdminSession,
    status: StatusCode,
    notice: Option<&'static str>,
    error: Option<String>,
) -> Response {
    let settings = match state.public_settings().await {
        Ok(settings) => settings,
        Err(error) => return internal_error(error),
    };
    page(
        status,
        &SettingsPage {
            layout: Layout::new(session, "settings"),
            groups: build_groups(&settings, state.discord_allowed()),
            notice,
            error,
            smtp_configured: settings.smtp_configured,
        },
    )
}

/// Paramètre `?notice=…` posé après une redirection.
#[derive(Debug, Default, Deserialize)]
pub struct NoticeQuery {
    #[serde(default)]
    notice: String,
}

/// Message affiché pour un `?notice=…` connu (un texte fixe : rien de ce que tape
/// le visiteur dans l'adresse n'est recopié dans la page).
fn notice_text(code: &str) -> Option<&'static str> {
    match code {
        "saved" => Some("Réglages enregistrés."),
        "unchanged" => Some("Rien n'a changé."),
        "reset" => Some("Réglage remis à sa valeur par défaut."),
        "test_sent" => Some("E-mail de test envoyé à votre adresse."),
        _ => None,
    }
}

fn redirect(notice: &str) -> Response {
    Redirect::to(&format!("/admin/reglages?notice={notice}")).into_response()
}

/// `GET /admin/reglages`.
pub async fn show(
    State(state): State<AppState>,
    session: AdminSession,
    Query(query): Query<NoticeQuery>,
) -> Response {
    render(
        &state,
        &session,
        StatusCode::OK,
        notice_text(&query.notice),
        None,
    )
    .await
}

/// `POST /admin/reglages`.
///
/// Le corps est lu dans un `BTreeMap` (nom du champ → valeur) : la page a beaucoup
/// de champs, presque tous facultatifs. On ne l'affiche jamais en `Debug` : il peut
/// contenir un secret.
pub async fn submit(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<BTreeMap<String, String>>,
) -> Response {
    let field = |name: &str| form.get(name).map(String::as_str).unwrap_or("");
    if !session.csrf_ok(field("csrf")) {
        return csrf_refused();
    }

    let result = if !field("reset").is_empty() {
        reset(&state, &session, field("reset")).await
    } else if field("action") == "test_email" {
        send_test_email(&state, &session).await
    } else {
        save_group(&state, &session, &form).await
    };
    match result {
        Ok(notice) => redirect(notice),
        // Message pour l'admin : la page revient avec lui, rien n'est écrit.
        Err(Failure::Shown(message)) => {
            render(
                &state,
                &session,
                StatusCode::BAD_REQUEST,
                None,
                Some(message),
            )
            .await
        }
        Err(Failure::Hub(error)) => internal_error(error),
    }
}

/// Échec d'une action de la page.
enum Failure {
    /// À afficher dans la page (valeur refusée, envoi raté…).
    Shown(String),
    /// Erreur interne (base indisponible…) : page d'erreur, détail au journal.
    Hub(HubError),
}

/// `From` permet l'opérateur `?` sur une `HubError` : une valeur refusée devient
/// un message pour l'admin, le reste une erreur interne.
impl From<HubError> for Failure {
    fn from(error: HubError) -> Failure {
        match error {
            HubError::InvalidSetting(message) => {
                Failure::Shown(format!("Valeur refusée : {message}."))
            }
            HubError::UnknownSetting => Failure::Shown("Réglage inconnu.".to_string()),
            other => Failure::Hub(other),
        }
    }
}

/// Efface un réglage de la base : on revient à l'environnement, sinon au défaut.
async fn reset(
    state: &AppState,
    session: &AdminSession,
    name: &str,
) -> Result<&'static str, Failure> {
    state
        .update_setting(Some(session.user_id), name, None)
        .await?;
    Ok("reset")
}

/// Enregistre les réglages d'un groupe : tout est vérifié avant d'écrire.
async fn save_group(
    state: &AppState,
    session: &AdminSession,
    form: &BTreeMap<String, String>,
) -> Result<&'static str, Failure> {
    let group = form.get("group").map(String::as_str).unwrap_or("");
    let keys = group_keys(group, state.discord_allowed()).ok_or(HubError::UnknownSetting)?;
    let current = state.public_settings().await?;

    // 1. Vérification de tout le groupe ; on garde ce qui change vraiment.
    let mut changes: Vec<(SettingKey, String)> = Vec::new();
    for key in keys {
        let raw = form.get(key.name()).map(String::as_str).unwrap_or("");
        if raw.trim().is_empty() {
            continue; // champ vide : rien ne change
        }
        let value = key.validate(raw)?;
        let old = current
            .entries
            .iter()
            .find(|e| e.name == key.name())
            .and_then(|e| e.value.clone());
        // Un secret n'a pas de valeur connue (`None`) : il est toujours réécrit.
        if old.as_deref() != Some(value.as_str()) {
            changes.push((*key, value));
        }
    }
    if changes.is_empty() {
        return Ok("unchanged");
    }

    // 2. Écriture ; `update_setting` ajoute la ligne `setting_changed` au journal.
    for (key, value) in &changes {
        state
            .update_setting(Some(session.user_id), key.name(), Some(value))
            .await?;
    }
    Ok("saved")
}

/// Envoie un e-mail de test à l'adresse de l'admin connecté, et attend la réponse
/// du serveur SMTP pour dire si l'envoi a marché.
async fn send_test_email(
    state: &AppState,
    session: &AdminSession,
) -> Result<&'static str, Failure> {
    let mailer = state.mailer().await?;
    if !mailer.is_configured() {
        return Err(Failure::Shown(
            "E-mail de test impossible : réglez d'abord l'adresse SMTP et l'expéditeur."
                .to_string(),
        ));
    }
    let Some(to) = state.store().email_of(session.user_id).await? else {
        return Err(Failure::Shown(
            "E-mail de test impossible : votre compte n'a pas d'adresse e-mail.".to_string(),
        ));
    };
    let hub_name = state.settings().await?.hub_name;
    let email = Email {
        to,
        subject: format!("{hub_name} : e-mail de test"),
        body: format!(
            "Bonjour {},\n\nCe message vient de la console d'admin de {hub_name} : \
             l'envoi d'e-mails marche.\n",
            session.username
        ),
    };
    // L'envoi bloque le thread : il part sur un thread à part (`spawn_blocking`),
    // et `.await` attend son résultat sans bloquer les autres requêtes.
    let sent = tokio::task::spawn_blocking(move || mailer.send(&email))
        .await
        .map_err(|e| HubError::Internal(format!("envoi de l'e-mail de test : {e}")))?;
    match sent {
        Ok(()) => Ok("test_sent"),
        Err(error) => {
            tracing::warn!(%error, "e-mail de test non envoyé");
            Err(Failure::Shown(
                "E-mail de test non envoyé : le serveur SMTP a refusé ou ne répond pas \
                 (détail dans le journal du hub)."
                    .to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_reglage_dans_un_seul_groupe() {
        for key in SettingKey::ALL {
            let count = GROUPS
                .iter()
                .filter(|(_, keys)| keys.contains(&key))
                .count();
            assert_eq!(count, 1, "{}", key.name());
        }
        assert!(group_keys("general", false).is_some());
        assert!(group_keys("inconnu", true).is_none());
        // Discord : seulement dans la version officielle.
        assert!(group_keys("discord", true).is_some());
        assert!(group_keys("discord", false).is_none());
    }

    #[test]
    fn messages_fixes_seulement() {
        assert!(notice_text("saved").is_some());
        assert!(notice_text("<script>").is_none());
    }
}
