//! Distribution du launcher (ADR 0036) : page d'admin `/admin/distribution`, page
//! publique `/telecharger` et zip `/telecharger/raftup-windows.zip`.
//!
//! Le travail (réseau, signature, zip) est dans [`crate::distribution`] ; ce module
//! ne fait que l'afficher. Les appels bloquants (disque, réseau) partent sur un
//! thread à part (`spawn_blocking`) pour ne pas figer les autres requêtes.

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Form;
use serde::Deserialize;

use super::{csrf_refused, error_page, internal_error, page, AdminSession, Layout};
use crate::distribution::{host_of, launcher_accepts, CacheStatus, DistributionError, ZIP_NAME};
use crate::settings::{SettingKey, SettingSource, Settings};
use crate::{now_unix, AppState, HubError};

/// Chemin du zip public.
pub const ZIP_PATH: &str = "/telecharger/raftup-windows.zip";
/// Chemin du zip pour les admins (le cookie de session ne vaut que sous `/admin`).
pub const ADMIN_ZIP_PATH: &str = "/admin/distribution/raftup-windows.zip";

/// Ce que la page d'admin montre de l'exe en cache.
pub struct CacheView {
    pub version: String,
    pub signature_ok: bool,
    pub fetched_at: String,
    pub size_mib: String,
}

#[derive(Template)]
#[template(path = "admin/distribution.html")]
struct DistributionPage {
    layout: Layout,
    cache: Option<CacheView>,
    /// Message après « Rafraîchir » : `(ok, texte)`.
    notice: Option<(bool, String)>,
    /// Pourquoi le zip ne peut pas être servi (adresse publique), ou `None`.
    url_problem: Option<&'static str>,
    hub_link: Option<String>,
    public_download: bool,
    zip_path: &'static str,
}

#[derive(Template)]
#[template(path = "public/download.html")]
struct PublicPage {
    hub_name: String,
    host: String,
    /// Vrai si un launcher vérifié est prêt à être servi.
    ready: bool,
    hub_link: Option<String>,
    zip_path: &'static str,
}

/// Formulaire du bouton « Rafraîchir ».
#[derive(Debug, Deserialize)]
pub struct RefreshForm {
    #[serde(default)]
    csrf: String,
}

/// Pourquoi l'adresse publique ne convient pas à un launcher relié, ou `None`.
fn url_problem(settings: &Settings) -> Option<&'static str> {
    if settings.source(SettingKey::PublicUrl) == SettingSource::Default {
        return Some(
            "L'adresse publique du hub n'est pas réglée : un launcher relié ne saurait pas où se connecter.",
        );
    }
    if !launcher_accepts(&settings.public_url) {
        return Some("Le launcher n'accepte qu'une adresse publique en HTTPS (ou vers ce PC).");
    }
    None
}

/// Lien `raftup://hub/<hôte>` : le launcher le transforme en `https://<hôte>`, donc
/// seulement pour une adresse en HTTPS.
fn hub_link(public_url: &str) -> Option<String> {
    if !public_url.starts_with("https://") {
        return None;
    }
    host_of(public_url).map(|host| format!("raftup://hub/{host}"))
}

/// Lance `work` sur un thread à part, avec un accès à l'état.
async fn blocking_with<T, F>(state: &AppState, work: F) -> Result<T, HubError>
where
    // `Send + 'static` : la fermeture part sur un autre thread ; elle reçoit son
    // propre clone de l'état au lieu de l'emprunter.
    F: FnOnce(&AppState) -> T + Send + 'static,
    T: Send + 'static,
{
    let state = state.clone();
    tokio::task::spawn_blocking(move || work(&state))
        .await
        .map_err(|e| HubError::Internal(format!("tâche de distribution : {e}")))
}

async fn render_admin(
    state: &AppState,
    session: &AdminSession,
    notice: Option<(bool, String)>,
) -> Response {
    let settings = match state.settings().await {
        Ok(settings) => settings,
        Err(error) => return internal_error(error),
    };
    let status = match blocking_with(state, |s| s.distribution().status()).await {
        Ok(status) => status,
        Err(error) => return internal_error(error),
    };
    let cache = match status {
        Ok(status) => status.map(cache_view),
        Err(error) => {
            tracing::warn!(%error, "cache du launcher illisible");
            None
        }
    };
    page(
        StatusCode::OK,
        &DistributionPage {
            layout: Layout::new(session, "distribution"),
            cache,
            notice,
            url_problem: url_problem(&settings),
            hub_link: hub_link(&settings.public_url),
            public_download: settings.public_download,
            zip_path: ADMIN_ZIP_PATH,
        },
    )
}

fn cache_view(status: CacheStatus) -> CacheView {
    CacheView {
        version: status.launcher.version,
        signature_ok: status.signature_ok,
        fetched_at: format_utc(status.launcher.fetched_at),
        size_mib: format!("{:.1}", status.launcher.size as f64 / (1024.0 * 1024.0)),
    }
}

/// `GET /admin/distribution`.
pub async fn show(State(state): State<AppState>, session: AdminSession) -> Response {
    render_admin(&state, &session, None).await
}

/// `POST /admin/distribution/rafraichir` : télécharge le dernier exe signé.
pub async fn refresh(
    State(state): State<AppState>,
    session: AdminSession,
    Form(form): Form<RefreshForm>,
) -> Response {
    if !session.csrf_ok(&form.csrf) {
        return csrf_refused();
    }
    let result = match blocking_with(&state, |s| s.distribution().refresh(now_unix())).await {
        Ok(result) => result,
        Err(error) => return internal_error(error),
    };
    let notice = match result {
        Ok(launcher) => (
            true,
            format!(
                "Launcher {} téléchargé, signature vérifiée.",
                launcher.version
            ),
        ),
        Err(error) => {
            tracing::warn!(%error, "rafraîchissement du launcher raté");
            (false, refresh_message(&error))
        }
    };
    render_admin(&state, &session, Some(notice)).await
}

/// Message affiché à l'admin après un échec (sans détail technique inutile).
fn refresh_message(error: &DistributionError) -> String {
    match error {
        DistributionError::BadSignature | DistributionError::HashMismatch => {
            "Le fichier reçu n'a pas la signature officielle : il a été refusé.".to_string()
        }
        DistributionError::Busy => "Un rafraîchissement est déjà en cours.".to_string(),
        DistributionError::Downgrade { offered, cached } => format!(
            "La release propose la version {offered}, plus ancienne que celle du cache ({cached}) : cache gardé."
        ),
        DistributionError::Network { .. } => {
            "GitHub n'a pas répondu : réessayez plus tard (détail dans le journal du hub).".to_string()
        }
        other => format!("Échec : {other}."),
    }
}

/// `GET /telecharger` : seule page publique du hub (désactivable).
pub async fn public_page(State(state): State<AppState>) -> Response {
    let settings = match state.settings().await {
        Ok(settings) => settings,
        Err(error) => return internal_error(error),
    };
    if !settings.public_download {
        return StatusCode::NOT_FOUND.into_response();
    }
    let ready = url_problem(&settings).is_none()
        && matches!(
            blocking_with(&state, |s| s.distribution().status()).await,
            Ok(Ok(Some(CacheStatus {
                signature_ok: true,
                ..
            })))
        );
    page(
        StatusCode::OK,
        &PublicPage {
            host: host_of(&settings.public_url).unwrap_or("").to_string(),
            hub_name: settings.hub_name.clone(),
            ready,
            hub_link: hub_link(&settings.public_url),
            zip_path: ZIP_PATH,
        },
    )
}

/// `GET /telecharger/raftup-windows.zip` : 404 si la page publique est désactivée.
pub async fn public_zip(State(state): State<AppState>) -> Response {
    let settings = match state.settings().await {
        Ok(settings) => settings,
        Err(error) => return internal_error(error),
    };
    if !settings.public_download {
        return StatusCode::NOT_FOUND.into_response();
    }
    serve_zip(&state, &settings).await
}

/// `GET /admin/distribution/raftup-windows.zip` : le même zip pour un admin connecté,
/// même si la page publique est désactivée. (Le cookie d'admin n'est envoyé que sous
/// `/admin` : d'où ce second chemin.)
pub async fn admin_zip(State(state): State<AppState>, _session: AdminSession) -> Response {
    let settings = match state.settings().await {
        Ok(settings) => settings,
        Err(error) => return internal_error(error),
    };
    serve_zip(&state, &settings).await
}

async fn serve_zip(state: &AppState, settings: &Settings) -> Response {
    if let Some(problem) = url_problem(settings) {
        tracing::warn!(
            problem,
            "zip du launcher demandé sans adresse publique utilisable"
        );
        return error_page(
            StatusCode::SERVICE_UNAVAILABLE,
            "Téléchargement indisponible",
            "Ce hub n'a pas encore d'adresse publique réglée. Revenez plus tard.",
        );
    }
    let hub_url = settings.public_url.clone();
    let zip = match blocking_with(state, move |s| s.distribution().zip_for(&hub_url)).await {
        Ok(Ok(zip)) => zip,
        Ok(Err(error)) => {
            tracing::warn!(%error, "zip du launcher non servi");
            return error_page(
                StatusCode::SERVICE_UNAVAILABLE,
                "Téléchargement indisponible",
                "Aucun launcher vérifié n'est prêt sur ce hub. Revenez plus tard.",
            );
        }
        Err(error) => return internal_error(error),
    };
    // `Arc<Vec<u8>>` → copie des octets dans le corps de la réponse.
    let mut response = zip.as_ref().clone().into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{ZIP_NAME}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// Date lisible en temps universel : `20/11/2026 14:05 UTC`.
fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{day:02}/{month:02}/{year} {:02}:{:02} UTC",
        rest / 3600,
        (rest % 3600) / 60
    )
}

/// Jour, mois, année d'un nombre de jours depuis le 1er janvier 1970 (algorithme de
/// Howard Hinnant, calendrier grégorien).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(format_utc(0), "01/01/1970 00:00 UTC");
        // 20 novembre 2026, 14 h 05.
        assert_eq!(format_utc(1_795_183_500), "20/11/2026 14:05 UTC");
        assert_eq!(format_utc(951_782_400), "29/02/2000 00:00 UTC");
    }

    #[test]
    fn lien_seulement_en_https() {
        assert_eq!(
            hub_link("https://hub.exemple.org").as_deref(),
            Some("raftup://hub/hub.exemple.org")
        );
        assert_eq!(hub_link("http://127.0.0.1:8787"), None);
    }
}
