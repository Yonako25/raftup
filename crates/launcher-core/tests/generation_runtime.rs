//! Tests de l'installation du runtime de génération : aucun réseau, archives fabriquées ici.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use launcher_core::download::{DirFetcher, Downloader};
use launcher_core::generation::{
    ensure_runtime, parse_pins, runtime_status, GenerationError, GeneratorPins, RuntimeStatus,
    SetupStep,
};
use launcher_core::hash::Sha256;
use launcher_core::Paths;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const PREFIX: &str = "Archipelago-abc";

// `ensure_runtime` prend le verrou global (une seule installation/génération à la fois dans
// ce process) : les tests de ce fichier tournent donc l'un après l'autre.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        builder.append_data(&mut header, name, *data).unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

/// Un faux monde : fichiers sources, pins et fetcher cohérents entre eux.
struct World {
    tmp: tempfile::TempDir,
    pins: GeneratorPins,
    fetcher: DirFetcher,
}

/// `wheel_b` : contenu du zip de la deuxième wheel (permet d'en fabriquer une mauvaise).
fn world_with(wheel_b: Vec<u8>, python_hash_override: Option<Sha256>) -> World {
    world_full(wheel_b, python_hash_override, None)
}

/// Comme `world_with`, avec en plus un faux hash pour la DERNIÈRE wheel (`pkgb`).
fn world_full(
    wheel_b: Vec<u8>,
    python_hash_override: Option<Sha256>,
    wheel_b_hash_override: Option<Sha256>,
) -> World {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();

    let python = build_tar_gz(&[("python/python.exe", b"EXE"), ("python/Lib/os.py", b"# os")]);
    let ap = build_zip(&[
        (&format!("{PREFIX}/Generate.py"), b"print(1)"),
        (&format!("{PREFIX}/worlds/a.py"), b"# a"),
        ("hors_prefixe.txt", b"hors prefixe"),
    ]);
    let wheel_a = build_zip(&[
        ("pkga/__init__.py", b"A"),
        ("pkga-1.dist-info/METADATA", b"m"),
    ]);

    let files: [(&str, &[u8]); 4] = [
        ("python.tar.gz", &python),
        ("ap.zip", &ap),
        ("pkga-1-py3-none-any.whl", &wheel_a),
        ("pkgb-1-py3-none-any.whl", &wheel_b),
    ];
    for (name, bytes) in files {
        fs::write(src.join(name), bytes).unwrap();
    }

    let py_hash = python_hash_override.unwrap_or_else(|| Sha256::of_bytes(&python));
    let toml = format!(
        r#"
format_version = 1
[python]
version = "3.12.0+1"
url = "https://github.com/t/python.tar.gz"
sha256 = "{py_hash}"
size = 1000
max_bytes = 100000
strip_prefix = "python"
max_entries = 100
[archipelago]
version = "0.6.8"
url = "https://github.com/t/ap.zip"
sha256 = "{ap_hash}"
size = 2000
max_bytes = 100000
strip_prefix = "{PREFIX}"
max_entries = 100
[[wheels]]
name = "pkga"
version = "1"
filename = "pkga-1-py3-none-any.whl"
url = "https://files.pythonhosted.org/pkga-1-py3-none-any.whl"
sha256 = "{a_hash}"
size = 30
max_bytes = 100000
[[wheels]]
name = "pkgb"
version = "1"
filename = "pkgb-1-py3-none-any.whl"
url = "https://files.pythonhosted.org/pkgb-1-py3-none-any.whl"
sha256 = "{b_hash}"
size = 4
max_bytes = 100000
"#,
        ap_hash = Sha256::of_bytes(&ap),
        a_hash = Sha256::of_bytes(&wheel_a),
        b_hash = wheel_b_hash_override.unwrap_or_else(|| Sha256::of_bytes(&wheel_b)),
    );
    let pins = parse_pins(&toml).unwrap();
    let fetcher = fetcher_for(&src, &pins.archipelago.url, "ap.zip");
    World { tmp, pins, fetcher }
}

/// Un `DirFetcher` qui connaît les 4 URL des pins de test ; `ap_url` peut être remplacée.
fn fetcher_for(src: &Path, ap_url: &str, ap_file: &str) -> DirFetcher {
    DirFetcher::new()
        .with_file(
            "https://github.com/t/python.tar.gz",
            src.join("python.tar.gz"),
        )
        .with_file(ap_url, src.join(ap_file))
        .with_file(
            "https://files.pythonhosted.org/pkga-1-py3-none-any.whl",
            src.join("pkga-1-py3-none-any.whl"),
        )
        .with_file(
            "https://files.pythonhosted.org/pkgb-1-py3-none-any.whl",
            src.join("pkgb-1-py3-none-any.whl"),
        )
}

fn good_world() -> World {
    world_with(build_zip(&[("pkgb/__init__.py", b"B")]), None)
}

/// Donne le fetcher du monde au téléchargeur (le cache est dans le dossier temporaire).
fn downloader(world: &mut World) -> Downloader {
    let fetcher = std::mem::take(&mut world.fetcher);
    Downloader::new(world.tmp.path().join("cache"), Box::new(fetcher))
}

fn paths(world: &World) -> Paths {
    Paths::under(&world.tmp.path().join("raftup"))
}

fn tree_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

const FINAL_NAMES: [&str; 3] = ["archipelago", "runtime.json", "site-packages"];

#[test]
fn installation_complete_et_disposition() {
    let _s = serial();
    let mut w = good_world();
    let paths = paths(&w);
    assert!(matches!(
        runtime_status(&paths, &w.pins),
        RuntimeStatus::Missing { .. }
    ));
    let dl = downloader(&mut w);
    let mut steps = Vec::new();
    let rt = ensure_runtime(&paths, &w.pins, &dl, &mut |s, _| {
        if steps.last() != Some(&s) {
            steps.push(s);
        }
    })
    .unwrap();

    assert_eq!(rt.ap_version, "0.6.8");
    assert_eq!(fs::read(&rt.python_exe).unwrap(), b"EXE");
    assert_eq!(
        rt.python_exe,
        paths.python_dir("3.12.0+1").join("python.exe")
    );
    assert!(rt.archipelago_dir.join("Generate.py").is_file());
    assert!(rt.archipelago_dir.join("worlds/a.py").is_file());
    assert!(!rt.archipelago_dir.join("hors_prefixe.txt").exists());
    let site = rt.site_packages.clone().unwrap();
    assert!(site.join("pkga/__init__.py").is_file());
    assert!(site.join("pkgb/__init__.py").is_file());
    // Marqueur écrit ; aucun `.partial` ni `.tmp` ne reste.
    assert_eq!(tree_names(&paths.generator_dir("0.6.8")), FINAL_NAMES);
    assert_eq!(tree_names(&paths.data_dir.join("python")), vec!["3.12.0+1"]);
    assert!(matches!(
        runtime_status(&paths, &w.pins),
        RuntimeStatus::Ready(_)
    ));
    assert_eq!(steps.first(), Some(&SetupStep::Python));
    assert_eq!(steps.last(), Some(&SetupStep::Finalize));
    assert!(steps.contains(&SetupStep::Wheel("pkgb".into())));
}

#[test]
fn deuxieme_appel_ne_telecharge_rien() {
    let _s = serial();
    let mut w = good_world();
    let paths = paths(&w);
    let opens = w.fetcher.open_counter();
    let dl = downloader(&mut w);
    ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap();
    let after_first = opens.load(Ordering::SeqCst);
    assert_eq!(after_first, 4);
    ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), after_first);
}

#[test]
fn mauvais_hash_ne_laisse_rien() {
    let _s = serial();
    // Le mauvais hash est sur la DERNIÈRE wheel : tout le reste a déjà été téléchargé et
    // vérifié, et pourtant rien n'a été extrait (« tout vérifié avant extraction »).
    let mut w = world_full(
        build_zip(&[("pkgb/__init__.py", b"B")]),
        None,
        Some(Sha256::of_bytes(b"autre chose")),
    );
    let paths = paths(&w);
    let dl = downloader(&mut w);
    let err = ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap_err();
    assert!(matches!(err, GenerationError::Download(_)), "{err:?}");
    assert!(!paths.data_dir.join("python").exists());
    assert!(!paths.data_dir.join("generator").exists());
    assert!(matches!(
        runtime_status(&paths, &w.pins),
        RuntimeStatus::Missing { .. }
    ));
}

#[test]
fn taille_a_telecharger_somme_des_tailles_reelles() {
    let _s = serial();
    let w = good_world();
    let paths = paths(&w);
    // 1000 (python) + 2000 (archipelago) + 30 + 4 (wheels), pas la somme des plafonds.
    match runtime_status(&paths, &w.pins) {
        RuntimeStatus::Missing { download_bytes } => assert_eq!(download_bytes, 3034),
        other => panic!("attendu Missing, reçu {other:?}"),
    }
}

#[test]
fn installation_interrompue_est_refaite() {
    let _s = serial();
    let mut w = good_world();
    let paths = paths(&w);
    // Une tentative interrompue : restes `.partial`, un dossier final sans marqueur.
    let generator = paths.generator_dir("0.6.8");
    fs::create_dir_all(generator.join("archipelago.partial")).unwrap();
    fs::write(generator.join("archipelago.partial/reste.txt"), b"x").unwrap();
    fs::create_dir_all(generator.join("archipelago")).unwrap();
    fs::write(generator.join("archipelago/vieux.txt"), b"x").unwrap();
    assert!(matches!(
        runtime_status(&paths, &w.pins),
        RuntimeStatus::Missing { .. }
    ));

    let dl = downloader(&mut w);
    let rt = ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap();
    assert!(rt.archipelago_dir.join("Generate.py").is_file());
    assert!(!rt.archipelago_dir.join("vieux.txt").exists());
    assert_eq!(tree_names(&generator), FINAL_NAMES);
}

#[test]
fn pin_modifie_donne_broken_puis_reinstallation() {
    let _s = serial();
    let mut w = good_world();
    let paths = paths(&w);
    let dl = downloader(&mut w);
    ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap();

    // Même version d'Archipelago, autre contenu : le hash du pin change.
    let src = w.tmp.path().join("src");
    let ap2 = build_zip(&[(&format!("{PREFIX}/Generate.py"), b"print(2)")]);
    fs::write(src.join("ap2.zip"), &ap2).unwrap();
    let mut changed = w.pins.clone();
    changed.archipelago.sha256 = Sha256::of_bytes(&ap2);
    changed.archipelago.url = "https://github.com/t/ap2.zip".to_string();
    let dl2 = Downloader::new(
        w.tmp.path().join("cache"),
        Box::new(fetcher_for(&src, &changed.archipelago.url, "ap2.zip")),
    );

    assert!(matches!(
        runtime_status(&paths, &changed),
        RuntimeStatus::Broken(_)
    ));
    let rt = ensure_runtime(&paths, &changed, &dl2, &mut |_, _| {}).unwrap();
    assert_eq!(
        fs::read(rt.archipelago_dir.join("Generate.py")).unwrap(),
        b"print(2)"
    );
    assert!(matches!(
        runtime_status(&paths, &changed),
        RuntimeStatus::Ready(_)
    ));
    // Aucun `.stale.partial` ne traîne.
    assert_eq!(tree_names(&paths.generator_dir("0.6.8")), FINAL_NAMES);
}

#[test]
fn wheel_avec_dossier_data_refusee() {
    let _s = serial();
    let mut w = world_with(
        build_zip(&[
            ("pkgb/__init__.py", b"B"),
            ("pkgb-1.data/scripts/x.py", b"x"),
        ]),
        None,
    );
    let paths = paths(&w);
    let dl = downloader(&mut w);
    let err = ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap_err();
    assert!(
        matches!(err, GenerationError::UnsupportedWheel { ref reason, .. } if reason.contains(".data")),
        "{err:?}"
    );
    // Pas de marqueur, pas de site-packages, pas de reste.
    let generator = paths.generator_dir("0.6.8");
    assert!(!generator.join("runtime.json").exists());
    assert!(!generator.join("site-packages").exists());
    assert!(!generator.join("site-packages.partial").exists());
}

#[test]
fn wheel_avec_chemin_dangereux_refusee() {
    let _s = serial();
    let mut w = world_with(build_zip(&[("../evil.py", b"x")]), None);
    let paths = paths(&w);
    let dl = downloader(&mut w);
    let err = ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap_err();
    assert!(matches!(err, GenerationError::Archive(_)), "{err:?}");
}

#[test]
fn meme_fichier_dans_deux_wheels_refuse() {
    let _s = serial();
    // `pkgb` réécrit `pkga/__init__.py` (autre casse) : refusé, sans rien écrire.
    let mut w = world_with(
        build_zip(&[("PKGA/__init__.PY", b"autre"), ("pkgb/__init__.py", b"B")]),
        None,
    );
    let paths = paths(&w);
    let dl = downloader(&mut w);
    let err = ensure_runtime(&paths, &w.pins, &dl, &mut |_, _| {}).unwrap_err();
    assert!(
        matches!(err, GenerationError::DuplicateWheelFile { ref wheel, .. } if wheel.contains("pkgb")),
        "{err:?}"
    );
    let generator = paths.generator_dir("0.6.8");
    assert!(!generator.join("runtime.json").exists());
    assert!(!generator.join("site-packages").exists());
    assert!(!generator.join("site-packages.partial").exists());
}
