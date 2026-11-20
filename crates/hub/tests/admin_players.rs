//! Page « Joueurs » de la console (ADR 0036) : liste, recherche, filtres,
//! pagination, suspension et réactivation, invitations, anti-CSRF.
//! Tout passe par le routeur en `oneshot`, sans ouvrir de port.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use hub::store::{PlayerFilter, Role};
use hub::{AppState, HubConfig, Store};
use serde_json::json;
use tower::ServiceExt;

use common::{account, register, set_presence, test_store};

const PASSWORD: &str = "mot de passe solide";

/// Réponse lue en entier : code, en-têtes, corps en texte.
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

    /// Valeur posée par `Set-Cookie` pour le cookie `name`.
    fn cookie_value(&self, name: &str) -> Option<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .find(|c| c.starts_with(&format!("{name}=")))
            .and_then(|line| {
                let first = line.split(';').next()?.to_string();
                Some(first.split_once('=')?.1.to_string())
            })
    }

    /// Valeur du premier champ caché `csrf` de la page.
    fn csrf_field(&self) -> String {
        let marker = "name=\"csrf\" value=\"";
        let start = self.body.find(marker).expect("champ csrf absent") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_string()
    }
}

/// Encode un formulaire en `application/x-www-form-urlencoded`.
fn form_body(fields: &[(&str, &str)]) -> String {
    fn encode(text: &str) -> String {
        text.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                    (b as char).to_string()
                }
                b' ' => "+".to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
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

async fn post_form(app: &Router, uri: &str, cookie: Option<&str>, fields: &[(&str, &str)]) -> Page {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(app, builder.body(Body::from(form_body(fields))).unwrap()).await
}

async fn hub() -> (Router, Store) {
    let store = test_store().await;
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    (hub::router(state), store)
}

/// Crée l'admin `Chef`, se connecte à la console et renvoie le cookie de session.
async fn admin_cookie(app: &Router, store: &Store) -> String {
    let (status, _) = register(app, "chef@example.org", "Chef", PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(store
        .set_role_by_email("chef@example.org", Role::Admin)
        .await
        .unwrap());
    let form = get(app, "/admin/login", None).await;
    let csrf = form.csrf_field();
    let page = post_form(
        app,
        "/admin/login",
        Some(&format!("rafthub_login_csrf={csrf}")),
        &[
            ("email", "chef@example.org"),
            ("password", PASSWORD),
            ("csrf", &csrf),
        ],
    )
    .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    format!(
        "rafthub_admin={}",
        page.cookie_value("rafthub_admin").unwrap()
    )
}

/// Identifiant du compte au pseudo `username`.
async fn user_id(store: &Store, username: &str) -> i64 {
    let found = store
        .list_players(username, PlayerFilter::All, 10, 0)
        .await
        .unwrap();
    found
        .items
        .iter()
        .find(|p| p.username == username)
        .expect("compte absent")
        .id
}

/// Types des lignes du journal, du plus récent au plus ancien.
async fn event_kinds(store: &Store) -> Vec<String> {
    let events = store.list_events(50, 0).await.unwrap();
    events.items.into_iter().map(|e| e.kind).collect()
}

#[tokio::test]
async fn sans_session_renvoie_vers_la_connexion() {
    let (app, _) = hub().await;
    let page = get(&app, "/admin/joueurs?q=x", None).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(page.location(), "/admin/login");
    for uri in [
        "/admin/joueurs/suspendre",
        "/admin/joueurs/reactiver",
        "/admin/joueurs/renommer",
        "/admin/joueurs/invitations",
        "/admin/joueurs/invitations/revoquer",
    ] {
        let page = post_form(&app, uri, None, &[("csrf", "x"), ("user_id", "1")]).await;
        assert_eq!(page.location(), "/admin/login", "{uri}");
    }
}

#[tokio::test]
async fn liste_recherche_et_filtres() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    let mireille = account(&app, "Mireille").await;
    account(&app, "Kael").await;
    assert_eq!(
        set_presence(
            &app,
            &mireille,
            json!({ "status": "in_game", "game": "APQuest" })
        )
        .await,
        StatusCode::NO_CONTENT
    );

    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("aria-current=\"page\">Joueurs"));
    assert!(page.body.contains("3 comptes"));
    assert!(page.body.contains("Mireille") && page.body.contains("Kael"));
    // Adresse masquée, jamais en entier.
    assert!(page.body.contains("m…@example.org"));
    assert!(!page.body.contains("mireille@example.org"));
    assert!(page.body.contains("En partie"));
    assert!(page.body.contains("Non vérifié"));
    // Pas de bouton « Suspendre » sur son propre compte.
    assert!(page.body.contains("(vous)"));
    assert_eq!(page.body.matches("/admin/joueurs/suspendre").count(), 2);
    // Pas de bloc « Invitations » en mode d'inscription ouvert.
    assert!(!page.body.contains("Créer un code"));

    // Recherche par pseudo ou par adresse, sans tenir compte des majuscules.
    let page = get(&app, "/admin/joueurs?q=mIrE", Some(&cookie)).await;
    assert!(page.body.contains("1 compte<"));
    assert!(page.body.contains("Mireille") && !page.body.contains("Kael"));
    let page = get(&app, "/admin/joueurs?q=kael%40example", Some(&cookie)).await;
    assert!(page.body.contains("Kael") && !page.body.contains("Mireille"));

    // Filtre « Suspendus » : personne pour l'instant.
    let page = get(&app, "/admin/joueurs?filtre=suspended", Some(&cookie)).await;
    assert!(page.body.contains("Aucun compte ne correspond."));
    // Paramètres bizarres : valeurs par défaut, pas d'erreur.
    let page = get(
        &app,
        "/admin/joueurs?filtre=nimporte&page=abc&msg=%3Cscript%3E",
        Some(&cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("3 comptes"));
    assert!(!page.body.contains("<script"));
}

#[tokio::test]
async fn pagination() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    for n in 0..26 {
        account(&app, &format!("Joueur{n:02}")).await;
    }
    // 27 comptes, 25 par page : deux pages.
    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert!(page.body.contains("Page 1 sur 2"));
    assert!(page.body.contains("href=\"/admin/joueurs?page=2\""));
    assert_eq!(page.body.matches("class=\"row player\"").count(), 25);
    let page = get(&app, "/admin/joueurs?page=2", Some(&cookie)).await;
    assert!(page.body.contains("Page 2 sur 2"));
    assert_eq!(page.body.matches("class=\"row player\"").count(), 2);
    // Les plus anciens en dernier : Chef est sur la page 2.
    assert!(page.body.contains("Chef"));
}

#[tokio::test]
async fn suspendre_puis_reactiver() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    account(&app, "Mireille").await;
    let id = user_id(&store, "Mireille").await.to_string();
    let csrf = get(&app, "/admin/joueurs", Some(&cookie))
        .await
        .csrf_field();

    let page = post_form(
        &app,
        "/admin/joueurs/suspendre",
        Some(&cookie),
        &[
            ("csrf", &csrf),
            ("user_id", &id),
            ("q", "mire"),
            ("filtre", "all"),
            ("page", "1"),
        ],
    )
    .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    // Retour à la même recherche, avec un message.
    assert_eq!(page.location(), "/admin/joueurs?q=mire&msg=suspendu");
    let id_num: i64 = id.parse().unwrap();
    assert!(store.is_suspended(id_num).await.unwrap());
    // Le joueur ne peut plus se connecter.
    let (status, _) = common::login(&app, "mireille@example.org", PASSWORD).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let page = get(&app, page.location(), Some(&cookie)).await;
    assert!(page.body.contains("Compte suspendu"));
    assert!(page.body.contains("Suspendu</span>"));
    assert!(page.body.contains("/admin/joueurs/reactiver"));
    let page = get(&app, "/admin/joueurs?filtre=suspended", Some(&cookie)).await;
    assert!(page.body.contains("Mireille") && !page.body.contains("(vous)"));

    let page = post_form(
        &app,
        "/admin/joueurs/reactiver",
        Some(&cookie),
        &[("csrf", &csrf), ("user_id", &id)],
    )
    .await;
    assert_eq!(page.location(), "/admin/joueurs?msg=reactive");
    assert!(!store.is_suspended(id_num).await.unwrap());
    let (status, _) = common::login(&app, "mireille@example.org", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);

    // Chaque action a sa ligne dans le journal, avec l'admin comme acteur.
    let kinds = event_kinds(&store).await;
    assert_eq!(kinds[0], "account_reactivated");
    assert_eq!(kinds[1], "account_suspended");
    let events = store.list_events(2, 0).await.unwrap();
    assert!(events
        .items
        .iter()
        .all(|e| e.actor_name.as_deref() == Some("Chef")
            && e.target_name.as_deref() == Some("Mireille")));

    // Se suspendre soi-même : refusé, avec un message.
    let chef = user_id(&store, "Chef").await.to_string();
    let page = post_form(
        &app,
        "/admin/joueurs/suspendre",
        Some(&cookie),
        &[("csrf", &csrf), ("user_id", &chef)],
    )
    .await;
    assert_eq!(page.location(), "/admin/joueurs?msg=soi");
    // Compte inconnu ou identifiant illisible.
    for bad in ["999999", "abc"] {
        let page = post_form(
            &app,
            "/admin/joueurs/suspendre",
            Some(&cookie),
            &[("csrf", &csrf), ("user_id", bad)],
        )
        .await;
        assert_eq!(page.location(), "/admin/joueurs?msg=inconnu", "{bad}");
    }
}

#[tokio::test]
async fn csrf_refuse() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    account(&app, "Mireille").await;
    let id = user_id(&store, "Mireille").await;
    let id_text = id.to_string();

    for fields in [
        vec![("csrf", "faux"), ("user_id", id_text.as_str())],
        vec![("user_id", id_text.as_str())],
    ] {
        let page = post_form(&app, "/admin/joueurs/suspendre", Some(&cookie), &fields).await;
        assert_eq!(page.status, StatusCode::FORBIDDEN);
    }
    assert!(!store.is_suspended(id).await.unwrap());
    let page = post_form(
        &app,
        "/admin/joueurs/invitations",
        Some(&cookie),
        &[("csrf", "faux")],
    )
    .await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    // Rien n'a été écrit dans le journal.
    assert!(event_kinds(&store)
        .await
        .iter()
        .all(|k| k == "account_created"));
}

#[tokio::test]
async fn invitations_en_mode_invite() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    store
        .put_setting("registration_mode", "invite", hub::now_unix())
        .await
        .unwrap();

    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert!(page.body.contains("Créer un code"));
    let csrf = page.csrf_field();

    // Création : le code est montré dans la réponse…
    let page = post_form(
        &app,
        "/admin/joueurs/invitations",
        Some(&cookie),
        &[("csrf", &csrf)],
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    let marker = "id=\"invite-code\">";
    let start = page.body.find(marker).expect("code absent") + marker.len();
    let end = page.body[start..].find('<').unwrap();
    let code = page.body[start..start + end].to_string();
    assert!(code.len() >= 16, "{code}");
    // … et plus jamais ensuite.
    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert!(!page.body.contains(&code));
    // (askama échappe l'apostrophe : on ne cherche que le début.)
    assert!(page.body.contains("Valable jusqu"));
    assert!(page.body.contains("par Chef"));

    // Le code permet de s'inscrire.
    let (status, _) = common::call(
        &app,
        Method::POST,
        "/v1/accounts",
        None,
        Some(json!({
            "email": "nouveau@example.org",
            "username": "Nouveau",
            "password": PASSWORD,
            "invite_code": code,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert!(page.body.contains("Utilisé par Nouveau"));

    // Révocation.
    let invite_id = store.list_invites(1, 0).await.unwrap().items[0]
        .id
        .to_string();
    let page = post_form(
        &app,
        "/admin/joueurs/invitations/revoquer",
        Some(&cookie),
        &[("csrf", &csrf), ("invite_id", &invite_id)],
    )
    .await;
    assert_eq!(page.location(), "/admin/joueurs?msg=revoque");
    assert_eq!(store.list_invites(10, 0).await.unwrap().total, 0);
    let page = post_form(
        &app,
        "/admin/joueurs/invitations/revoquer",
        Some(&cookie),
        &[("csrf", &csrf), ("invite_id", &invite_id)],
    )
    .await;
    assert_eq!(page.location(), "/admin/joueurs?msg=invitation-inconnue");

    let kinds = event_kinds(&store).await;
    assert!(kinds.contains(&"invite_created".to_string()));
    assert!(kinds.contains(&"invite_revoked".to_string()));
}

/// Envoie le formulaire « Renommer » de la console (recherche `ka` en cours).
async fn rename(app: &Router, cookie: &str, csrf: &str, id: &str, username: &str) -> Page {
    post_form(
        app,
        "/admin/joueurs/renommer",
        Some(cookie),
        &[
            ("csrf", csrf),
            ("user_id", id),
            ("username", username),
            ("q", "ka"),
        ],
    )
    .await
}

#[tokio::test]
async fn renommer_un_compte() {
    let (app, store) = hub().await;
    let cookie = admin_cookie(&app, &store).await;
    let kael = account(&app, "Kael").await;
    account(&app, "Mireille").await;
    let id = user_id(&store, "Kael").await.to_string();
    let page = get(&app, "/admin/joueurs", Some(&cookie)).await;
    assert!(page.body.contains("/admin/joueurs/renommer"));
    let csrf = page.csrf_field();

    // Sans le bon jeton anti-CSRF : refusé, rien ne change.
    let page = post_form(
        &app,
        "/admin/joueurs/renommer",
        Some(&cookie),
        &[("csrf", "faux"), ("user_id", &id), ("username", "Pirate")],
    )
    .await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);

    let page = rename(&app, &cookie, &csrf, &id, "Mireille").await;
    assert_eq!(page.location(), "/admin/joueurs?q=ka&msg=pseudo-pris");
    let page = rename(&app, &cookie, &csrf, &id, "a b").await;
    assert_eq!(page.location(), "/admin/joueurs?q=ka&msg=pseudo-invalide");
    let page = rename(&app, &cookie, &csrf, &id, " Kaelis ").await;
    assert_eq!(page.location(), "/admin/joueurs?q=ka&msg=renomme");
    let page = get(&app, page.location(), Some(&cookie)).await;
    assert!(page.body.contains("Pseudo changé."));
    assert!(page.body.contains("Kaelis"));
    // L'admin n'a pas de limite de 24 heures.
    assert_eq!(
        rename(&app, &cookie, &csrf, &id, "Kaelan").await.location(),
        "/admin/joueurs?q=ka&msg=renomme"
    );

    // Journal : l'admin est l'acteur, ancien → nouveau.
    let events = store.list_events(1, 0).await.unwrap();
    let event = &events.items[0];
    assert_eq!(event.kind, "username_changed");
    assert_eq!(event.actor_name.as_deref(), Some("Chef"));
    assert_eq!(event.detail, "Kaelis → Kaelan");
    let page = get(&app, "/admin/journaux", Some(&cookie)).await;
    assert!(page.body.contains("Pseudo changé"));

    // Les changements de l'admin ne comptent pas dans la limite du joueur.
    let (status, _) = common::call(
        &app,
        Method::PATCH,
        "/v1/me",
        Some(&kael),
        Some(json!({ "username": "KaelLeVrai" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
