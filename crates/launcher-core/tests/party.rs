//! Générer puis héberger (H1) : faux générateur, vrai `Host` sur 127.0.0.1.
//!
//! La génération étant exclusive dans le process, les tests prennent `SERIAL`.

// Le verrou SERIAL est gardé pendant les `await` exprès : une seule génération à la fois.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::await_holding_lock)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;

use launcher_core::generation::{GenerateRequest, GeneratorBackend};
use launcher_core::{
    generate_and_host, host_generated, Encryption, GenerationError, Host, LauncherError,
    PartyError, PartyHostOptions, Paths,
};
use py_bridge::{CancelToken, GenerateParams, GenerateResult, GeneratedPlayer};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn seed_bytes() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::read(path).expect("seed de test")
}

fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
    let file = std::fs::File::create(path).expect("zip");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        zip.start_file(*name, options).expect("entrée");
        zip.write_all(bytes).expect("écriture");
    }
    zip.finish().expect("fin");
}

/// Faux générateur : compte ses appels, puis écrit un zip avec la seed de test
/// ou échoue avec une erreur donnée.
struct FakeBackend {
    ap_dir: PathBuf,
    calls: u32,
    fail_with: Option<GenerationError>,
    /// Vrai : annule le jeton PENDANT `run`, puis répond `Cancelled`.
    cancel_in_run: bool,
}

impl FakeBackend {
    fn new(ap_dir: &Path) -> FakeBackend {
        FakeBackend {
            ap_dir: ap_dir.to_path_buf(),
            calls: 0,
            fail_with: None,
            cancel_in_run: false,
        }
    }
}

impl GeneratorBackend for FakeBackend {
    fn archipelago_dir(&self) -> &Path {
        &self.ap_dir
    }

    fn run(
        &mut self,
        p: &GenerateParams,
        _timeout: Duration,
        cancel: &CancelToken,
        _log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        self.calls += 1;
        if self.cancel_in_run {
            cancel.cancel();
            return Err(GenerationError::Cancelled);
        }
        if let Some(error) = self.fail_with.take() {
            return Err(error);
        }
        let zip = p.output_dir.join("AP_1.zip");
        write_zip(&zip, &[("AP_1.archipelago", seed_bytes())]);
        Ok(GenerateResult {
            zip,
            seed_name: "1".to_string(),
            ap_version: "0.0.0".to_string(),
            players: vec![GeneratedPlayer {
                slot: 1,
                name: "Alice".to_string(),
                game: "G".to_string(),
            }],
            seconds: 0.1,
        })
    }
}

fn request(dir: &Path) -> GenerateRequest {
    let file = dir.join("alice.yaml");
    std::fs::write(&file, "name: Alice\ngame: G\n").unwrap();
    GenerateRequest {
        player_files: vec![file],
        ..GenerateRequest::default()
    }
}

fn options(port: u16) -> PartyHostOptions {
    PartyHostOptions {
        port,
        password: None,
        encryption: Encryption::Plain,
        public_host: "127.0.0.1".to_string(),
        disable_save: true,
    }
}

#[tokio::test]
async fn generer_et_heberger() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let (tx, _rx) = channel();

    let started = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap();

    assert_eq!(backend.calls, 1);
    assert!(host.is_running());
    assert!(started.generated.archipelago_file.is_file());
    assert_eq!(started.status.seed.players, 3);
    assert_eq!(started.status.seed.seed_name, "72844992796894532502");
    assert!(
        started.status.address.starts_with("ws://127.0.0.1:"),
        "{}",
        started.status.address
    );
    assert!(!started.status.encrypted);
    host.stop().await.unwrap();
    assert!(!host.is_running());
}

// Même scénario sur un runtime multi-thread : c'est la branche `block_in_place`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generer_et_heberger_multi_thread() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let started = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap();
    assert_eq!(started.status.seed.players, 3);
    host.stop().await.unwrap();
}

#[tokio::test]
async fn deja_en_cours_ne_genere_pas() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let first = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap();
    assert_eq!(backend.calls, 1);

    let (tx, _rx) = channel();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PartyError::AlreadyRunning), "{error}");
    // Le générateur n'a pas été rappelé, et la première partie tourne toujours.
    assert_eq!(backend.calls, 1);
    assert!(host.status().await.is_some());

    // `host_generated` refuse aussi.
    let error = host_generated(&mut host, &first.generated, options(0))
        .await
        .unwrap_err();
    assert!(matches!(error, PartyError::AlreadyRunning), "{error}");
    host.stop().await.unwrap();
}

#[tokio::test]
async fn generation_annulee() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let cancel = CancelToken::new();
    cancel.cancel();
    let (tx, _rx) = channel();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &cancel,
        tx,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, PartyError::Generation(GenerationError::Cancelled)),
        "{error}"
    );
    assert!(!host.is_running());
    // Annulé avant l'appel : le générateur n'a rien lancé, aucun dossier ne traîne.
    assert_eq!(backend.calls, 0);
    assert_generations_empty(&paths);
}

/// Le générateur annule le jeton PENDANT `run` : le dossier de génération est supprimé.
#[tokio::test]
async fn annulation_pendant_la_generation() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    backend.cancel_in_run = true;
    let mut host = Host::new();
    let cancel = CancelToken::new();
    let (tx, _rx) = channel();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &cancel,
        tx,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, PartyError::Generation(GenerationError::Cancelled)),
        "{error}"
    );
    assert_eq!(backend.calls, 1);
    assert!(!host.is_running());
    assert_generations_empty(&paths);
}

fn assert_generations_empty(paths: &Paths) {
    match std::fs::read_dir(paths.generations_dir()) {
        Ok(mut entries) => assert!(entries.next().is_none(), "generations/ doit être vide"),
        Err(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
    }
}

#[tokio::test]
async fn echec_du_generateur() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    backend.fail_with = Some(GenerationError::GeneratorFailed {
        kind: "Boom".to_string(),
        message: "raté".to_string(),
        traceback: String::new(),
    });
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            PartyError::Generation(GenerationError::GeneratorFailed { .. })
        ),
        "{error}"
    );
    assert!(!host.is_running());
}

#[tokio::test]
async fn echec_de_l_hote_garde_la_seed() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    // Un port déjà pris : `Host::start` échoue après la génération.
    let busy = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let port = busy.local_addr().unwrap().port();
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        options(port),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap_err();
    // On récupère la sortie dans l'erreur et on réessaie sans régénérer.
    let PartyError::Host { source, generated } = error else {
        panic!("PartyError::Host attendue : {error}");
    };
    assert!(
        matches!(source, LauncherError::Server(server::ServerError::Io(_))),
        "{source}"
    );
    assert!(!host.is_running());
    assert_eq!(backend.calls, 1);
    // Le dossier de génération et la seed sont conservés pour réessayer.
    let kept: Vec<_> = std::fs::read_dir(paths.generations_dir())
        .unwrap()
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(generated.archipelago_file.is_file());
    let output = kept[0]
        .as_ref()
        .unwrap()
        .path()
        .join("output")
        .join("extracted");
    let has_seed = std::fs::read_dir(output).unwrap().any(|e| {
        e.unwrap()
            .path()
            .extension()
            .is_some_and(|x| x == "archipelago")
    });
    assert!(has_seed, "la seed doit rester sur le disque");

    // Réessai sur un port libre : l'hôte démarre, sans nouvelle génération.
    drop(busy);
    let status = host_generated(&mut host, &generated, options(0))
        .await
        .unwrap();
    assert!(host.is_running());
    assert_eq!(status.seed.players, 3);
    assert_eq!(backend.calls, 1);
    host.stop().await.unwrap();
}

/// Un bind raté ne doit pas laisser de `.raftup` : le réessai repart d'une partie neuve.
#[tokio::test]
async fn bind_rate_ne_cree_pas_de_sauvegarde() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let (tx, _rx) = channel();
    let generated = launcher_core::generate_blocking(
        &paths,
        &mut backend,
        &request(work.path()),
        &CancelToken::new(),
        tx,
    )
    .unwrap();

    let busy = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
    let port = busy.local_addr().unwrap().port();
    let mut with_save = options(port);
    with_save.disable_save = false;
    let mut host = Host::new();
    let error = host_generated(&mut host, &generated, with_save.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, PartyError::Host { .. }), "{error}");
    let save = server::save::default_save_path(&generated.archipelago_file);
    assert!(!save.exists(), "aucun .raftup après un bind raté");

    drop(busy);
    with_save.port = 0;
    let status = host_generated(&mut host, &generated, with_save)
        .await
        .unwrap();
    assert!(!status.resumed, "la partie ne doit pas « reprendre »");
    assert!(status.save.is_some());
    host.stop().await.unwrap();
}

/// Le `Future` de `generate_and_host` doit être `Send` (ne compile que si c'est vrai).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn le_future_est_send() {
    fn assert_send<T: Send>(_: &T) {}
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let req = request(work.path());
    let cancel = CancelToken::new();
    let future = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &req,
        options(0),
        &cancel,
        tx,
    );
    assert_send(&future);
    future.await.unwrap();
    host.stop().await.unwrap();
}

/// Hors de tout runtime tokio : pas de panique (`Handle::try_current`).
#[test]
fn hors_runtime_tokio_sans_panique() {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    // Mini exécuteur : le future échoue avant de toucher au réseau, donc un simple
    // `poll` suffit (aucun runtime tokio n'est nécessaire).

    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    backend.fail_with = Some(GenerationError::GeneratorFailed {
        kind: "Boom".to_string(),
        message: "raté".to_string(),
        traceback: String::new(),
    });
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let req = request(work.path());
    let cancel = CancelToken::new();
    let mut future = Box::pin(generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &req,
        options(0),
        &cancel,
        tx,
    ));
    let mut cx = Context::from_waker(Waker::noop());
    let Poll::Ready(result) = future.as_mut().poll(&mut cx) else {
        panic!("le future aurait dû finir au premier poll");
    };
    assert!(
        matches!(
            result,
            Err(PartyError::Generation(
                GenerationError::GeneratorFailed { .. }
            ))
        ),
        "{:?}",
        result.err()
    );
    drop(future);
    assert_eq!(backend.calls, 1);
}

#[test]
fn into_host_options_recopie_les_reglages() {
    let mut party = options(4242);
    party.password = Some("secret".to_string());
    let host = party
        .into_host_options(PathBuf::from("seed.archipelago"))
        .unwrap();
    assert_eq!(host.seed, PathBuf::from("seed.archipelago"));
    assert_eq!(host.port, 4242);
    assert_eq!(host.password.as_deref(), Some("secret"));
    assert_eq!(host.public_host, "127.0.0.1");
    assert!(host.disable_save);
}

#[test]
fn into_host_options_refuse_un_nom_public_vide() {
    for vide in ["", "   "] {
        let mut party = options(0);
        party.public_host = vide.to_string();
        let error = party.into_host_options(PathBuf::new()).unwrap_err();
        assert!(matches!(error, PartyError::EmptyPublicHost), "{error}");
    }
}

/// Le nom public vide est refusé AVANT de générer.
#[tokio::test]
async fn nom_public_vide_ne_genere_pas() {
    let _lock = serial();
    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let mut backend = FakeBackend::new(&work.path().join("ap"));
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let mut bad = options(0);
    bad.public_host = " ".to_string();
    let error = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &request(work.path()),
        bad,
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, PartyError::EmptyPublicHost), "{error}");
    assert_eq!(backend.calls, 0);
    assert_generations_empty(&paths);
}

/// Test réel : vrai runtime installé (`RAFTUP_REAL_RUNTIME`), jamais de téléchargement.
///
/// `cargo test -p launcher-core --test party -- --ignored --nocapture --test-threads=1`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "demande un runtime installé : RAFTUP_REAL_RUNTIME"]
async fn partie_reelle_apquest() {
    use launcher_core::generation::{pins, runtime_status, PythonBackend, RuntimeStatus};

    let Some(root) = std::env::var_os("RAFTUP_REAL_RUNTIME").map(PathBuf::from) else {
        eprintln!("RAFTUP_REAL_RUNTIME absente : test réel sauté");
        return;
    };
    let runtime = match runtime_status(&Paths::under(&root), &pins().unwrap()) {
        RuntimeStatus::Ready(runtime) => runtime,
        other => panic!("runtime non prêt dans {} : {other:?}", root.display()),
    };
    let mut backend = PythonBackend::new(runtime);

    let work = tempfile::tempdir().unwrap();
    let paths = Paths::under(work.path());
    let file = work.path().join("test.yaml");
    std::fs::write(
        &file,
        "name: Test\ngame: APQuest\nAPQuest:\n  hard_mode: false\n",
    )
    .unwrap();
    let req = GenerateRequest {
        player_files: vec![file],
        seed: Some(123456),
        timeout: Duration::from_secs(300),
        ..GenerateRequest::default()
    };
    let mut host = Host::new();
    let (tx, _rx) = channel();
    let started = generate_and_host(
        &mut host,
        &paths,
        &mut backend,
        &req,
        options(0),
        &CancelToken::new(),
        tx,
    )
    .await
    .unwrap();
    assert_eq!(started.status.seed.players, 1);
    assert_eq!(started.status.seed.slots[0].game, "APQuest");

    // Un vrai client rejoint la partie générée.
    let address = started.status.address.clone();
    let connect = client_core::ConnectOptions::new(&address, "Test", "APQuest");
    let (client, session) = client_core::Client::connect(&connect).await.unwrap();
    assert_eq!(session.connected.slot, 1);
    client.close().await.unwrap();
    host.stop().await.unwrap();
}
