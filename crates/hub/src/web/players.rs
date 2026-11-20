//! Page « Joueurs » de la console (ADR 0036) : liste paginée des comptes, recherche
//! et filtres, suspension et réactivation, et codes d'invitation quand le hub est en
//! mode d'inscription `invite`.
//!
//! | Méthode et chemin | Rôle |
//! | --- | --- |
//! | `GET /admin/joueurs?q=…&filtre=…&page=…` | liste |
//! | `POST /admin/joueurs/suspendre` | suspend un compte (`csrf`, `user_id`) |
//! | `POST /admin/joueurs/reactiver` | lève la suspension (`csrf`, `user_id`) |
//! | `POST /admin/joueurs/renommer` | change le pseudo (`csrf`, `user_id`, `username`), sans limite |
//! | `POST /admin/joueurs/invitations` | crée un code, montré une seule fois (`csrf`) |
//! | `POST /admin/joueurs/invitations/revoquer` | retire un code (`csrf`, `invite_id`) |
//!
//! Chaque action passe par les opérations de `crate::console`, qui écrivent la ligne
//! du journal (`admin_events`). Après une action, on renvoie vers la liste (`303`)
//! avec la même recherche et un petit message (`msg=…`, texte fixe choisi ici).

use std::time::Duration;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Form;
use serde::Deserialize;

use super::{csrf_refused, internal_error, page, AdminSession, Layout};
use crate::console::{mask_email, CreatedInvite};
use crate::error::HubError;
use crate::memory::Status;
use crate::settings::RegistrationMode;
use crate::store::PlayerFilter;
use crate::AppState;

/// Joueurs par page.
const PAGE_SIZE: i64 = 25;
/// Codes d'invitation affichés (les plus récents).
const INVITES_SHOWN: i64 = 50;
/// Durée de validité d'un code d'invitation créé depuis la console (7 jours).
const INVITE_LIFETIME: Duration = Duration::from_secs(7 * 24 * 3600);

// ---------- Paramètres reçus ----------

/// Paramètres de l'adresse de la liste. Tout est en texte avec `#[serde(default)]` :
/// une valeur absente ou bizarre (`page=abc`) donne le défaut au lieu d'une erreur 400.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    filtre: String,
    #[serde(default)]
    page: String,
    #[serde(default)]
    msg: String,
}

/// Recherche, filtre et numéro de page, déjà lus et bornés.
struct ListState {
    search: String,
    filter: PlayerFilter,
    /// Numéro de page, à partir de 1.
    page: i64,
}

impl ListState {
    fn read(search: &str, filter: &str, page: &str) -> ListState {
        ListState {
            search: search.trim().chars().take(100).collect(),
            filter: PlayerFilter::parse(filter).unwrap_or_default(),
            page: page.trim().parse::<i64>().unwrap_or(1).clamp(1, 1_000_000),
        }
    }

    /// Adresse de la liste avec cet état, à la page `page`, et un message éventuel.
    fn url(&self, page: i64, msg: Option<&str>) -> String {
        // `form_urlencoded` encode les caractères spéciaux de la recherche
        // (`&`, `=`, espaces, accents…) : l'adresse reste correcte quoi qu'on tape.
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if !self.search.is_empty() {
            query.append_pair("q", &self.search);
        }
        if self.filter != PlayerFilter::All {
            query.append_pair("filtre", filter_name(self.filter));
        }
        if page > 1 {
            query.append_pair("page", &page.to_string());
        }
        if let Some(msg) = msg {
            query.append_pair("msg", msg);
        }
        let query = query.finish();
        if query.is_empty() {
            "/admin/joueurs".to_string()
        } else {
            format!("/admin/joueurs?{query}")
        }
    }
}

fn filter_name(filter: PlayerFilter) -> &'static str {
    match filter {
        PlayerFilter::All => "all",
        PlayerFilter::Unverified => "unverified",
        PlayerFilter::Suspended => "suspended",
    }
}

/// Formulaire des boutons « Suspendre » et « Réactiver ». Il renvoie aussi la
/// recherche en cours (champs cachés), pour revenir au même endroit de la liste.
#[derive(Debug, Deserialize)]
pub struct PlayerActionForm {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    filtre: String,
    #[serde(default)]
    page: String,
}

/// Formulaire « Renommer » : comme [`PlayerActionForm`], plus le nouveau pseudo.
#[derive(Debug, Deserialize)]
pub struct RenameForm {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    filtre: String,
    #[serde(default)]
    page: String,
}

/// Formulaire des codes d'invitation (création et révocation).
#[derive(Debug, Deserialize)]
pub struct InviteForm {
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    invite_id: String,
}

// ---------- Ce que le gabarit affiche ----------

/// Une ligne de la liste des joueurs.
pub struct PlayerRow {
    pub id: i64,
    pub username: String,
    /// Adresse masquée (`a…@exemple.org`), ou vide pour un compte Discord seul.
    pub email: String,
    pub discord: bool,
    pub created: String,
    pub verified: bool,
    pub has_email: bool,
    pub suspended: bool,
    /// `online`, `in_game` ou `offline`.
    pub presence: &'static str,
    /// Le compte de l'admin connecté : pas de bouton « Suspendre ».
    pub is_self: bool,
}

/// Une ligne de la liste des codes d'invitation.
pub struct InviteLine {
    pub id: i64,
    pub created: String,
    pub created_by: String,
    /// « Utilisé par Kael », « Expiré » ou « Valable jusqu'au … ».
    pub status: String,
    /// Code encore utilisable (affiché en vert).
    pub usable: bool,
}

/// Un onglet de filtre.
pub struct FilterTab {
    pub label: &'static str,
    pub value: &'static str,
    pub active: bool,
}

#[derive(Template)]
#[template(path = "admin/players.html")]
struct PlayersPage {
    layout: Layout,
    search: String,
    filter: &'static str,
    tabs: Vec<FilterTab>,
    rows: Vec<PlayerRow>,
    total: i64,
    page: i64,
    pages: i64,
    prev_url: Option<String>,
    next_url: Option<String>,
    /// Message après une action (texte fixe), et s'il s'agit d'une erreur.
    notice: Option<&'static str>,
    notice_is_error: bool,
    /// Bloc « Invitations » : seulement en mode `invite`.
    invite_mode: bool,
    invites: Vec<InviteLine>,
    /// Code tout juste créé, affiché cette fois-ci seulement.
    new_invite: Option<CreatedInvite>,
    new_invite_expires: String,
}

/// Messages possibles après une action. Le paramètre `msg` de l'adresse n'est
/// jamais affiché tel quel : seule une valeur connue choisit un de ces textes.
fn notice(msg: &str) -> Option<(&'static str, bool)> {
    match msg {
        "suspendu" => Some(("Compte suspendu : ses sessions sont fermées.", false)),
        "reactive" => Some(("Compte réactivé.", false)),
        "soi" => Some(("Vous ne pouvez pas suspendre votre propre compte.", true)),
        "inconnu" => Some(("Ce compte n'existe plus.", true)),
        "renomme" => Some(("Pseudo changé.", false)),
        "pseudo-pris" => Some(("Ce pseudo est déjà pris.", true)),
        "pseudo-invalide" => Some((
            "Pseudo refusé : 3 à 24 caractères, lettres, chiffres, _, - et ., pas de nom réservé.",
            true,
        )),
        "revoque" => Some(("Code d'invitation révoqué.", false)),
        "invitation-inconnue" => Some(("Ce code d'invitation n'existe plus.", true)),
        _ => None,
    }
}

// ---------- Dates ----------

/// Date `AAAA-MM-JJ` (UTC) d'un instant Unix, sans dépendance de plus.
///
/// Algorithme « days from civil » de Howard Hinnant, à l'envers : on compte en ères
/// de 400 ans (146 097 jours), où le calendrier grégorien se répète exactement.
fn date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097; // jour dans l'ère, 0 à 146 096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // année dans l'ère
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // jour dans l'année (depuis le 1er mars)
    let mp = (5 * doy + 2) / 153; // mois, mars = 0
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

// ---------- Pages ----------

/// `GET /admin/joueurs`.
pub async fn list(
    State(state): State<AppState>,
    session: AdminSession,
    Query(query): Query<ListQuery>,
) -> Response {
    let list = ListState::read(&query.q, &query.filtre, &query.page);
    match render(&state, &session, list, notice(&query.msg), None).await {
        Ok(response) => response,
        Err(error) => internal_error(error),
    }
}

/// Construit la page entière (liste, et invitations en mode `invite`).
async fn render(
    state: &AppState,
    session: &AdminSession,
    list: ListState,
    notice: Option<(&'static str, bool)>,
    new_invite: Option<CreatedInvite>,
) -> Result<Response, HubError> {
    let offset = (list.page - 1).saturating_mul(PAGE_SIZE);
    let found = state
        .players(&list.search, list.filter, PAGE_SIZE, offset)
        .await?;
    let pages = ((found.total + PAGE_SIZE - 1) / PAGE_SIZE).max(1);

    let rows = found
        .items
        .into_iter()
        .map(|player| {
            let presence = match state.inner.presence.get(player.id).status {
                Status::Online => "online",
                Status::InGame => "in_game",
                Status::Offline => "offline",
            };
            PlayerRow {
                id: player.id,
                username: player.username,
                email: player.email.as_deref().map(mask_email).unwrap_or_default(),
                discord: player.discord_linked,
                created: date(player.created_at),
                verified: player.email_verified,
                has_email: player.email.is_some(),
                suspended: player.suspended_at.is_some(),
                presence,
                is_self: player.id == session.user_id,
            }
        })
        .collect();

    let invite_mode =
        state.settings().await?.public().registration_mode == RegistrationMode::Invite;
    let invites = if invite_mode {
        invite_lines(state).await?
    } else {
        Vec::new()
    };

    let tabs = [
        ("Tous", PlayerFilter::All),
        ("E-mail non vérifié", PlayerFilter::Unverified),
        ("Suspendus", PlayerFilter::Suspended),
    ]
    .into_iter()
    .map(|(label, filter)| FilterTab {
        label,
        value: filter_name(filter),
        active: filter == list.filter,
    })
    .collect();

    let new_invite_expires = new_invite
        .as_ref()
        .map(|invite| date(invite.expires_at))
        .unwrap_or_default();
    Ok(page(
        StatusCode::OK,
        &PlayersPage {
            layout: Layout::new(session, "players"),
            search: list.search.clone(),
            filter: filter_name(list.filter),
            tabs,
            rows,
            total: found.total,
            page: list.page,
            pages,
            prev_url: (list.page > 1).then(|| list.url(list.page - 1, None)),
            next_url: (list.page < pages).then(|| list.url(list.page + 1, None)),
            notice: notice.map(|(text, _)| text),
            notice_is_error: notice.is_some_and(|(_, is_error)| is_error),
            invite_mode,
            invites,
            new_invite,
            new_invite_expires,
        },
    ))
}

async fn invite_lines(state: &AppState) -> Result<Vec<InviteLine>, HubError> {
    let now = crate::now_unix();
    let invites = state.invites(INVITES_SHOWN, 0).await?;
    Ok(invites
        .items
        .into_iter()
        .map(|invite| {
            let (status, usable) = match (&invite.used_by, invite.used_at) {
                (Some(name), _) => (format!("Utilisé par {name}"), false),
                // Compte utilisateur supprimé depuis (RGPD) : plus de pseudo.
                (None, Some(_)) => ("Utilisé".to_string(), false),
                (None, None) if invite.expires_at <= now => ("Expiré".to_string(), false),
                (None, None) => (
                    format!("Valable jusqu'au {}", date(invite.expires_at)),
                    true,
                ),
            };
            InviteLine {
                id: invite.id,
                created: date(invite.created_at),
                created_by: invite.created_by.unwrap_or_default(),
                status,
                usable,
            }
        })
        .collect())
}

// ---------- Actions ----------

/// `POST /admin/joueurs/suspendre`.
pub async fn suspend(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<PlayerActionForm>,
) -> Response {
    player_action(&state, &session, form, true).await
}

/// `POST /admin/joueurs/reactiver`.
pub async fn reactivate(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<PlayerActionForm>,
) -> Response {
    player_action(&state, &session, form, false).await
}

async fn player_action(
    state: &AppState,
    session: &AdminSession,
    form: PlayerActionForm,
    suspend: bool,
) -> Response {
    if !session.csrf_ok(&form.csrf) {
        return csrf_refused();
    }
    let list = ListState::read(&form.q, &form.filtre, &form.page);
    // Un identifiant illisible vaut « compte inconnu ».
    let Ok(user_id) = form.user_id.trim().parse::<i64>() else {
        return Redirect::to(&list.url(list.page, Some("inconnu"))).into_response();
    };
    let actor = Some(session.user_id);
    let result = if suspend {
        state.suspend_user(actor, user_id).await
    } else {
        state.reactivate_user(actor, user_id).await
    };
    let msg = match result {
        Ok(()) if suspend => "suspendu",
        Ok(()) => "reactive",
        Err(HubError::SelfSuspend) => "soi",
        Err(HubError::UserNotFound) => "inconnu",
        Err(error) => return internal_error(error),
    };
    Redirect::to(&list.url(list.page, Some(msg))).into_response()
}

/// `POST /admin/joueurs/renommer` : l'admin change le pseudo d'un compte. Mêmes
/// règles qu'à l'inscription, mais sans la limite de 24 heures du joueur.
pub async fn rename(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<RenameForm>,
) -> Response {
    if !session.csrf_ok(&form.csrf) {
        return csrf_refused();
    }
    let list = ListState::read(&form.q, &form.filtre, &form.page);
    let Ok(user_id) = form.user_id.trim().parse::<i64>() else {
        return Redirect::to(&list.url(list.page, Some("inconnu"))).into_response();
    };
    let result = state
        .rename_user(Some(session.user_id), user_id, form.username.trim(), false)
        .await;
    let msg = match result {
        Ok(_) => "renomme",
        Err(HubError::UsernameTaken) => "pseudo-pris",
        Err(HubError::InvalidUsername(_)) => "pseudo-invalide",
        Err(HubError::UserNotFound) => "inconnu",
        Err(error) => return internal_error(error),
    };
    Redirect::to(&list.url(list.page, Some(msg))).into_response()
}

/// `POST /admin/joueurs/invitations` : crée un code et l'affiche **une seule fois**.
///
/// On répond directement par la page (pas de redirection) : le code en clair ne
/// passe ni dans une adresse ni en base, il n'existe que dans cette réponse.
pub async fn create_invite(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<InviteForm>,
) -> Response {
    if !session.csrf_ok(&form.csrf) {
        return csrf_refused();
    }
    let created = match state
        .create_invite(Some(session.user_id), INVITE_LIFETIME)
        .await
    {
        Ok(created) => created,
        Err(error) => return internal_error(error),
    };
    let list = ListState::read("", "", "");
    match render(&state, &session, list, None, Some(created)).await {
        Ok(response) => response,
        Err(error) => internal_error(error),
    }
}

/// `POST /admin/joueurs/invitations/revoquer`.
pub async fn revoke_invite(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<InviteForm>,
) -> Response {
    if !session.csrf_ok(&form.csrf) {
        return csrf_refused();
    }
    let list = ListState::read("", "", "");
    let Ok(invite_id) = form.invite_id.trim().parse::<i64>() else {
        return Redirect::to(&list.url(1, Some("invitation-inconnue"))).into_response();
    };
    let msg = match state.revoke_invite(Some(session.user_id), invite_id).await {
        Ok(()) => "revoque",
        Err(HubError::InviteNotFound) => "invitation-inconnue",
        Err(error) => return internal_error(error),
    };
    Redirect::to(&list.url(1, Some(msg))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_utc() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_795_132_800), "2026-11-20");
        assert_eq!(date(-86_400), "1969-12-31");
    }

    #[test]
    fn adresse_de_la_liste() {
        let list = ListState::read(" a&b c ", "suspended", "2");
        assert_eq!(
            list.url(1, None),
            "/admin/joueurs?q=a%26b+c&filtre=suspended"
        );
        assert_eq!(
            list.url(3, Some("soi")),
            "/admin/joueurs?q=a%26b+c&filtre=suspended&page=3&msg=soi"
        );
        let list = ListState::read("", "nimporte", "abc");
        assert_eq!(list.filter, PlayerFilter::All);
        assert_eq!(list.page, 1);
        assert_eq!(list.url(1, None), "/admin/joueurs");
    }

    #[test]
    fn message_inconnu_ignore() {
        assert!(notice("<script>").is_none());
        assert_eq!(notice("soi").map(|(_, error)| error), Some(true));
    }
}
