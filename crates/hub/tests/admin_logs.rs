//! Page « Journaux » de la console (ADR 0036) : requête filtrée du store, page
//! `/admin/journaux` (filtres en GET, pagination, accès réservé aux admins).
//! Tout passe par le routeur en `oneshot`, sans ouvrir de port.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use hub::store::{EventFilter, Role};
use hub::{AppState, HubConfig, Store};
use tower::ServiceExt;

use common::{register, test_store};

const PASSWORD: &str = "mot de passe solide";

/// Réponse lue en entier.
struct Page {
    status: StatusCode,
    headers: HeaderMap,
    body: String,
}

impl Page {
    fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    /// Nombre de lignes du journal affichées.
    fn event_rows(&self) -> usize {
        self.body.matches("class=\"row event\"").count()
    }
}

async fn send(app: &Router, request: Request<Body>) -> Page {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 4 << 20)
        .await
        .unwrap();
    Page {
        status,
        headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> Page {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(app, builder.body(Body::empty()).unwrap()).await
}

async fn hub() -> (Router, Store) {
    let store = test_store().await;
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    (hub::router(state), store)
}

/// Crée un compte (pseudo `name`, adresse `<name>@example.org`) ; renvoie son id.
async fn user(app: &Router, store: &Store, name: &str) -> i64 {
    let email = format!("{}@example.org", name.to_lowercase());
    let (status, _) = register(app, &email, name, PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
    store
        .user_by_username(&name.to_lowercase())
        .await
        .unwrap()
        .unwrap()
        .id
}

/// Crée un admin et ouvre sa session ; renvoie le cookie `rafthub_admin=…`.
async fn admin_cookie(app: &Router, store: &Store, name: &str) -> String {
    let email = format!("{}@example.org", name.to_lowercase());
    user(app, store, name).await;
    assert!(store.set_role_by_email(&email, Role::Admin).await.unwrap());

    let form = get(app, "/admin/login", None).await;
    let marker = "name=\"csrf\" value=\"";
    let start = form.body.find(marker).unwrap() + marker.len();
    let end = form.body[start..].find('"').unwrap();
    let csrf = form.body[start..start + end].to_string();
    let body = format!(
        "email={}&password={}&csrf={csrf}",
        email.replace('@', "%40"),
        PASSWORD.replace(' ', "+")
    );
    let request = Request::builder()
        .method(Method::POST)
        .uri("/admin/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, format!("rafthub_login_csrf={csrf}"))
        .body(Body::from(body))
        .unwrap();
    let page = send(app, request).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    let cookie = page
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .find(|c| c.starts_with("rafthub_admin="))
        .expect("cookie de session");
    cookie.split(';').next().unwrap().to_string()
}

/// Date de test : le jour `day` de novembre 2026, à 00:00 UTC.
fn nov(day: i64) -> i64 {
    1_793_404_800 + day * 86_400
}

// ---------- Store ----------

#[tokio::test]
async fn filtres_du_store() {
    let (app, store) = hub().await;
    let alice = user(&app, &store, "Alice").await;
    let bob = user(&app, &store, "Bob").await;
    let carole = user(&app, &store, "Carole").await;
    // Les inscriptions ont déjà écrit 3 lignes `account_created` (date du jour).
    store
        .insert_event(nov(1), Some(alice), "account_suspended", Some(bob), "")
        .await
        .unwrap();
    store
        .insert_event(nov(2), Some(alice), "account_reactivated", Some(bob), "")
        .await
        .unwrap();
    store
        .insert_event(nov(2), Some(carole), "setting_changed", None, "hub_name")
        .await
        .unwrap();
    store
        .insert_event(nov(5), None, "gdpr_delete", None, "")
        .await
        .unwrap();

    let all = store
        .list_events_filtered(&EventFilter::default(), 100, 0)
        .await
        .unwrap();
    assert_eq!(all.total, 7);
    // Plus récent d'abord ; à la même seconde (nov(2)), le dernier écrit d'abord.
    let kinds: Vec<&str> = all.items.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds[3..],
        [
            "gdpr_delete",
            "setting_changed",
            "account_reactivated",
            "account_suspended"
        ]
    );

    // Par type.
    let filter = EventFilter {
        kind: Some("account_created".into()),
        ..EventFilter::default()
    };
    let page = store.list_events_filtered(&filter, 100, 0).await.unwrap();
    assert_eq!(page.total, 3);
    assert!(page.items.iter().all(|e| e.kind == "account_created"));

    // Par pseudo (morceau) : toutes les lignes où Bob agit ou est visé, dont les
    // deux où il n'est que la cible.
    let filter = EventFilter {
        username: "bo".into(),
        ..EventFilter::default()
    };
    let page = store.list_events_filtered(&filter, 100, 0).await.unwrap();
    assert!(page.items.iter().all(|e| {
        e.actor_name.as_deref() == Some("Bob") || e.target_name.as_deref() == Some("Bob")
    }));
    assert_eq!(
        page.items
            .iter()
            .filter(|e| e.kind.starts_with("account_re") || e.kind == "account_suspended")
            .count(),
        2
    );
    // Sans tenir compte des majuscules ; acteur seulement aussi.
    let filter = EventFilter {
        username: "CAROLE".into(),
        kind: Some("setting_changed".into()),
        ..EventFilter::default()
    };
    let page = store.list_events_filtered(&filter, 100, 0).await.unwrap();
    assert_eq!(page.total, 1);
    // `%` est lu mot pour mot, pas comme un joker.
    let filter = EventFilter {
        username: "%".into(),
        ..EventFilter::default()
    };
    assert_eq!(
        store
            .list_events_filtered(&filter, 100, 0)
            .await
            .unwrap()
            .total,
        0
    );

    // Par période : du 2 (inclus) au 5 (exclu).
    let filter = EventFilter {
        from: Some(nov(2)),
        until: Some(nov(5)),
        ..EventFilter::default()
    };
    let page = store.list_events_filtered(&filter, 100, 0).await.unwrap();
    let kinds: Vec<&str> = page.items.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["setting_changed", "account_reactivated"]);

    // Pagination : total inchangé, lignes à la suite.
    let first = store
        .list_events_filtered(&EventFilter::default(), 2, 3)
        .await
        .unwrap();
    assert_eq!(first.total, 7);
    assert_eq!(first.items[0].kind, "gdpr_delete");
    assert_eq!(first.items[1].kind, "setting_changed");
}

// ---------- Page ----------

#[tokio::test]
async fn sans_session_pas_de_journaux() {
    let (app, _) = hub().await;
    for uri in ["/admin/journaux", "/admin/journaux?type=gdpr_export&page=2"] {
        let page = get(&app, uri, None).await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(page.location(), "/admin/login", "{uri}");
    }
    let page = get(&app, "/admin/journaux", Some("rafthub_admin=0123abcd")).await;
    assert_eq!(page.location(), "/admin/login");
}

#[tokio::test]
async fn page_des_journaux_et_filtres() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store, "Chef").await;
    let bob = user(&app, &store, "Bob").await;
    let chef = store.user_by_username("chef").await.unwrap().unwrap().id;
    // Une vieille ligne écrite sans nettoyage : la page doit quand même masquer l'adresse.
    store
        .insert_event(
            nov(3),
            Some(chef),
            "account_suspended",
            Some(bob),
            "signalé par bob.martin@example.org",
        )
        .await
        .unwrap();

    let page = get(&app, "/admin/journaux", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("aria-current=\"page\">Journaux"));
    assert_eq!(page.event_rows(), 3);
    assert!(page.body.contains("Compte suspendu"));
    assert!(page.body.contains("Compte créé"));
    assert!(page.body.contains("03/11/2026 00:00 UTC"));
    assert!(page.body.contains("Par Chef · cible : Bob"));
    assert!(page.body.contains("b…@example.org"));
    assert!(!page.body.contains("bob.martin@"));
    assert!(!page.body.contains("chef@example.org"));

    // Filtre par type : l'option est cochée et seule la suspension reste.
    let page = get(
        &app,
        "/admin/journaux?type=account_suspended",
        Some(&cookie),
    )
    .await;
    assert_eq!(page.event_rows(), 1);
    assert!(page
        .body
        .contains("<option value=\"account_suspended\" selected>"));
    // Un type inconnu est ignoré.
    let page = get(&app, "/admin/journaux?type=drop+table", Some(&cookie)).await;
    assert_eq!(page.event_rows(), 3);

    // Filtre par pseudo (cible) et par période.
    let page = get(&app, "/admin/journaux?pseudo=bob", Some(&cookie)).await;
    assert_eq!(page.event_rows(), 2);
    let page = get(
        &app,
        "/admin/journaux?du=2026-11-03&au=2026-11-03",
        Some(&cookie),
    )
    .await;
    assert_eq!(page.event_rows(), 1);
    assert!(page.body.contains("value=\"2026-11-03\""));
    let page = get(&app, "/admin/journaux?au=2026-11-02", Some(&cookie)).await;
    assert_eq!(page.event_rows(), 0);
    assert!(page.body.contains("Aucune ligne pour ces filtres."));

    // Date illisible : ignorée, avec un message.
    let page = get(&app, "/admin/journaux?du=31%2F02%2F2026", Some(&cookie)).await;
    assert_eq!(page.event_rows(), 3);
    assert!(page.body.contains("role=\"alert\""));

    // La page n'écrit rien.
    let before = store
        .list_events_filtered(&EventFilter::default(), 1, 0)
        .await
        .unwrap()
        .total;
    get(&app, "/admin/journaux", Some(&cookie)).await;
    let after = store
        .list_events_filtered(&EventFilter::default(), 1, 0)
        .await
        .unwrap()
        .total;
    assert_eq!(before, after);
}

#[tokio::test]
async fn pagination_de_la_page() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store, "Chef").await;
    // 1 ligne d'inscription + 60 réglages = 61 lignes : deux pages (50 + 11).
    for i in 0..60 {
        store
            .insert_event(nov(1) + i, None, "setting_changed", None, "hub_name")
            .await
            .unwrap();
    }
    let page = get(&app, "/admin/journaux?type=setting_changed", Some(&cookie)).await;
    assert_eq!(page.event_rows(), 50);
    assert!(page.body.contains("Page 1 sur 2"));
    assert!(page
        .body
        .contains("href=\"/admin/journaux?type=setting_changed&#38;page=2\""));
    assert!(!page.body.contains("rel=\"prev\""));

    let page = get(
        &app,
        "/admin/journaux?type=setting_changed&page=2",
        Some(&cookie),
    )
    .await;
    assert_eq!(page.event_rows(), 10);
    assert!(page.body.contains("Page 2 sur 2"));
    assert!(page.body.contains("rel=\"prev\""));
    assert!(!page.body.contains("rel=\"next\""));

    // Sans filtre : 61 lignes ; un numéro de page illisible revient à la page 1.
    let page = get(&app, "/admin/journaux?page=abc", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("61 lignes"));
    assert!(page.body.contains("Page 1 sur 2"));
}
