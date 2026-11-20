//! Page « Journaux » de la console (ADR 0036) : le fil des actions
//! d'administration (table `admin_events`), du plus récent au plus ancien.
//!
//! La page ne fait que lire. Les filtres arrivent dans l'adresse (`GET`, sans
//! JavaScript) : `type`, `pseudo` (acteur ou cible), `du` et `au` (dates
//! `AAAA-MM-JJ`, en UTC, `au` compris), `page`.
//!
//! Les journaux techniques du serveur (`tracing`) ne sont pas ici : leur place dans
//! la console n'est pas encore décidée (ADR 0036, « Pas encore décidé »).

use askama::Template;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;

use super::{internal_error, page, AdminSession, Layout};
use crate::console::clean_detail;
use crate::store::{AdminEvent, EventFilter};
use crate::AppState;

/// Lignes par page.
const PAGE_SIZE: i64 = 50;
/// Secondes dans une journée.
const DAY: i64 = 24 * 3600;

/// Types du journal et leur nom en français, dans l'ordre de la liste déroulante.
const KINDS: [(&str, &str); 9] = [
    ("account_created", "Compte créé"),
    ("account_suspended", "Compte suspendu"),
    ("account_reactivated", "Compte réactivé"),
    ("setting_changed", "Réglage modifié"),
    ("invite_created", "Invitation créée"),
    ("invite_revoked", "Invitation révoquée"),
    ("gdpr_export", "Export RGPD"),
    ("gdpr_delete", "Suppression RGPD"),
    ("username_changed", "Pseudo changé"),
];

/// Nom en français d'un type ; un type inconnu (hub plus récent) s'affiche tel quel.
fn kind_label(kind: &str) -> String {
    KINDS
        .iter()
        .find(|(value, _)| *value == kind)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| kind.to_string())
}

/// Paramètres de l'adresse. Tous en texte : une saisie bizarre (`page=abc`) est
/// ignorée par la page au lieu d'être refusée par axum avec une erreur 400.
/// `#[serde(default)]` : un paramètre absent devient un texte vide.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct LogsQuery {
    #[serde(rename = "type")]
    kind: String,
    pseudo: String,
    du: String,
    au: String,
    page: String,
}

/// Une option de la liste déroulante des types.
pub struct KindOption {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

/// Une ligne du journal, prête à afficher.
pub struct EventView {
    pub date: String,
    pub actor: String,
    pub kind: String,
    pub target: String,
    pub detail: String,
}

#[derive(Template)]
#[template(path = "admin/logs.html")]
struct LogsPage {
    layout: Layout,
    kinds: Vec<KindOption>,
    pseudo: String,
    du: String,
    au: String,
    /// Message si une date n'a pas pu être lue (le filtre est alors ignoré).
    notice: Option<&'static str>,
    rows: Vec<EventView>,
    total: i64,
    page: i64,
    pages: i64,
    prev_href: Option<String>,
    next_href: Option<String>,
}

/// `GET /admin/journaux`.
pub async fn show(
    State(state): State<AppState>,
    session: AdminSession,
    Query(query): Query<LogsQuery>,
) -> Response {
    // Type : seulement une valeur connue, sinon pas de filtre.
    let kind = KINDS
        .iter()
        .find(|(value, _)| *value == query.kind)
        .map(|(value, _)| value.to_string());

    // Dates : `du` au début de sa journée, `au` à la fin de la sienne (incluse).
    let mut notice = None;
    let from = read_date(&query.du, &mut notice);
    let until = read_date(&query.au, &mut notice).map(|start| start + DAY);

    let page_number = query
        .page
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|n| *n >= 1 && *n <= 1_000_000)
        .unwrap_or(1);

    let filter = EventFilter {
        kind: kind.clone(),
        username: query.pseudo.clone(),
        from,
        until,
    };
    let offset = (page_number - 1) * PAGE_SIZE;
    let events = match state
        .store()
        .list_events_filtered(&filter, PAGE_SIZE, offset)
        .await
    {
        Ok(events) => events,
        Err(error) => return internal_error(error),
    };

    // Nombre de pages, au moins une (« page 1 sur 1 » même si la liste est vide).
    let pages = ((events.total + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
    let du = if from.is_some() { query.du.trim() } else { "" }.to_string();
    let au = if until.is_some() { query.au.trim() } else { "" }.to_string();
    let link = |n: i64| page_href(kind.as_deref(), query.pseudo.trim(), &du, &au, n);
    let prev_href = (page_number > 1).then(|| link(page_number - 1));
    let next_href = (page_number < pages).then(|| link(page_number + 1));

    let kinds = KINDS
        .iter()
        .map(|(value, label)| KindOption {
            value,
            label,
            selected: kind.as_deref() == Some(*value),
        })
        .collect();

    page(
        StatusCode::OK,
        &LogsPage {
            layout: Layout::new(&session, "logs"),
            kinds,
            pseudo: query.pseudo.trim().to_string(),
            du,
            au,
            notice,
            rows: events.items.iter().map(event_view).collect(),
            total: events.total,
            page: page_number,
            pages,
            prev_href,
            next_href,
        },
    )
}

/// Ligne du journal mise en forme pour la page.
fn event_view(event: &AdminEvent) -> EventView {
    // Pas d'acteur : action du système ou de la ligne de commande, ou compte effacé
    // depuis (suppression RGPD : l'identifiant a été retiré).
    let actor = match (&event.actor_name, event.actor_id) {
        (Some(name), _) => name.clone(),
        (None, Some(_)) => "Compte supprimé".to_string(),
        (None, None) => "Système".to_string(),
    };
    let target = match (&event.target_name, event.target_id) {
        (Some(name), _) => name.clone(),
        (None, Some(_)) => "Compte supprimé".to_string(),
        (None, None) => "—".to_string(),
    };
    EventView {
        date: format_datetime(event.created_at),
        actor,
        kind: kind_label(&event.kind),
        target,
        // Déjà nettoyé à l'écriture ; on repasse quand même le filtre pour qu'aucune
        // adresse e-mail complète ne s'affiche, même venue d'une vieille ligne.
        detail: clean_detail(&event.detail),
    }
}

/// Lien vers une autre page de résultats, avec les mêmes filtres.
fn page_href(kind: Option<&str>, pseudo: &str, du: &str, au: &str, page: i64) -> String {
    // `form_urlencoded` encode chaque valeur (espaces, `&`, accents…).
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(kind) = kind {
        query.append_pair("type", kind);
    }
    if !pseudo.is_empty() {
        query.append_pair("pseudo", pseudo);
    }
    if !du.is_empty() {
        query.append_pair("du", du);
    }
    if !au.is_empty() {
        query.append_pair("au", au);
    }
    query.append_pair("page", &page.to_string());
    format!("/admin/journaux?{}", query.finish())
}

/// Lit une date du formulaire ; vide → pas de filtre ; illisible → pas de filtre et
/// un message pour l'admin.
fn read_date(text: &str, notice: &mut Option<&'static str>) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let parsed = parse_date(text);
    if parsed.is_none() {
        *notice =
            Some("Une date n'a pas pu être lue (format attendu : AAAA-MM-JJ) : elle est ignorée.");
    }
    parsed
}

/// `AAAA-MM-JJ` → secondes Unix au début de ce jour, en UTC.
fn parse_date(text: &str) -> Option<i64> {
    let mut parts = text.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1970..=9999).contains(&year) {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    // Le 31 février n'existe pas : l'aller-retour ne retombe pas sur la même date.
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    Some(days * DAY)
}

/// Date et heure lisibles, en UTC : `20/11/2026 14:03 UTC`.
fn format_datetime(unix: i64) -> String {
    let days = unix.div_euclid(DAY);
    let seconds = unix.rem_euclid(DAY);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{day:02}/{month:02}/{year} {:02}:{:02} UTC",
        seconds / 3600,
        (seconds % 3600) / 60
    )
}

// Conversions entre jours depuis le 1er janvier 1970 et date du calendrier
// grégorien (algorithme de Howard Hinnant, « chrono-Compatible Low-Level Date
// Algorithms »). L'année commence en mars dans le calcul, pour que le jour en plus
// des années bissextiles tombe à la fin.

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_lues() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2026-11-20"), Some(1_795_132_800));
        assert_eq!(parse_date("2024-02-29"), Some(1_709_164_800));
        assert_eq!(parse_date("2025-02-29"), None);
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("20-11-2026"), None);
        assert_eq!(parse_date("2026-11-20-1"), None);
        assert_eq!(parse_date("demain"), None);
    }

    #[test]
    fn dates_affichees() {
        assert_eq!(format_datetime(0), "01/01/1970 00:00 UTC");
        assert_eq!(
            format_datetime(1_795_132_800 + 14 * 3600 + 3 * 60 + 59),
            "20/11/2026 14:03 UTC"
        );
        assert_eq!(format_datetime(1_709_164_800), "29/02/2024 00:00 UTC");
    }

    #[test]
    fn types_en_francais() {
        assert_eq!(kind_label("account_suspended"), "Compte suspendu");
        assert_eq!(kind_label("username_changed"), "Pseudo changé");
        assert_eq!(kind_label("nouveau_type"), "nouveau_type");
    }

    #[test]
    fn lien_de_page_garde_les_filtres() {
        assert_eq!(
            page_href(Some("gdpr_export"), "Bob & co", "2026-11-01", "", 3),
            "/admin/journaux?type=gdpr_export&pseudo=Bob+%26+co&du=2026-11-01&page=3"
        );
        assert_eq!(page_href(None, "", "", "", 2), "/admin/journaux?page=2");
    }
}
