//! Tests de l'orchestration de `generate` avec un faux générateur.
//!
//! Une seule génération à la fois étant la règle, tous les tests prennent `SERIAL`
//! pour ne pas se marcher dessus (les tests d'un même fichier tournent en parallèle).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;
use std::time::Duration;

use launcher_core::download::{DirFetcher, Downloader};
use launcher_core::generation::{
    ensure_runtime, generate, pins, GenerateRequest, GeneratorBackend, DEFAULT_SPOILER,
    DEFAULT_TIMEOUT,
};
use launcher_core::{GenerationError, Paths};
use py_bridge::{
    BridgeError, CancelToken, GenerateParams, GenerateResult, GeneratedPlayer, RpcError,
};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn seed_bytes() -> Vec<u8> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../multidata/tests/data/seed-mit.archipelago");
    std::fs::read(path).expect("seed de test")
}

/// Écrit un zip contenant `(nom, contenu)` à `path`.
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

/// Faux générateur : écrit un zip avec ces entrées dans `output_dir`.
struct FakeBackend {
    ap_dir: PathBuf,
    entries: Vec<(String, Vec<u8>)>,
    /// Les noms de fichiers joueurs vus au moment de l'appel.
    seen_players: Vec<String>,
    /// Nom de seed annoncé (le vrai zip s'appelle toujours `AP_1.zip`).
    seed_name: String,
    /// Chemin de zip annoncé à la place du vrai (un worker menteur).
    reported_zip: Option<PathBuf>,
    /// Si présent : `AP_1.zip` est un lien vers ce fichier au lieu d'un vrai fichier.
    zip_is_link_to: Option<PathBuf>,
}

impl FakeBackend {
    fn with_seed(ap_dir: &Path) -> FakeBackend {
        FakeBackend::with_entries(ap_dir, vec![("AP_1.archipelago".to_string(), seed_bytes())])
    }

    fn with_entries(ap_dir: &Path, entries: Vec<(String, Vec<u8>)>) -> FakeBackend {
        FakeBackend {
            ap_dir: ap_dir.to_path_buf(),
            entries,
            seen_players: Vec::new(),
            seed_name: "1".to_string(),
            reported_zip: None,
            zip_is_link_to: None,
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
        _cancel: &CancelToken,
        _log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        for entry in std::fs::read_dir(&p.player_files_dir).expect("dossier joueurs") {
            self.seen_players
                .push(entry.expect("entrée").file_name().to_string_lossy().into());
        }
        self.seen_players.sort();
        let zip = p.output_dir.join("AP_1.zip");
        let entries: Vec<(&str, Vec<u8>)> = self
            .entries
            .iter()
            .map(|(n, b)| (n.as_str(), b.clone()))
            .collect();
        match &self.zip_is_link_to {
            None => write_zip(&zip, &entries),
            Some(target) => {
                write_zip(target, &entries);
                if !make_file_link(target, &zip) {
                    // Pas le droit de créer un lien (Windows sans mode développeur) : on le
                    // signale par une erreur que le test reconnaît pour se terminer sans échec.
                    return Err(GenerationError::Seed("LIEN_IMPOSSIBLE".to_string()));
                }
            }
        }
        Ok(GenerateResult {
            zip: self.reported_zip.clone().unwrap_or(zip),
            seed_name: self.seed_name.clone(),
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

#[cfg(windows)]
fn make_file_link(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn make_file_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

/// Faux qui échoue toujours avec cette erreur.
struct FailingBackend {
    ap_dir: PathBuf,
    error: Option<GenerationError>,
}

impl FailingBackend {
    fn new(ap_dir: &Path, error: GenerationError) -> FailingBackend {
        FailingBackend {
            ap_dir: ap_dir.to_path_buf(),
            error: Some(error),
        }
    }
}

impl GeneratorBackend for FailingBackend {
    fn archipelago_dir(&self) -> &Path {
        &self.ap_dir
    }

    fn run(
        &mut self,
        _p: &GenerateParams,
        _t: Duration,
        _c: &CancelToken,
        _l: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        Err(self.error.take().expect("une seule fois"))
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    paths: Paths,
    /// Archipelago SOUS les données de Raftup : `custom_worlds` peut y être vidé.
    ap_dir: PathBuf,
    /// Un dossier d'Archipelago HORS des données de Raftup.
    external_dir: PathBuf,
    yaml: PathBuf,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().expect("tmp");
    let paths = Paths::under(tmp.path());
    let ap_dir = paths.data_dir.join("generator").join("archipelago");
    std::fs::create_dir_all(&ap_dir).expect("ap");
    let external_dir = tmp.path().join("archipelago-externe");
    std::fs::create_dir_all(&external_dir).expect("externe");
    let yaml = tmp.path().join("alice.yaml");
    std::fs::write(&yaml, "name: Alice\ngame: G\n").expect("yaml");
    Env {
        _tmp: tmp,
        paths,
        ap_dir,
        external_dir,
        yaml,
    }
}

fn request(files: Vec<PathBuf>) -> GenerateRequest {
    GenerateRequest {
        player_files: files,
        seed: Some(1),
        timeout: Duration::from_secs(60),
        ..GenerateRequest::default()
    }
}

fn run(
    e: &Env,
    backend: &mut dyn GeneratorBackend,
    req: &GenerateRequest,
) -> Result<launcher_core::generation::GenerateOutput, GenerationError> {
    let (tx, _rx) = channel();
    generate(&e.paths, backend, req, &CancelToken::new(), tx)
}

/// Les entrées de `generations/` (vide si le dossier n'existe pas).
fn generation_dirs(e: &Env) -> Vec<PathBuf> {
    std::fs::read_dir(e.paths.generations_dir())
        .map(|rd| rd.map(|x| x.expect("entrée").path()).collect())
        .unwrap_or_default()
}

#[test]
fn lit_les_slots_de_la_seed() {
    let _s = serial();
    let e = env();
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    let out = run(&e, &mut backend, &request(vec![e.yaml.clone()])).expect("génération");
    assert_eq!(out.seed_name, "1");
    assert!(out.archipelago_file.is_file());
    assert!(out.zip_file.is_file());
    assert!(!out.summary.slots.is_empty());
    assert_eq!(out.summary.slots[0].name, "Alice");
    assert_eq!(backend.seen_players, vec!["P01_alice.yaml".to_string()]);
    // Après un succès, le dossier de la génération reste.
    assert_eq!(generation_dirs(&e).len(), 1);
}

#[test]
fn valeurs_par_defaut_de_la_requete() {
    let req = GenerateRequest::default();
    assert_eq!(req.timeout, DEFAULT_TIMEOUT);
    assert_eq!(req.timeout, Duration::from_secs(15 * 60));
    assert_eq!(req.spoiler, DEFAULT_SPOILER);
    assert_eq!(req.spoiler, 3);
    assert!(req.player_files.is_empty() && req.apworlds.is_empty());
    assert!(req.seed.is_none() && !req.race);
}

#[test]
fn refuse_meta_yaml() {
    let _s = serial();
    let e = env();
    let meta = e.yaml.with_file_name("meta.yaml");
    std::fs::write(&meta, "meta_description: x\n").expect("meta");
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    match run(&e, &mut backend, &request(vec![meta])) {
        Err(GenerationError::InvalidPlayerFile { .. }) => {}
        other => panic!("attendu InvalidPlayerFile, reçu {other:?}"),
    }
}

#[test]
fn refuse_un_dossier_et_un_gros_fichier() {
    let _s = serial();
    let e = env();
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    let dir = e.yaml.with_file_name("dossier.yaml");
    std::fs::create_dir(&dir).expect("dossier");
    assert!(matches!(
        run(&e, &mut backend, &request(vec![dir])),
        Err(GenerationError::InvalidPlayerFile { .. })
    ));
    let big = e.yaml.with_file_name("gros.yaml");
    std::fs::write(&big, vec![b'a'; 1024 * 1024 + 1]).expect("gros");
    assert!(matches!(
        run(&e, &mut backend, &request(vec![big])),
        Err(GenerationError::InvalidPlayerFile { .. })
    ));
}

#[test]
fn aucun_joueur() {
    let _s = serial();
    let e = env();
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    assert!(matches!(
        run(&e, &mut backend, &request(Vec::new())),
        Err(GenerationError::NoPlayers)
    ));
}

#[test]
fn annulation_propagee_et_dossier_nettoye() {
    let _s = serial();
    let e = env();
    let mut backend =
        FailingBackend::new(&e.ap_dir, GenerationError::Bridge(BridgeError::Cancelled));
    assert!(matches!(
        run(&e, &mut backend, &request(vec![e.yaml.clone()])),
        Err(GenerationError::Cancelled)
    ));
    // Le dossier `generations/<horodatage>/` créé pour cette tentative a disparu.
    assert!(generation_dirs(&e).is_empty());
}

#[test]
fn erreur_du_generateur_avec_traceback_et_dossier_nettoye() {
    let _s = serial();
    let e = env();
    let rpc = RpcError {
        code: -32010,
        message: "impossible de placer".to_string(),
        data: Some(serde_json::json!({"type": "FillError", "traceback": "Traceback..."})),
    };
    let mut backend =
        FailingBackend::new(&e.ap_dir, GenerationError::Bridge(BridgeError::Worker(rpc)));
    match run(&e, &mut backend, &request(vec![e.yaml.clone()])) {
        Err(GenerationError::GeneratorFailed {
            kind, traceback, ..
        }) => {
            assert_eq!(kind, "FillError");
            assert_eq!(traceback, "Traceback...");
        }
        other => panic!("attendu GeneratorFailed, reçu {other:?}"),
    }
    assert!(generation_dirs(&e).is_empty());
}

#[test]
fn le_nettoyage_ne_touche_pas_aux_autres_generations() {
    let _s = serial();
    let e = env();
    // Une génération précédente réussie, à côté.
    let ancienne = e.paths.generations_dir().join("ancienne");
    std::fs::create_dir_all(&ancienne).expect("ancienne");
    std::fs::write(ancienne.join("seed.txt"), b"x").expect("seed");
    let mut backend = FailingBackend::new(&e.ap_dir, GenerationError::Cancelled);
    let _ = run(&e, &mut backend, &request(vec![e.yaml.clone()]));
    assert!(ancienne.join("seed.txt").is_file());
    assert_eq!(generation_dirs(&e), vec![ancienne]);
}

#[test]
fn zip_malveillant_refuse_et_dossier_nettoye() {
    let _s = serial();
    let e = env();
    let mut backend = FakeBackend::with_entries(
        &e.ap_dir,
        vec![("../evil.archipelago".to_string(), seed_bytes())],
    );
    match run(&e, &mut backend, &request(vec![e.yaml.clone()])) {
        Err(GenerationError::UnsafeOutput(_)) => {}
        other => panic!("attendu UnsafeOutput, reçu {other:?}"),
    }
    assert!(generation_dirs(&e).is_empty());
}

#[test]
fn zip_sans_archipelago() {
    let _s = serial();
    let e = env();
    let mut backend =
        FakeBackend::with_entries(&e.ap_dir, vec![("spoiler.txt".to_string(), b"x".to_vec())]);
    assert!(matches!(
        run(&e, &mut backend, &request(vec![e.yaml.clone()])),
        Err(GenerationError::NoOutput)
    ));
}

#[test]
fn le_chemin_du_zip_annonce_par_le_worker_est_ignore() {
    let _s = serial();
    let e = env();
    // Le worker annonce un zip hors du dossier de sortie, plein de bêtises : on ne l'ouvre pas,
    // on reconstruit `output/AP_<seed>.zip`.
    let dehors = e.yaml.with_file_name("hors-sortie.zip");
    std::fs::write(&dehors, b"pas un zip").expect("faux zip");
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    backend.reported_zip = Some(dehors.clone());
    let out = run(&e, &mut backend, &request(vec![e.yaml.clone()])).expect("génération");
    assert_eq!(out.zip_file.file_name().unwrap(), "AP_1.zip");
    assert_eq!(
        out.zip_file.parent().unwrap().file_name().unwrap(),
        "output"
    );
    assert_ne!(out.zip_file, dehors);

    // Même chose avec un chemin contenant `..`.
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    backend.reported_zip = Some(PathBuf::from("..").join("..").join("evil.zip"));
    let out = run(&e, &mut backend, &request(vec![e.yaml.clone()])).expect("génération");
    assert_eq!(out.zip_file.file_name().unwrap(), "AP_1.zip");
}

#[test]
fn nom_de_seed_piege_refuse() {
    let _s = serial();
    let e = env();
    for piege in ["../x", "a/b", "a\\b", "x.zip", "", "a b", "C:evil"] {
        let mut backend = FakeBackend::with_seed(&e.ap_dir);
        backend.seed_name = piege.to_string();
        match run(&e, &mut backend, &request(vec![e.yaml.clone()])) {
            Err(GenerationError::UnsafeOutput(_)) => {}
            other => panic!("« {piege} » : attendu UnsafeOutput, reçu {other:?}"),
        }
    }
    assert!(generation_dirs(&e).is_empty());
}

#[test]
fn zip_absent_sous_le_nom_attendu() {
    let _s = serial();
    let e = env();
    // Le worker annonce une autre seed que le fichier écrit (`AP_1.zip`) : introuvable.
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    backend.seed_name = "2".to_string();
    assert!(matches!(
        run(&e, &mut backend, &request(vec![e.yaml.clone()])),
        Err(GenerationError::Io { .. })
    ));
}

#[test]
fn zip_qui_est_un_lien_refuse() {
    let _s = serial();
    let e = env();
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    backend.zip_is_link_to = Some(e.yaml.with_file_name("cible.zip"));
    match run(&e, &mut backend, &request(vec![e.yaml.clone()])) {
        Err(GenerationError::UnsafeOutput(_)) => {}
        // Création de lien non permise sur cette machine : le test ne prouve rien, on s'arrête.
        Err(GenerationError::Seed(m)) if m == "LIEN_IMPOSSIBLE" => {}
        other => panic!("attendu UnsafeOutput, reçu {other:?}"),
    }
}

/// Faux qui prévient qu'il a démarré puis attend le feu vert.
struct BlockingBackend {
    ap_dir: PathBuf,
    started: Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl GeneratorBackend for BlockingBackend {
    fn archipelago_dir(&self) -> &Path {
        &self.ap_dir
    }

    fn run(
        &mut self,
        _p: &GenerateParams,
        _t: Duration,
        _c: &CancelToken,
        _l: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        let _ = self.started.send(());
        let _ = self.release.recv();
        Err(GenerationError::Cancelled)
    }
}

/// Lance une génération bloquée dans un thread ; rend de quoi la libérer.
fn start_blocked_generation(
    e: &Env,
) -> (
    std::thread::JoinHandle<Result<launcher_core::generation::GenerateOutput, GenerationError>>,
    Sender<()>,
) {
    let (started_tx, started_rx) = channel();
    let (release_tx, release_rx) = channel();
    let paths = e.paths.clone();
    let ap_dir = e.ap_dir.clone();
    let yaml = e.yaml.clone();
    let handle = std::thread::spawn(move || {
        let mut backend = BlockingBackend {
            ap_dir,
            started: started_tx,
            release: release_rx,
        };
        let (tx, _rx) = channel();
        generate(
            &paths,
            &mut backend,
            &request(vec![yaml]),
            &CancelToken::new(),
            tx,
        )
    });
    started_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("le premier a démarré");
    (handle, release_tx)
}

#[test]
fn une_seule_generation_a_la_fois() {
    let _s = serial();
    let e = env();
    let (handle, release_tx) = start_blocked_generation(&e);

    let mut second = FakeBackend::with_seed(&e.ap_dir);
    let result = run(&e, &mut second, &request(vec![e.yaml.clone()]));
    let _ = release_tx.send(());
    let first = handle.join().expect("thread");

    assert!(matches!(result, Err(GenerationError::Busy)), "{result:?}");
    assert!(matches!(first, Err(GenerationError::Cancelled)));
    // Le verrou est libéré : une nouvelle génération passe.
    let mut third = FakeBackend::with_seed(&e.ap_dir);
    run(&e, &mut third, &request(vec![e.yaml.clone()])).expect("après libération");
}

#[test]
fn ensure_runtime_refuse_pendant_une_generation() {
    let _s = serial();
    let e = env();
    let (handle, release_tx) = start_blocked_generation(&e);

    // Rien n'est installé : `ensure_runtime` voudrait installer, mais une génération tourne.
    let real_pins = pins().expect("pins");
    let downloader = Downloader::new(
        e.paths.data_dir.join("cache-dl"),
        Box::new(DirFetcher::new()),
    );
    let result = ensure_runtime(&e.paths, &real_pins, &downloader, &mut |_, _| {});
    let _ = release_tx.send(());
    let _ = handle.join().expect("thread");

    assert!(matches!(result, Err(GenerationError::Busy)), "{result:?}");
    // Rien n'a été téléchargé ni installé.
    assert!(!e.paths.data_dir.join("python").exists());
}

fn apworld(path: &Path, manifest: &str) {
    write_zip(
        path,
        &[
            ("mon_monde/archipelago.json", manifest.as_bytes().to_vec()),
            ("mon_monde/__init__.py", b"".to_vec()),
        ],
    );
}

#[test]
fn apworld_incompatible_refuse() {
    let _s = serial();
    let e = env();
    std::fs::write(e.ap_dir.join("Utils.py"), "__version__ = \"0.6.0\"\n").expect("Utils");
    let world = e.yaml.with_file_name("mon_monde.apworld");
    apworld(&world, r#"{"game": "G", "minimum_ap_version": "0.7.0"}"#);
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![world];
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    assert!(matches!(
        run(&e, &mut backend, &req),
        Err(GenerationError::IncompatibleApworld { .. })
    ));
}

#[test]
fn apworld_trop_ancien_pour_cet_archipelago_refuse() {
    let _s = serial();
    let e = env();
    std::fs::write(e.ap_dir.join("Utils.py"), "__version__ = \"0.6.8\"\n").expect("Utils");
    let world = e.yaml.with_file_name("mon_monde.apworld");
    apworld(&world, r#"{"game": "G", "maximum_ap_version": "0.6.2"}"#);
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![world.clone()];
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    match run(&e, &mut backend, &req) {
        Err(GenerationError::IncompatibleApworld { reason, .. }) => {
            assert!(reason.contains("au maximum"), "{reason}");
        }
        other => panic!("attendu IncompatibleApworld, reçu {other:?}"),
    }
    // Le même apworld avec un maximum suffisant passe.
    apworld(&world, r#"{"game": "G", "maximum_ap_version": "0.6.8"}"#);
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    run(&e, &mut backend, &req).expect("compatible");
}

#[test]
fn apworld_compatible_copie_et_vide_custom_worlds() {
    let _s = serial();
    let e = env();
    std::fs::write(e.ap_dir.join("Utils.py"), "__version__ = \"0.6.8\"\n").expect("Utils");
    let custom = e.ap_dir.join("custom_worlds");
    std::fs::create_dir_all(&custom).expect("custom");
    std::fs::write(custom.join("ancien.apworld"), b"x").expect("ancien");
    let world = e.yaml.with_file_name("mon_monde.apworld");
    apworld(&world, r#"{"game": "G", "minimum_ap_version": "0.6.0"}"#);
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![world];
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    run(&e, &mut backend, &req).expect("génération");
    assert!(custom.join("mon_monde.apworld").is_file());
    assert!(!custom.join("ancien.apworld").exists());
}

#[test]
fn archipelago_hors_des_donnees_de_raftup_jamais_vide() {
    let _s = serial();
    let e = env();
    let custom = e.external_dir.join("custom_worlds");
    std::fs::create_dir_all(&custom).expect("custom");
    std::fs::write(custom.join("a_moi.apworld"), b"x").expect("fichier");
    let world = e.yaml.with_file_name("mon_monde.apworld");
    apworld(&world, r#"{"game": "G"}"#);

    // Avec un apworld demandé : refus typé, rien n'est touché.
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![world];
    let mut backend = FakeBackend::with_seed(&e.external_dir);
    match run(&e, &mut backend, &req) {
        Err(GenerationError::UnmanagedArchipelago(p)) => assert_eq!(p, e.external_dir),
        other => panic!("attendu UnmanagedArchipelago, reçu {other:?}"),
    }
    assert!(custom.join("a_moi.apworld").is_file());
    assert!(!custom.join("mon_monde.apworld").exists());
    assert!(generation_dirs(&e).is_empty());

    // Sans apworld : la génération passe et `custom_worlds` reste intact.
    let mut backend = FakeBackend::with_seed(&e.external_dir);
    run(&e, &mut backend, &request(vec![e.yaml.clone()])).expect("génération");
    assert!(custom.join("a_moi.apworld").is_file());
}

#[test]
fn le_dossier_de_donnees_lui_meme_n_est_pas_un_archipelago_gere() {
    let _s = serial();
    let e = env();
    let custom = e.paths.data_dir.join("custom_worlds");
    std::fs::create_dir_all(&custom).expect("custom");
    std::fs::write(custom.join("x.apworld"), b"x").expect("fichier");
    let world = e.yaml.with_file_name("mon_monde.apworld");
    apworld(&world, r#"{"game": "G"}"#);
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![world];
    let mut backend = FakeBackend::with_seed(&e.paths.data_dir);
    assert!(matches!(
        run(&e, &mut backend, &req),
        Err(GenerationError::UnmanagedArchipelago(_))
    ));
    assert!(custom.join("x.apworld").is_file());
}

#[test]
fn deux_apworlds_de_meme_nom_refuses() {
    let _s = serial();
    let e = env();
    let un = e.yaml.with_file_name("a").join("mon_monde.apworld");
    let deux = e.yaml.with_file_name("b").join("MON_MONDE.apworld");
    for p in [&un, &deux] {
        std::fs::create_dir_all(p.parent().unwrap()).expect("dossier");
        apworld(p, r#"{"game": "G"}"#);
    }
    let custom = e.ap_dir.join("custom_worlds");
    std::fs::create_dir_all(&custom).expect("custom");
    std::fs::write(custom.join("ancien.apworld"), b"x").expect("ancien");
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![un, deux];
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    match run(&e, &mut backend, &req) {
        Err(GenerationError::IncompatibleApworld { reason, .. }) => {
            assert!(reason.contains("même nom"), "{reason}");
        }
        other => panic!("attendu IncompatibleApworld, reçu {other:?}"),
    }
    // Refus avant tout effacement.
    assert!(custom.join("ancien.apworld").is_file());
}

#[test]
fn apworld_mauvaise_extension() {
    let _s = serial();
    let e = env();
    let bad = e.yaml.with_file_name("monde.zip");
    std::fs::write(&bad, b"x").expect("zip");
    let mut req = request(vec![e.yaml.clone()]);
    req.apworlds = vec![bad];
    let mut backend = FakeBackend::with_seed(&e.ap_dir);
    assert!(matches!(
        run(&e, &mut backend, &req),
        Err(GenerationError::IncompatibleApworld { .. })
    ));
}

/// Bout en bout avec le vrai worker et le faux Archipelago de py-bridge.
/// `RAFTUP_PYTHON=C:\...\python.exe cargo test -p launcher-core --test generation -- --ignored`
#[test]
#[ignore = "demande RAFTUP_PYTHON"]
fn bout_en_bout_avec_faux_archipelago() {
    use launcher_core::generation::{PythonBackend, Runtime};
    let _s = serial();
    let e = env();
    let python = std::env::var("RAFTUP_PYTHON").expect("RAFTUP_PYTHON non défini");
    // On copie les .py du faux Archipelago dans le dossier temporaire : la génération
    // vide et recrée `custom_worlds`, on ne veut pas toucher au dépôt.
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../py-bridge/tests/fixtures/fake_archipelago");
    for entry in std::fs::read_dir(&source).expect("fixture") {
        let path = entry.expect("entrée").path();
        if path.extension().is_some_and(|x| x == "py") {
            std::fs::copy(&path, e.ap_dir.join(path.file_name().expect("nom"))).expect("copie");
        }
    }
    let mut backend =
        PythonBackend::new(Runtime::from_local(python, &e.ap_dir, None, "0.0.0-fake"));
    let (tx, rx) = channel();
    let result = generate(
        &e.paths,
        &mut backend,
        &request(vec![e.yaml.clone()]),
        &CancelToken::new(),
        tx,
    );
    // Le faux Archipelago écrit « fake » dans son .archipelago : le zip est bien produit et
    // extrait, mais la seed est illisible, ce qui prouve que tout le chemin a été parcouru
    // (le message « Lecture de la seed produite » est envoyé juste avant l'extraction).
    match result {
        Err(GenerationError::Seed(_)) => {}
        other => panic!("attendu Seed(..), reçu {other:?}"),
    }
    let messages: Vec<String> = rx.try_iter().collect();
    assert!(
        messages.iter().any(|m| m.contains("Lecture de la seed")),
        "{messages:?}"
    );
    // L'échec a nettoyé le dossier de la génération.
    assert!(generation_dirs(&e).is_empty());
}

/// ADR 0019, décision 3 : le backend Python bloque le réseau par défaut.
#[test]
fn python_backend_blocks_network_by_default() {
    use launcher_core::generation::{PythonBackend, Runtime};
    let backend = PythonBackend::new(Runtime::from_local("python", "ap", None, "0.6.8"));
    assert!(backend.block_network);
}
