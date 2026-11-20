//! Distribution du launcher (ADR 0036) : rafraîchissement du cache, signature,
//! zip relié au hub, pages admin et publique. Aucun réseau : la source de l'exe est
//! un faux en mémoire, et l'exe est signé avec une clé générée dans le test.

// Les aides des tests paniquent en cas d'erreur : c'est voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use hub::distribution::{
    DistributionConfig, DistributionError, ReleaseSource, UPDATE_MANIFEST_URL, UPDATE_PUBLIC_KEY,
};
use hub::store::Role;
use hub::{AppState, HubConfig, SettingKey, Store};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use common::{register, test_store};

const PASSWORD: &str = "mot de passe solide";
const EXE_URL: &str = "https://github.com/Yonako25/raftup/releases/download/v0.3.0/raftup.exe";
const HUB_URL: &str = "https://hub.exemple.org";

// ---------- Faux GitHub ----------

/// Sert des fichiers en mémoire et note les adresses demandées.
#[derive(Default)]
struct FakeRelease {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
    asked: Mutex<Vec<String>>,
}

impl FakeRelease {
    fn put(&self, url: &str, bytes: Vec<u8>) {
        self.files.lock().unwrap().insert(url.to_string(), bytes);
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ReleaseSource for FakeRelease {
    fn fetch(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>, DistributionError> {
        self.asked.lock().unwrap().push(url.to_string());
        let bytes = self
            .files
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or_else(|| DistributionError::Network {
                url: url.to_string(),
                reason: "404".into(),
            })?;
        if bytes.len() as u64 > max_bytes {
            return Err(DistributionError::TooLarge(max_bytes));
        }
        Ok(bytes)
    }
}

/// Paire de clés minisign générée pour le test.
struct TestKey {
    pair: minisign::KeyPair,
}

impl TestKey {
    fn new() -> TestKey {
        TestKey {
            pair: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
        }
    }
    fn public(&self) -> String {
        self.pair.pk.to_base64()
    }
    fn sign(&self, data: &[u8]) -> String {
        minisign::sign(
            Some(&self.pair.pk),
            &self.pair.sk,
            Cursor::new(data),
            Some("test"),
            Some("signature de test"),
        )
        .unwrap()
        .into_string()
    }
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Manifeste de l'ADR 0022 pour `exe`, signé par `signer`.
fn manifest(version: &str, url: &str, exe: &[u8], signer: &TestKey) -> Vec<u8> {
    serde_json::json!({
        "format_version": 1,
        "version": version,
        "url": url,
        "sha256": sha256_hex(exe),
        "max_bytes": 1024 * 1024,
        "signature": signer.sign(exe),
        "notes": "champ ignoré par le hub"
    })
    .to_string()
    .into_bytes()
}

// ---------- Hub de test ----------

struct Hub {
    app: Router,
    store: Store,
    source: Arc<FakeRelease>,
    key: TestKey,
    cache: tempfile::TempDir,
}

/// Hub dont la distribution lit `source`, avec la clé `key`, et des réglages choisis.
async fn start_hub(settings: &[(SettingKey, &str)]) -> Hub {
    let key = TestKey::new();
    let source = Arc::new(FakeRelease::default());
    let cache = tempfile::tempdir().unwrap();
    let mut config = HubConfig::for_tests();
    for (setting, value) in settings {
        config.settings_env.set(*setting, value).unwrap();
    }
    config.distribution = DistributionConfig {
        cache_dir: cache.path().join("launcher"),
        manifest_url: UPDATE_MANIFEST_URL.to_string(),
        public_key: key.public(),
        // `as Arc<dyn …>` : on range le faux derrière le trait attendu.
        source: Arc::clone(&source) as Arc<dyn ReleaseSource>,
    };
    let store = test_store().await;
    let state = AppState::new(store.clone(), config).await.unwrap();
    Hub {
        app: hub::router(state),
        store,
        source,
        key,
        cache,
    }
}

async fn hub_with_url() -> Hub {
    start_hub(&[(SettingKey::PublicUrl, HUB_URL)]).await
}

/// Publie sur le faux GitHub un exe signé par la clé du hub.
fn publish(hub: &Hub, version: &str, exe: &[u8]) {
    hub.source.put(
        UPDATE_MANIFEST_URL,
        manifest(version, EXE_URL, exe, &hub.key),
    );
    hub.source.put(EXE_URL, exe.to_vec());
}

// ---------- Requêtes ----------

struct Page {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Page {
    fn body(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
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
            .and_then(|c| Some(c.split(';').next()?.split_once('=')?.1.to_string()))
    }
    fn csrf_field(&self) -> String {
        let body = self.body();
        let marker = "name=\"csrf\" value=\"";
        let start = body.find(marker).expect("champ csrf absent") + marker.len();
        let end = body[start..].find('"').unwrap();
        body[start..start + end].to_string()
    }
}

async fn send(app: &Router, request: Request<Body>) -> Page {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 16 << 20)
        .await
        .unwrap()
        .to_vec();
    Page {
        status,
        headers,
        bytes,
    }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> Page {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    send(app, builder.body(Body::empty()).unwrap()).await
}

async fn post_form(app: &Router, uri: &str, cookie: &str, body: String) -> Page {
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .body(Body::from(body))
        .unwrap();
    send(app, request).await
}

/// Crée un admin, le connecte et rend son cookie de session.
async fn admin_cookie(hub: &Hub) -> String {
    let (status, _) = register(&hub.app, "chef@example.org", "Chef", PASSWORD).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(hub
        .store
        .set_role_by_email("chef@example.org", Role::Admin)
        .await
        .unwrap());
    let form = get(&hub.app, "/admin/login", None).await;
    let csrf = form.csrf_field();
    let page = post_form(
        &hub.app,
        "/admin/login",
        &format!("rafthub_login_csrf={csrf}"),
        format!("email=chef%40example.org&password=mot+de+passe+solide&csrf={csrf}"),
    )
    .await;
    assert_eq!(page.status, StatusCode::SEE_OTHER, "{}", page.body());
    format!(
        "rafthub_admin={}",
        page.cookie_value("rafthub_admin").unwrap()
    )
}

/// Clique « Rafraîchir » sur la page d'admin et rend la page obtenue.
async fn refresh(hub: &Hub, cookie: &str) -> Page {
    let csrf = get(&hub.app, "/admin/distribution", Some(cookie))
        .await
        .csrf_field();
    post_form(
        &hub.app,
        "/admin/distribution/rafraichir",
        cookie,
        format!("csrf={csrf}"),
    )
    .await
}

/// Fichiers d'un zip : nom → contenu.
fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut files = BTreeMap::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        files.insert(file.name().unwrap().to_string(), data);
    }
    files
}

// ---------- Tests ----------

#[tokio::test]
async fn rafraichir_puis_telecharger_le_zip_relie() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    let exe = b"MZ faux launcher signe".to_vec();
    publish(&hub, "0.3.0", &exe);

    // Avant : rien en cache, rien à servir.
    let page = get(&hub.app, "/admin/distribution", Some(&cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body().contains("Aucun launcher en cache"));
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::SERVICE_UNAVAILABLE);

    let page = refresh(&hub, &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    let body = page.body();
    assert!(
        body.contains("Launcher 0.3.0 téléchargé, signature vérifiée."),
        "{body}"
    );
    assert!(body.contains("Version 0.3.0"));
    assert!(body.contains("Signature OK"));
    assert!(body.contains("raftup://hub/hub.exemple.org"));
    assert!(body.contains("/admin/static/copy.js"));
    assert_eq!(hub.source.asked(), vec![UPDATE_MANIFEST_URL, EXE_URL]);

    // Le zip public : l'exe inchangé et `rafthub.toml`.
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::OK);
    assert_eq!(zip.headers[header::CONTENT_TYPE], "application/zip");
    assert!(zip.headers[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .contains("raftup-windows.zip"));
    let files = unzip(&zip.bytes);
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        vec!["rafthub.toml", "raftup.exe"]
    );
    assert_eq!(files["raftup.exe"], exe);
    let toml: toml::Table = String::from_utf8(files["rafthub.toml"].clone())
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(toml["format_version"].as_integer(), Some(1));
    assert_eq!(toml["hub_url"].as_str(), Some(HUB_URL));

    // Le même zip pour l'admin, sous `/admin`.
    let admin = get(
        &hub.app,
        "/admin/distribution/raftup-windows.zip",
        Some(&cookie),
    )
    .await;
    assert_eq!(admin.status, StatusCode::OK);
    assert_eq!(unzip(&admin.bytes), files);

    // La page publique propose le téléchargement.
    let public = get(&hub.app, "/telecharger", None).await;
    assert_eq!(public.status, StatusCode::OK);
    let body = public.body();
    assert!(body.contains("id=\"download\""), "{body}");
    assert!(body.contains("raftup://hub/hub.exemple.org"));
    assert!(body.contains("Rafthub"));
}

#[tokio::test]
async fn signature_d_une_autre_cle_refusee() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    let exe = b"MZ exe pirate".to_vec();
    let intruder = TestKey::new();
    hub.source.put(
        UPDATE_MANIFEST_URL,
        manifest("0.3.0", EXE_URL, &exe, &intruder),
    );
    hub.source.put(EXE_URL, exe);

    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("pas la signature officielle"), "{body}");
    assert!(body.contains("Aucun launcher en cache"));
    // Rien n'a été rangé sur le disque.
    assert!(!hub
        .cache
        .path()
        .join("launcher")
        .join("raftup.exe")
        .exists());
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn exe_different_du_manifeste_refuse() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ vrai");
    // GitHub sert un autre fichier que celui annoncé.
    hub.source.put(EXE_URL, b"MZ autre".to_vec());
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("pas la signature officielle"), "{body}");
    assert!(body.contains("Aucun launcher en cache"));
}

#[tokio::test]
async fn adresse_hors_liste_blanche_jamais_demandee() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    let exe = b"MZ".to_vec();
    let evil = "https://evil.example.com/raftup.exe";
    hub.source
        .put(UPDATE_MANIFEST_URL, manifest("0.3.0", evil, &exe, &hub.key));
    hub.source.put(evil, exe);
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("liste blanche"), "{body}");
    assert_eq!(hub.source.asked(), vec![UPDATE_MANIFEST_URL]);
}

#[tokio::test]
async fn exe_modifie_sur_le_disque_jamais_servi() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ officiel");
    refresh(&hub, &cookie).await;
    // Premier téléchargement : le zip est fabriqué et mémorisé.
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::OK);

    // Quelqu'un remplace l'exe du cache.
    std::fs::write(
        hub.cache.path().join("launcher").join("raftup.exe"),
        b"MZ trafique",
    )
    .unwrap();
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::SERVICE_UNAVAILABLE);
    let page = get(&hub.app, "/admin/distribution", Some(&cookie)).await;
    assert!(page.body().contains("Signature invalide"));
    let public = get(&hub.app, "/telecharger", None).await;
    assert!(public.body().contains("id=\"not-ready\""));
}

#[tokio::test]
async fn pas_de_retour_en_arriere() {
    let hub = hub_with_url().await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ 0.3");
    refresh(&hub, &cookie).await;
    publish(&hub, "0.2.0", b"MZ 0.2");
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("plus ancienne"), "{body}");
    assert!(body.contains("Version 0.3.0"));
    // Une version plus récente remplace le cache.
    publish(&hub, "0.4.0", b"MZ 0.4");
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("Version 0.4.0"), "{body}");
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(unzip(&zip.bytes)["raftup.exe"], b"MZ 0.4");
}

#[tokio::test]
async fn sans_adresse_publique_le_zip_est_refuse() {
    // Adresse par défaut (http://127.0.0.1:8787) : un joueur ne pourrait pas s'y relier.
    let hub = start_hub(&[]).await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ");
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("id=\"url-problem\""), "{body}");
    assert!(!body.contains("raftup://hub/"), "pas de lien sans HTTPS");
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(zip.body().contains("adresse publique"));
    let admin = get(
        &hub.app,
        "/admin/distribution/raftup-windows.zip",
        Some(&cookie),
    )
    .await;
    assert_eq!(admin.status, StatusCode::SERVICE_UNAVAILABLE);

    // Une adresse en HTTP vers un autre PC est refusée aussi (le launcher la rejetterait).
    let other = start_hub(&[(SettingKey::PublicUrl, "http://hub.exemple.org")]).await;
    publish(&other, "0.3.0", b"MZ");
    let cookie = admin_cookie(&other).await;
    refresh(&other, &cookie).await;
    let zip = get(&other.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn adresse_locale_reglee_acceptee() {
    let hub = start_hub(&[(SettingKey::PublicUrl, "http://127.0.0.1:8787")]).await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ");
    refresh(&hub, &cookie).await;
    let zip = get(&hub.app, "/telecharger/raftup-windows.zip", None).await;
    assert_eq!(zip.status, StatusCode::OK);
    let toml = String::from_utf8(unzip(&zip.bytes)["rafthub.toml"].clone()).unwrap();
    assert!(
        toml.contains("hub_url = \"http://127.0.0.1:8787\""),
        "{toml}"
    );
}

#[tokio::test]
async fn page_publique_desactivee() {
    let hub = start_hub(&[
        (SettingKey::PublicUrl, HUB_URL),
        (SettingKey::PublicDownload, "false"),
    ])
    .await;
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ");
    let body = refresh(&hub, &cookie).await.body();
    assert!(body.contains("Désactivée dans les réglages"), "{body}");

    assert_eq!(
        get(&hub.app, "/telecharger", None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&hub.app, "/telecharger/raftup-windows.zip", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    // L'admin connecté garde son téléchargement.
    let admin = get(
        &hub.app,
        "/admin/distribution/raftup-windows.zip",
        Some(&cookie),
    )
    .await;
    assert_eq!(admin.status, StatusCode::OK);
}

#[tokio::test]
async fn admin_protege_et_csrf() {
    let hub = hub_with_url().await;
    for uri in [
        "/admin/distribution",
        "/admin/distribution/raftup-windows.zip",
    ] {
        let page = get(&hub.app, uri, None).await;
        assert_eq!(page.status, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(page.location(), "/admin/login");
    }
    let cookie = admin_cookie(&hub).await;
    publish(&hub, "0.3.0", b"MZ");
    let page = post_form(
        &hub.app,
        "/admin/distribution/rafraichir",
        &cookie,
        "csrf=faux".into(),
    )
    .await;
    assert_eq!(page.status, StatusCode::FORBIDDEN);
    assert!(hub.source.asked().is_empty(), "rien téléchargé sans CSRF");
}

#[tokio::test]
async fn copie_js_servi() {
    let hub = hub_with_url().await;
    let page = get(&hub.app, "/admin/static/copy.js", None).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body().contains("clipboard"));
}

/// Le hub recopie la clé publique et l'adresse du manifeste de `launcher-core` (il n'en
/// dépend pas) : elles doivent rester égales.
#[test]
fn constantes_identiques_a_launcher_core() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../launcher-core/src/update.rs");
    let source = std::fs::read_to_string(path).unwrap();
    assert!(
        source.contains(&format!(
            "pub const UPDATE_PUBLIC_KEY: &str = \"{UPDATE_PUBLIC_KEY}\";"
        )),
        "clé publique différente de launcher_core::update"
    );
    assert!(
        source.contains(&format!("\"{UPDATE_MANIFEST_URL}\";")),
        "adresse du manifeste différente de launcher_core::update"
    );
}
