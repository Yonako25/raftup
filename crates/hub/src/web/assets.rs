//! Fichiers statiques de la console, sous `/admin/static/` (ADR 0036).
//!
//! Tout est copié dans le binaire à la compilation (`include_bytes!`) : la console ne
//! fait aucune requête vers un autre domaine (pas de Google Fonts). Les polices sont
//! celles que le launcher embarque déjà (`crates/launcher-ui/fonts/`, licence OFL),
//! servies avec le texte de leur licence.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// Chemin publié, type MIME et contenu. Liste fermée : aucun autre fichier n'est servi.
const FILES: &[(&str, &str, &[u8])] = &[
    (
        "rafthub.css",
        "text/css; charset=utf-8",
        include_bytes!("../../static/rafthub.css"),
    ),
    (
        "copy.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../../static/copy.js"),
    ),
    (
        "fonts/Inter.ttf",
        "font/ttf",
        include_bytes!("../../../launcher-ui/fonts/Inter.ttf"),
    ),
    (
        "fonts/Inter-SemiBold.ttf",
        "font/ttf",
        include_bytes!("../../../launcher-ui/fonts/Inter-SemiBold.ttf"),
    ),
    (
        "fonts/ChakraPetch-SemiBold.ttf",
        "font/ttf",
        include_bytes!("../../../launcher-ui/fonts/ChakraPetch-SemiBold.ttf"),
    ),
    (
        "fonts/ChakraPetch-Bold.ttf",
        "font/ttf",
        include_bytes!("../../../launcher-ui/fonts/ChakraPetch-Bold.ttf"),
    ),
    (
        "fonts/OFL-Inter.txt",
        "text/plain; charset=utf-8",
        include_bytes!("../../../launcher-ui/fonts/OFL-Inter.txt"),
    ),
    (
        "fonts/OFL-ChakraPetch.txt",
        "text/plain; charset=utf-8",
        include_bytes!("../../../launcher-ui/fonts/OFL-ChakraPetch.txt"),
    ),
];

/// `GET /admin/static/{*path}` : public (la page de connexion en a besoin).
pub async fn serve(Path(path): Path<String>) -> Response {
    let Some((_, mime, bytes)) = FILES.iter().find(|(name, _, _)| *name == path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = (*bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Les polices ne changent pas : un jour de cache. La feuille de style et le script
    // changent avec chaque version du hub : `no-cache` les fait relire à chaque page,
    // sinon un admin verrait l'ancienne mise en page après une mise à jour (ADR 0037).
    let cache = if mime.starts_with("font/") {
        "public, max-age=86400"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}
