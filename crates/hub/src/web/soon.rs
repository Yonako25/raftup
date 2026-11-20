//! Pages du menu pas encore faites (Joueurs, Distribution du launcher, Réglages,
//! Journaux) : elles existent déjà pour que le menu mène quelque part, et disent
//! « Bientôt ». Chacune sera remplacée par son propre module (ADR 0036).

use askama::Template;
use axum::http::StatusCode;
use axum::response::Response;

use super::{page, AdminSession, Layout};

#[derive(Template)]
#[template(path = "admin/soon.html")]
struct SoonPage {
    layout: Layout,
    title: &'static str,
    summary: &'static str,
}

fn soon(
    session: &AdminSession,
    active: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Response {
    page(
        StatusCode::OK,
        &SoonPage {
            layout: Layout::new(session, active),
            title,
            summary,
        },
    )
}

/// `GET /admin/joueurs`.
pub async fn players(session: AdminSession) -> Response {
    soon(
        &session,
        "players",
        "Joueurs",
        "Liste des comptes, recherche, suspension et réactivation, export et suppression RGPD.",
    )
}

/// `GET /admin/distribution`.
pub async fn distribution(session: AdminSession) -> Response {
    soon(
        &session,
        "distribution",
        "Distribution du launcher",
        "Launcher déjà relié à ce hub, à télécharger ou à partager par un lien raftup://hub/….",
    )
}

/// `GET /admin/journaux`.
pub async fn logs(session: AdminSession) -> Response {
    soon(
        &session,
        "logs",
        "Journaux",
        "Fil des actions d'administration : comptes créés, suspensions, réglages modifiés.",
    )
}
