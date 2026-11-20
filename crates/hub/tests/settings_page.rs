//! Page « Réglages » de la console (ADR 0036) : secrets jamais réaffichés, valeur
//! invalide refusée sans rien écrire, CSRF, source de chaque réglage, retour au
//! défaut, journal et e-mail de test. Tout passe par le routeur en `oneshot`.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use hub::store::Role;
use hub::{AppState, EnvSettings, HubConfig, MemoryMailer, SettingKey, Store};
use tower::ServiceExt;

use common::{register, test_store};

const PASSWORD: &str = "mot de passe solide";
const ADMIN_EMAIL: &str = "chef@example.org";
const SMTP: &str = "smtps://moi:motdepasse-secret@smtp.exemple.org:465";
const DISCORD_SECRET: &str = "secret-discord-xyz";

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

    fn csrf_field(&self) -> String {
        let marker = "name=\"csrf\" value=\"";
        let start = self.body.find(marker).expect("champ csrf absent") + marker.len();
        let end = self.body[start..].find('"').unwrap();
        self.body[start..start + end].to_string()
    }

    /// Morceau de la page qui affiche le réglage `name` (jusqu'au réglage suivant).
    fn setting_row(&self, name: &str) -> &str {
        let start = self
            .body
            .find(&format!("id=\"setting-{name}\""))
            .expect("réglage absent de la page");
        let rest = &self.body[start + 1..];
        let end = rest
            .find("id=\"setting-")
            .or_else(|| rest.find("</section>"))
            .unwrap_or(rest.len());
        &rest[..end]
    }
}

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

/// Hub de test prêt : un admin connecté. Rend le routeur, l'état (pour lire le
/// journal), la base, le cookie de session et le jeton anti-CSRF.
struct Hub {
    app: Router,
    state: AppState,
    store: Store,
    cookie: String,
    csrf: String,
}

impl Hub {
    async fn start(config: HubConfig, mailer: Arc<MemoryMailer>) -> Hub {
        let store = test_store().await;
        let state = AppState::with_mailer(store.clone(), config, mailer)
            .await
            .unwrap();
        let app = hub::router(state.clone());
        let (status, _) = register(&app, ADMIN_EMAIL, "Chef", PASSWORD).await;
        assert_eq!(status, StatusCode::CREATED);
        assert!(store
            .set_role_by_email(ADMIN_EMAIL, Role::Admin)
            .await
            .unwrap());

        let form = get(&app, "/admin/login", None).await;
        let login_csrf = form.csrf_field();
        let page = post_form(
            &app,
            "/admin/login",
            Some(&format!("rafthub_login_csrf={login_csrf}")),
            &[
                ("email", ADMIN_EMAIL),
                ("password", PASSWORD),
                ("csrf", &login_csrf),
            ],
        )
        .await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
        let cookie = format!(
            "rafthub_admin={}",
            page.cookie_value("rafthub_admin").unwrap()
        );
        let csrf = get(&app, "/admin/reglages", Some(&cookie))
            .await
            .csrf_field();
        Hub {
            app,
            state,
            store,
            cookie,
            csrf,
        }
    }

    async fn default() -> Hub {
        Hub::start(HubConfig::for_tests(), Arc::new(MemoryMailer::new())).await
    }

    async fn page(&self) -> Page {
        let page = get(&self.app, "/admin/reglages", Some(&self.cookie)).await;
        assert_eq!(page.status, StatusCode::OK);
        page
    }

    /// Envoie un formulaire de la page, avec le bon jeton anti-CSRF.
    async fn post(&self, fields: &[(&str, &str)]) -> Page {
        let mut all = vec![("csrf", self.csrf.as_str())];
        all.extend_from_slice(fields);
        post_form(&self.app, "/admin/reglages", Some(&self.cookie), &all).await
    }

    /// Détails des lignes `setting_changed` du journal, du plus récent au plus ancien.
    async fn setting_events(&self) -> Vec<String> {
        let events = self.state.events(200, 0).await.unwrap();
        events
            .items
            .into_iter()
            .filter(|e| e.kind == "setting_changed")
            .map(|e| e.detail)
            .collect()
    }
}

#[tokio::test]
async fn secret_jamais_dans_le_html() {
    let hub = Hub::default().await;
    let page = hub
        .post(&[
            ("group", "email"),
            ("smtp_url", SMTP),
            ("mail_from", "Rafthub <no-reply@exemple.org>"),
        ])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin/reglages?notice=saved");
    let page = hub
        .post(&[
            ("group", "discord"),
            ("discord_client_id", "123456"),
            ("discord_client_secret", DISCORD_SECRET),
        ])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);

    let page = get(&hub.app, "/admin/reglages?notice=saved", Some(&hub.cookie)).await;
    assert!(page.body.contains("Réglages enregistrés."));
    assert!(
        !page.body.contains("motdepasse-secret"),
        "secret SMTP affiché"
    );
    assert!(
        !page.body.contains(DISCORD_SECRET),
        "secret Discord affiché"
    );
    // Seul l'hôte du serveur SMTP est montré.
    let smtp = page.setting_row("smtp_url");
    assert!(
        smtp.contains("Configurée · serveur smtp.exemple.org"),
        "{smtp}"
    );
    assert!(
        smtp.contains("value=\"\""),
        "le champ d'un secret reste vide"
    );
    assert!(page
        .setting_row("discord_client_secret")
        .contains("Configuré"));
    // Une valeur ordinaire, elle, est remise dans son champ.
    assert!(page
        .setting_row("discord_client_id")
        .contains("value=\"123456\""));

    // Le journal ne garde que les noms.
    let events = hub.setting_events().await;
    for name in [
        "smtp_url",
        "mail_from",
        "discord_client_id",
        "discord_client_secret",
    ] {
        assert!(events.iter().any(|d| d == name), "{name} : {events:?}");
    }
    assert!(events
        .iter()
        .all(|d| !d.contains("motdepasse") && !d.contains(DISCORD_SECRET)));

    // Champ secret laissé vide à l'envoi : le secret reste en place.
    let page = hub
        .post(&[
            ("group", "email"),
            ("smtp_url", ""),
            ("mail_from", "Autre <autre@exemple.org>"),
        ])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    let settings = hub.state.settings().await.unwrap();
    assert_eq!(settings.smtp_url.as_deref(), Some(SMTP));
    assert_eq!(
        settings.mail_from.as_deref(),
        Some("Autre <autre@exemple.org>")
    );
}

#[tokio::test]
async fn valeur_invalide_refusee_sans_rien_ecrire() {
    let hub = Hub::default().await;
    let page = hub
        .post(&[
            ("group", "general"),
            ("hub_name", "Nouveau nom"),
            ("public_url", "javascript:alert(1)"),
        ])
        .await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(page.body.contains("role=\"alert\""), "{}", page.body);
    assert!(page.body.contains("Valeur refusée : adresse publique"));
    // Le groupe est refusé en entier : le nom, pourtant valable, n'est pas écrit.
    assert!(hub.store.all_settings().await.unwrap().is_empty());
    assert!(hub.setting_events().await.is_empty());

    // Une valeur hors liste, envoyée à la main, est refusée aussi.
    let page = hub
        .post(&[("group", "registration"), ("registration_mode", "ouvert")])
        .await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    // Un SMTP illisible : le message ne recopie pas la valeur.
    let page = hub
        .post(&[("group", "email"), ("smtp_url", "motdepasse-secret")])
        .await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(!page.body.contains("motdepasse-secret"));
    // Groupe ou réglage inconnu.
    let page = hub.post(&[("group", "inconnu")]).await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    let page = hub.post(&[("reset", "inconnu")]).await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(hub.store.all_settings().await.unwrap().is_empty());
}

#[tokio::test]
async fn csrf_refuse() {
    let hub = Hub::default().await;
    let fields = [("group", "general"), ("hub_name", "Pirate")];
    // Sans jeton, ou avec un faux.
    let page = post_form(&hub.app, "/admin/reglages", Some(&hub.cookie), &fields).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    let mut forged = vec![("csrf", "faux")];
    forged.extend_from_slice(&fields);
    let page = post_form(&hub.app, "/admin/reglages", Some(&hub.cookie), &forged).await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    // Sans session : vers la connexion.
    let page = post_form(&hub.app, "/admin/reglages", None, &fields).await;
    assert_eq!(page.location(), "/admin/login");
    assert!(hub.store.all_settings().await.unwrap().is_empty());
}

#[tokio::test]
async fn source_affichee_et_retour_au_defaut() {
    let mut config = HubConfig::for_tests();
    config.settings_env = EnvSettings::from_lookup(|name| {
        (name == "RAFTUP_HUB_NAME").then(|| "Hub de l'environnement".to_string())
    });
    let hub = Hub::start(config, Arc::new(MemoryMailer::new())).await;

    let page = hub.page().await;
    let row = page.setting_row("hub_name");
    assert!(row.contains("source-environment"), "{row}");
    assert!(
        row.contains("value=\"Hub de l&#39;environnement\""),
        "{row}"
    );
    assert!(!row.contains("Revenir au défaut"), "rien en base à effacer");
    assert!(page
        .setting_row("registration_mode")
        .contains("source-default"));

    // Enregistrer change la source ; la même valeur renvoyée ne réécrit rien.
    let page = hub
        .post(&[("group", "registration"), ("registration_mode", "invite")])
        .await;
    assert_eq!(page.location(), "/admin/reglages?notice=saved");
    let page = hub
        .post(&[
            ("group", "general"),
            ("hub_name", "Hub de l'environnement"),
            ("public_url", "http://127.0.0.1:8787"),
        ])
        .await;
    assert_eq!(page.location(), "/admin/reglages?notice=unchanged");
    assert_eq!(hub.setting_events().await, vec!["registration_mode"]);

    let page = hub.page().await;
    let row = page.setting_row("registration_mode");
    assert!(row.contains("source-database"), "{row}");
    assert!(row.contains("<option value=\"invite\" selected>"), "{row}");
    assert!(
        row.contains("name=\"reset\" value=\"registration_mode\""),
        "{row}"
    );
    // Le nom vient toujours de l'environnement.
    assert!(page.setting_row("hub_name").contains("source-environment"));

    // « Revenir au défaut » efface la ligne en base.
    let page = hub.post(&[("reset", "registration_mode")]).await;
    assert_eq!(page.location(), "/admin/reglages?notice=reset");
    assert!(hub.store.all_settings().await.unwrap().is_empty());
    let page = get(&hub.app, "/admin/reglages?notice=reset", Some(&hub.cookie)).await;
    assert!(page.body.contains("Réglage remis à sa valeur par défaut."));
    assert!(page
        .setting_row("registration_mode")
        .contains("source-default"));
    assert_eq!(
        hub.setting_events().await.first().map(String::as_str),
        Some("registration_mode (valeur retirée)")
    );
    // Un `?notice=` inconnu n'est pas recopié.
    let page = get(
        &hub.app,
        "/admin/reglages?notice=%3Cscript%3E",
        Some(&hub.cookie),
    )
    .await;
    assert!(!page.body.contains("<script"));
}

#[tokio::test]
async fn page_publique_desactivable() {
    let hub = Hub::default().await;
    let page = hub
        .post(&[("group", "download"), ("public_download", "false")])
        .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert!(!hub.state.settings().await.unwrap().public_download);
    let page = hub.page().await;
    assert!(page
        .setting_row(SettingKey::PublicDownload.name())
        .contains("<option value=\"false\" selected>"));
}

#[tokio::test]
async fn email_de_test_envoye_a_l_admin() {
    let mailer = Arc::new(MemoryMailer::new());
    let hub = Hub::start(HubConfig::for_tests(), Arc::clone(&mailer)).await;
    // L'inscription de l'admin a déjà envoyé un e-mail de vérification.
    let before = mailer.sent().len();
    let page = hub.post(&[("action", "test_email")]).await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body);
    assert_eq!(page.location(), "/admin/reglages?notice=test_sent");
    let sent = mailer.sent();
    assert_eq!(sent.len(), before + 1);
    let last = sent.last().unwrap();
    assert_eq!(last.to, ADMIN_EMAIL);
    assert!(last.subject.contains("e-mail de test"));
}

#[tokio::test]
async fn email_de_test_sans_smtp() {
    // Hub sans service d'envoi : la page le dit, rien ne part.
    let store = test_store().await;
    let state = AppState::new(store.clone(), HubConfig::for_tests())
        .await
        .unwrap();
    let app = hub::router(state);
    let (status, _) = register(&app, ADMIN_EMAIL, "Chef", PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(store
        .set_role_by_email(ADMIN_EMAIL, Role::Admin)
        .await
        .unwrap());
    let form = get(&app, "/admin/login", None).await;
    let login_csrf = form.csrf_field();
    let page = post_form(
        &app,
        "/admin/login",
        Some(&format!("rafthub_login_csrf={login_csrf}")),
        &[
            ("email", ADMIN_EMAIL),
            ("password", PASSWORD),
            ("csrf", &login_csrf),
        ],
    )
    .await;
    let cookie = format!(
        "rafthub_admin={}",
        page.cookie_value("rafthub_admin").unwrap()
    );
    let csrf = get(&app, "/admin/reglages", Some(&cookie))
        .await
        .csrf_field();
    let page = post_form(
        &app,
        "/admin/reglages",
        Some(&cookie),
        &[("csrf", &csrf), ("action", "test_email")],
    )
    .await;
    assert_eq!(page.status, StatusCode::BAD_REQUEST);
    assert!(
        page.body.contains("E-mail de test impossible"),
        "{}",
        page.body
    );
}
