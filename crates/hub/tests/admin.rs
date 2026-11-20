//! Console d'admin (ADR 0036) : connexion par cookie, anti-CSRF, limite des essais,
//! tableau de bord. Tout passe par le routeur en `oneshot`, sans ouvrir de port.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use hub::store::Role;
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

    /// Tous les en-têtes `Set-Cookie`.
    fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect()
    }

    /// En-tête `Set-Cookie` du cookie `name`, s'il y en a un.
    fn set_cookie(&self, name: &str) -> Option<String> {
        self.set_cookies()
            .into_iter()
            .find(|c| c.starts_with(&format!("{name}=")))
    }

    /// Valeur posée pour le cookie `name`.
    fn cookie_value(&self, name: &str) -> Option<String> {
        let line = self.set_cookie(name)?;
        let first = line.split(';').next()?;
        Some(first.split_once('=')?.1.to_string())
    }

    /// Valeur du champ caché `csrf` du formulaire de la page.
    fn csrf_field(&self) -> String {
        let marker = "name=\"csrf\" value=\"";
        let start = self.body.find(marker).expect("champ csrf absent") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_string()
    }

    /// Chiffre affiché dans la carte `id` du tableau de bord.
    fn card_value(&self, id: &str) -> String {
        let start = self
            .body
            .find(&format!("id=\"{id}\""))
            .expect("carte absente");
        let marker = "card-value\">";
        let from = start + self.body[start..].find(marker).unwrap() + marker.len();
        let end = self.body[from..].find('<').unwrap();
        self.body[from..from + end].to_string()
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

/// Hub de test, avec accès direct à la base pour donner le rôle d'admin.
async fn hub_with(config: HubConfig) -> (Router, Store) {
    let store = test_store().await;
    let state = AppState::new(store.clone(), config).await.unwrap();
    (hub::router(state), store)
}

async fn hub() -> (Router, Store) {
    hub_with(HubConfig::for_tests()).await
}

/// Crée un compte et lui donne le rôle d'admin.
async fn make_admin(app: &Router, store: &Store, email: &str, username: &str) {
    let (status, _) = register(app, email, username, PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(store.set_role_by_email(email, Role::Admin).await.unwrap());
}

/// Ouvre le formulaire puis l'envoie ; renvoie la réponse du `POST`.
async fn try_login(app: &Router, email: &str, password: &str) -> Page {
    let form = get(app, "/admin/login", None).await;
    assert_eq!(form.status, StatusCode::OK);
    let csrf = form.csrf_field();
    assert_eq!(
        form.cookie_value("rafthub_login_csrf").as_deref(),
        Some(csrf.as_str())
    );
    let cookie = format!("rafthub_login_csrf={csrf}");
    post_form(
        app,
        "/admin/login",
        Some(&cookie),
        &[("email", email), ("password", password), ("csrf", &csrf)],
    )
    .await
}

/// Connexion réussie : renvoie le cookie à envoyer ensuite (`rafthub_admin=…`).
async fn login_cookie(app: &Router, email: &str) -> String {
    let page = try_login(app, email, PASSWORD).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin");
    let token = page
        .cookie_value("rafthub_admin")
        .expect("cookie de session");
    format!("rafthub_admin={token}")
}

#[tokio::test]
async fn sans_session_renvoie_vers_la_connexion() {
    let (app, _) = hub().await;
    for uri in [
        "/admin",
        "/admin/joueurs",
        "/admin/distribution",
        "/admin/reglages",
        "/admin/journaux",
    ] {
        let page = get(&app, uri, None).await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(page.location(), "/admin/login", "{uri}");
    }
    // Un cookie inventé ne vaut pas mieux.
    let page = get(&app, "/admin", Some("rafthub_admin=0123abcd")).await;
    assert_eq!(page.location(), "/admin/login");
    // La déconnexion aussi demande une session.
    let page = post_form(&app, "/admin/logout", None, &[("csrf", "x")]).await;
    assert_eq!(page.location(), "/admin/login");
}

#[tokio::test]
async fn un_jeton_de_l_api_ne_donne_pas_acces() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    let (status, body) = common::login(&app, "chef@example.org", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
    let api_token = body["token"].as_str().unwrap().to_string();

    // Ni en en-tête Bearer…
    let request = Request::builder()
        .uri("/admin")
        .header(header::AUTHORIZATION, format!("Bearer {api_token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, request).await.location(), "/admin/login");
    // … ni recopié dans le cookie de la console.
    let page = get(&app, "/admin", Some(&format!("rafthub_admin={api_token}"))).await;
    assert_eq!(page.location(), "/admin/login");
}

#[tokio::test]
async fn connexion_cookie_et_tableau_de_bord() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    let joueur = account(&app, "Mireille").await;
    account(&app, "Kael").await;
    assert_eq!(
        set_presence(
            &app,
            &joueur,
            json!({ "status": "in_game", "game": "APQuest" })
        )
        .await,
        StatusCode::NO_CONTENT
    );

    let page = try_login(&app, "chef@example.org", PASSWORD).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    let cookie = page.set_cookie("rafthub_admin").unwrap();
    assert!(cookie.contains("HttpOnly"), "{cookie}");
    assert!(cookie.contains("SameSite=Strict"), "{cookie}");
    assert!(cookie.contains("Path=/admin"), "{cookie}");
    assert!(cookie.contains("Max-Age=43200"), "12 h : {cookie}");
    // Hub joint en http://127.0.0.1 : pas de `Secure`, sinon le navigateur le refuse.
    assert!(!cookie.contains("Secure"), "{cookie}");
    // Le jeton du formulaire de connexion est effacé.
    let used = page.set_cookie("rafthub_login_csrf").unwrap();
    assert!(used.contains("Max-Age=0"), "{used}");

    let cookie = login_cookie(&app, "chef@example.org").await;
    let page = get(&app, "/admin", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Tableau de bord"));
    assert!(page.body.contains("Connecté : Chef"));
    assert_eq!(page.card_value("stat-accounts"), "3");
    assert_eq!(page.card_value("stat-online"), "1");
    assert!(page.body.contains("dont 1 en partie"));
    // Les trois adresses viennent d'être inscrites, aucune n'est vérifiée.
    assert_eq!(page.card_value("stat-unverified"), "3");
    // Services : base qui répond, ni SMTP ni Discord dans ce hub de test.
    assert!(
        page.body.contains("SQLite · répond")
            || page.body.contains("PostgreSQL · répond")
            || page.body.contains("MySQL / MariaDB · répond")
    );
    assert!(page.body.contains("E-mails (SMTP)"));
    assert!(page.body.contains("Non configurée"));

    // En-têtes de sécurité, et aucune ressource venue d'un autre domaine.
    let csp = page.headers.get(header::CONTENT_SECURITY_POLICY).unwrap();
    assert!(csp.to_str().unwrap().contains("default-src 'self'"));
    assert_eq!(page.headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
    assert!(!page.body.contains("googleapis"));
    assert!(!page.body.contains("<script"));

    // Déjà connecté : la page de connexion renvoie au tableau de bord.
    assert_eq!(
        get(&app, "/admin/login", Some(&cookie)).await.location(),
        "/admin"
    );
    // L'entrée du menu de la page ouverte est mise en avant.
    let page = get(&app, "/admin/reglages", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("aria-current=\"page\">Réglages"));
}

#[tokio::test]
async fn compte_joueur_ou_mauvais_mot_de_passe_refuses() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    account(&app, "Mireille").await;

    for (email, password) in [
        ("mireille@example.org", PASSWORD),
        ("chef@example.org", "pas le bon"),
        ("inconnu@example.org", PASSWORD),
    ] {
        let page = try_login(&app, email, password).await;
        assert_eq!(page.status, StatusCode::UNAUTHORIZED, "{email}");
        assert!(page.set_cookie("rafthub_admin").is_none(), "{email}");
        // Même message dans tous les cas.
        assert!(
            page.body
                .contains("E-mail, pseudo ou mot de passe incorrect"),
            "{email}"
        );
    }
}

#[tokio::test]
async fn csrf_refuse() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;

    // Connexion : champ différent du cookie, ou cookie absent.
    let form = get(&app, "/admin/login", None).await;
    let csrf = form.csrf_field();
    let cookie = format!("rafthub_login_csrf={csrf}");
    let fields = [
        ("email", "chef@example.org"),
        ("password", PASSWORD),
        ("csrf", "faux"),
    ];
    let page = post_form(&app, "/admin/login", Some(&cookie), &fields).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    let fields = [
        ("email", "chef@example.org"),
        ("password", PASSWORD),
        ("csrf", &csrf),
    ];
    let page = post_form(&app, "/admin/login", None, &fields).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    assert!(page.set_cookie("rafthub_admin").is_none());

    // Déconnexion sans le bon jeton : refusée, la session reste ouverte.
    let session = login_cookie(&app, "chef@example.org").await;
    let page = post_form(&app, "/admin/logout", Some(&session), &[("csrf", "faux")]).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    let page = post_form(&app, "/admin/logout", Some(&session), &[]).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    assert_eq!(
        get(&app, "/admin", Some(&session)).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn deconnexion_ferme_la_session() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    let session = login_cookie(&app, "chef@example.org").await;
    let dashboard = get(&app, "/admin", Some(&session)).await;
    let csrf = dashboard.csrf_field();

    let page = post_form(&app, "/admin/logout", Some(&session), &[("csrf", &csrf)]).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(page.location(), "/admin/login");
    assert!(page
        .set_cookie("rafthub_admin")
        .unwrap()
        .contains("Max-Age=0"));
    // L'ancien cookie ne vaut plus rien, même renvoyé.
    assert_eq!(
        get(&app, "/admin", Some(&session)).await.location(),
        "/admin/login"
    );
}

#[tokio::test]
async fn retirer_le_role_ferme_l_acces() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    let session = login_cookie(&app, "chef@example.org").await;
    assert!(store
        .set_role_by_email("chef@example.org", Role::Player)
        .await
        .unwrap());
    assert_eq!(
        get(&app, "/admin", Some(&session)).await.location(),
        "/admin/login"
    );
    // Adresse inconnue : rien à changer.
    assert!(!store
        .set_role_by_email("personne@example.org", Role::Admin)
        .await
        .unwrap());
}

#[tokio::test]
async fn essais_rates_limites() {
    let config = HubConfig {
        login_max_failures: 2,
        ..HubConfig::for_tests()
    };
    let (app, store) = hub_with(config).await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    for _ in 0..2 {
        let page = try_login(&app, "chef@example.org", "pas le bon").await;
        assert_eq!(page.status, StatusCode::UNAUTHORIZED);
    }
    // Même avec le bon mot de passe, la porte reste fermée pour cette adresse.
    let page = try_login(&app, "chef@example.org", PASSWORD).await;
    assert_eq!(page.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(page.set_cookie("rafthub_admin").is_none());
    // L'API du joueur, elle, n'est pas bloquée par les essais sur la console.
    let (status, _) = common::login(&app, "chef@example.org", PASSWORD).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn cookie_secure_hors_de_la_machine() {
    let config = HubConfig {
        public_url: "https://hub.example.org".to_string(),
        ..HubConfig::for_tests()
    };
    let (app, store) = hub_with(config).await;
    make_admin(&app, &store, "chef@example.org", "Chef").await;
    let form = get(&app, "/admin/login", None).await;
    assert!(form
        .set_cookie("rafthub_login_csrf")
        .unwrap()
        .contains("; Secure"));
    let page = try_login(&app, "chef@example.org", PASSWORD).await;
    let cookie = page.set_cookie("rafthub_admin").unwrap();
    assert!(cookie.contains("; Secure"), "{cookie}");
}

#[tokio::test]
async fn fichiers_statiques_servis_par_le_hub() {
    let (app, store) = hub().await;
    // Avec un admin, la connexion s'affiche (sans admin, elle mène à l'installation).
    make_admin(&app, &store, "chef@exemple.org", "Chef").await;
    let css = get(&app, "/admin/static/rafthub.css", None).await;
    assert_eq!(css.status, StatusCode::OK);
    assert!(css.headers[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/css"));
    // Aucune adresse externe dans la feuille de style : les polices viennent du hub.
    assert!(!css.body.contains("http://") && !css.body.contains("https://"));
    for font in [
        "fonts/Inter.ttf",
        "fonts/Inter-SemiBold.ttf",
        "fonts/ChakraPetch-SemiBold.ttf",
        "fonts/ChakraPetch-Bold.ttf",
        "fonts/OFL-Inter.txt",
        "fonts/OFL-ChakraPetch.txt",
    ] {
        let page = get(&app, &format!("/admin/static/{font}"), None).await;
        assert_eq!(page.status, StatusCode::OK, "{font}");
    }
    for missing in ["/admin/static/../Cargo.toml", "/admin/static/secret.txt"] {
        assert_eq!(get(&app, missing, None).await.status, StatusCode::NOT_FOUND);
    }
    // La page de connexion est publique et ne charge rien d'ailleurs.
    let login = get(&app, "/admin/login", None).await;
    assert!(login.body.contains("/admin/static/rafthub.css"));
    assert!(!login.body.contains("googleapis"));
}

// ---------- Installation : premier admin depuis le navigateur (ADR 0037) ----------

const SETUP_CODE: &str = "K7QX-2M9P-AB12-CD34";

/// Hub neuf avec un code d'installation.
async fn fresh_hub() -> (Router, Store) {
    hub_with(HubConfig {
        setup_code: Some(SETUP_CODE.to_string()),
        ..HubConfig::for_tests()
    })
    .await
}

/// Ouvre la page d'installation puis envoie le formulaire.
async fn try_setup(app: &Router, code: &str, email: &str, username: &str) -> Page {
    let form = get(app, "/admin/installation", None).await;
    assert_eq!(form.status, StatusCode::OK, "{}", form.body);
    let csrf = form.csrf_field();
    let cookie = format!("rafthub_setup_csrf={csrf}");
    post_form(
        app,
        "/admin/installation",
        Some(&cookie),
        &[
            ("code", code),
            ("email", email),
            ("username", username),
            ("password", PASSWORD),
            ("password_confirm", PASSWORD),
            ("csrf", &csrf),
        ],
    )
    .await
}

#[tokio::test]
async fn hub_neuf_renvoie_la_connexion_vers_l_installation() {
    let (app, _) = fresh_hub().await;
    let page = get(&app, "/admin/login", None).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER);
    assert_eq!(page.location(), "/admin/installation");
}

#[tokio::test]
async fn installation_cree_le_premier_admin_et_le_connecte() {
    let (app, store) = fresh_hub().await;
    // Code tapé en minuscules et sans tirets : accepté.
    let page = try_setup(&app, "k7qx2m9pab12cd34", "chef@exemple.org", "Chef").await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin");
    let token = page.cookie_value("rafthub_admin").expect("session ouverte");
    assert!(store.has_admin().await.unwrap());
    let dashboard = get(&app, "/admin", Some(&format!("rafthub_admin={token}"))).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    // Le compte sert aussi pour la connexion normale.
    login_cookie(&app, "chef@exemple.org").await;

    // Une fois l'admin créé, la page d'installation est fermée.
    let again = get(&app, "/admin/installation", None).await;
    assert_eq!(again.location(), "/admin/login");
}

const SETUP_PASS: &str = "0123456789abcdef0123456789abcdef";

/// Hub qui sort de l'assistant : il connaît le jeton du cookie de l'étape 1.
async fn hub_after_wizard() -> (Router, Store) {
    hub_with(HubConfig {
        setup_code: Some(SETUP_CODE.to_string()),
        setup_pass: Some(SETUP_PASS.to_string()),
        ..HubConfig::for_tests()
    })
    .await
}

/// Comme `try_setup`, sans champ « code » et avec ce cookie d'étape 1.
async fn try_setup_with_pass(app: &Router, pass: &str) -> Page {
    let pass_cookie = format!("rafthub_setup_pass={pass}");
    let form = get(app, "/admin/installation", Some(&pass_cookie)).await;
    assert_eq!(form.status, StatusCode::OK, "{}", form.body);
    let csrf = form.csrf_field();
    let cookie = format!("rafthub_setup_csrf={csrf}; {pass_cookie}");
    post_form(
        app,
        "/admin/installation",
        Some(&cookie),
        &[
            ("email", "chef@exemple.org"),
            ("username", "Chef"),
            ("password", PASSWORD),
            ("password_confirm", PASSWORD),
            ("csrf", &csrf),
        ],
    )
    .await
}

#[tokio::test]
async fn installation_sans_retaper_le_code_apres_l_etape_1() {
    let (app, store) = hub_after_wizard().await;
    // Avec le cookie de l'étape 1, le champ du code n'est pas montré.
    let form = get(
        &app,
        "/admin/installation",
        Some(&format!("rafthub_setup_pass={SETUP_PASS}")),
    )
    .await;
    assert!(!form.body.contains("name=\"code\""), "{}", form.body);
    assert!(form.body.contains("vérifié à l'étape 1"));
    // Sans lui, le champ est là.
    let form = get(&app, "/admin/installation", None).await;
    assert!(form.body.contains("name=\"code\""));

    let page = try_setup_with_pass(&app, SETUP_PASS).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin");
    assert!(store.has_admin().await.unwrap());
    // Le cookie de l'étape 1 a servi : il est effacé.
    let cleared = page
        .set_cookie("rafthub_setup_pass")
        .expect("cookie effacé");
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

#[tokio::test]
async fn installation_refuse_un_faux_cookie_d_etape_1() {
    // Mauvais jeton : le code est demandé, et son absence est refusée.
    let (app, store) = hub_after_wizard().await;
    let page = try_setup_with_pass(&app, "ffffffffffffffffffffffffffffffff").await;
    assert_eq!(page.status, StatusCode::FORBIDDEN, "{}", page.body);
    assert!(!store.has_admin().await.unwrap());

    // Hub relancé (pas de jeton en mémoire) : un ancien cookie ne suffit pas.
    let (app, store) = fresh_hub().await;
    let page = try_setup_with_pass(&app, SETUP_PASS).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN, "{}", page.body);
    assert!(!store.has_admin().await.unwrap());
}

#[tokio::test]
async fn installation_refuse_un_mauvais_code() {
    let (app, store) = fresh_hub().await;
    let page = try_setup(&app, "AAAA-BBBB-CCCC-DDDD", "chef@exemple.org", "Chef").await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    assert!(
        page.body.contains("installation incorrect"),
        "{}",
        page.body
    );
    assert!(!store.has_admin().await.unwrap());
}

#[tokio::test]
async fn installation_fermee_sans_code_ou_avec_un_admin() {
    // Sans code (hub lancé autrement que par `rafthub`) : rien ne se crée.
    let (app, store) = hub().await;
    let form = get(&app, "/admin/installation", None).await;
    assert!(form.body.contains("Installation fermée"), "{}", form.body);
    let page = post_form(
        &app,
        "/admin/installation",
        Some("rafthub_setup_csrf=abc"),
        &[
            ("code", SETUP_CODE),
            ("email", "chef@exemple.org"),
            ("username", "Chef"),
            ("password", PASSWORD),
            ("password_confirm", PASSWORD),
            ("csrf", "abc"),
        ],
    )
    .await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    assert!(!store.has_admin().await.unwrap());

    // Avec un admin déjà là : le code ne sert plus.
    let (app, store) = fresh_hub().await;
    make_admin(&app, &store, "premier@exemple.org", "Premier").await;
    let form = get(&app, "/admin/installation", None).await;
    assert_eq!(form.location(), "/admin/login");
}

#[tokio::test]
async fn installation_refuse_un_csrf_absent() {
    let (app, store) = fresh_hub().await;
    let page = post_form(
        &app,
        "/admin/installation",
        None,
        &[
            ("code", SETUP_CODE),
            ("email", "chef@exemple.org"),
            ("username", "Chef"),
            ("password", PASSWORD),
            ("password_confirm", PASSWORD),
            ("csrf", "x"),
        ],
    )
    .await;
    assert_ne!(page.status, StatusCode::SEE_OTHER);
    assert!(!store.has_admin().await.unwrap());
}

#[tokio::test]
async fn installation_refuse_deux_mots_de_passe_differents() {
    let (app, store) = fresh_hub().await;
    let form = get(&app, "/admin/installation", None).await;
    let csrf = form.csrf_field();
    let page = post_form(
        &app,
        "/admin/installation",
        Some(&format!("rafthub_setup_csrf={csrf}")),
        &[
            ("code", SETUP_CODE),
            ("email", "chef@exemple.org"),
            ("username", "Chef"),
            ("password", PASSWORD),
            ("password_confirm", "autre mot de passe"),
            ("csrf", &csrf),
        ],
    )
    .await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(
        page.body.contains("ne sont pas identiques"),
        "{}",
        page.body
    );
    // Ce qui a été tapé reste dans le formulaire, sauf les mots de passe.
    assert!(page.body.contains("chef@exemple.org"));
    assert!(!page.body.contains(&format!("value=\"{PASSWORD}\"")));
    assert!(!store.has_admin().await.unwrap());
}

/// Rafthub auto-hébergé : la console ne montre ni ne règle Discord (ADR 0038).
#[tokio::test]
async fn console_sans_discord_hors_version_officielle() {
    let (app, store) = hub_with(HubConfig {
        discord_allowed: false,
        ..HubConfig::for_tests()
    })
    .await;
    make_admin(&app, &store, "chef@exemple.org", "Chef").await;
    let cookie = login_cookie(&app, "chef@exemple.org").await;

    let dashboard = get(&app, "/admin", Some(&cookie)).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(!dashboard.body.contains("Discord"), "{}", dashboard.body);

    let settings = get(&app, "/admin/reglages", Some(&cookie)).await;
    assert_eq!(settings.status, StatusCode::OK);
    assert!(!settings.body.contains("Discord"), "{}", settings.body);

    // Un formulaire Discord forgé est refusé, rien n'est écrit.
    let csrf = settings.csrf_field();
    let reply = post_form(
        &app,
        "/admin/reglages",
        Some(&cookie),
        &[
            ("csrf", &csrf),
            ("group", "discord"),
            ("discord_client_id", "123456789012345678"),
        ],
    )
    .await;
    assert_ne!(reply.location(), "/admin/reglages?notice=saved");
    assert!(!store
        .all_settings()
        .await
        .unwrap()
        .contains_key("discord_client_id"));
}

/// La console accepte aussi le pseudo à la connexion (ADR 0038).
#[tokio::test]
async fn connexion_console_par_pseudo() {
    let (app, store) = hub().await;
    make_admin(&app, &store, "chef@exemple.org", "Chef").await;
    let page = try_login(&app, "chef", PASSWORD).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin");
}
