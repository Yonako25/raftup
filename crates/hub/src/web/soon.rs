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

    )
}
