//! Tableau de bord de la console (ADR 0036) : chiffres réels tirés de la base et
//! de la présence en mémoire, et état des services du hub.

use askama::Template;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;

use super::{internal_error, page, AdminSession, Layout};
use crate::AppState;

/// Une ligne du cadre « Services ».
pub struct ServiceRow {
    pub name: &'static str,
    pub detail: &'static str,
    /// Classe CSS de la pastille : `ok` (vert), `warn` (orange) ou `off` (gris).
    pub state: &'static str,
    /// Vrai si le service reste à régler (lien vers les réglages).
    pub needs_setup: bool,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardPage {
    layout: Layout,
    public_url: String,
    accounts: i64,
    online: usize,
    in_game: usize,
    unverified_emails: i64,
    services: Vec<ServiceRow>,
}

/// `GET /admin`.
pub async fn show(State(state): State<AppState>, session: AdminSession) -> Response {
    let counts = match state.store().dashboard_counts().await {
        Ok(counts) => counts,
        Err(error) => return internal_error(error),
    };
    let (online, in_game) = state.inner.presence.online_counts();

    let database = match state.store().ping().await {
        Ok(()) => ServiceRow {
            name: "Base de données",
            detail: match state.store().backend() {
                crate::store::Backend::Sqlite => "SQLite · répond",
                crate::store::Backend::Postgres => "PostgreSQL · répond",
            },
            state: "ok",
            needs_setup: false,
        },
        Err(error) => {
            tracing::warn!(%error, "la base ne répond pas");
            ServiceRow {
                name: "Base de données",
                detail: "Ne répond pas : voir le journal du hub",
                state: "warn",
                needs_setup: false,
            }
        }
    };
    let smtp = if state.inner.mailer.is_configured() {
        ServiceRow {
            name: "E-mails (SMTP)",
            detail: "Configuré",
            state: "ok",
            needs_setup: false,
        }
    } else {
        ServiceRow {
            name: "E-mails (SMTP)",
            detail: "Non configuré : pas de vérification d'adresse ni de mot de passe oublié",
            state: "warn",
            needs_setup: true,
        }
    };
    let discord = if state.inner.discord.is_some() {
        ServiceRow {
            name: "Connexion Discord",
            detail: "Configurée",
            state: "ok",
            needs_setup: false,
        }
    } else {
        ServiceRow {
            name: "Connexion Discord",
            detail: "Non configurée",
            state: "off",
            needs_setup: true,
        }
    };

    page(
        StatusCode::OK,
        &DashboardPage {
            layout: Layout::new(&session, "dashboard"),
            public_url: state.inner.config.public_url.clone(),
            accounts: counts.accounts,
            online,
            in_game,
            unverified_emails: counts.unverified_emails,
            services: vec![database, smtp, discord],
        },
    )
}
