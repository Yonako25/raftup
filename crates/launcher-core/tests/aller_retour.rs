//! Tests d'aller-retour de l'installation (T8, ADR 0015 « Réversibilité ») :
//! installer puis désinstaller doit rendre au joueur le dossier exact qu'il avait.
//! Aucun réseau : `DirFetcher` sert des zips fabriqués ici.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use launcher_core::hash::Sha256;
use launcher_core::{
    install, status, uninstall, DirFetcher, Downloader, InstallOptions, InstallRecord,
    InstallStatus, Paths, Profile, RecordState,
};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const GAME: &str = "jeu";
const URL_V1: &str = "https://github.com/x/y/v1.zip";
const URL_V2: &str = "https://github.com/x/y/v2.zip";

// Le verrou « un seul install par jeu » est global au processus : les tests de ce
// fichier, qui partagent le même `game_id`, passent l'un après l'autre.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Env {
    _serial: std::sync::MutexGuard<'static, ()>,
    tmp: tempfile::TempDir,
    paths: Paths,
    game_dir: PathBuf,
}

fn env() -> Env {
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(&tmp.path().join("app"));
    let game_dir = tmp.path().join("Mon Jeu");
    fs::create_dir_all(&game_dir).unwrap();
    Env {
        _serial: serial,
        tmp,
        paths,
        game_dir,
    }
}

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

/// Un dossier de jeu « réaliste » : un exe, un sous-dossier, un fichier que le mod va remplacer.
fn fill_game_dir(env: &Env) {
    write(&env.game_dir.join("Jeu.exe"), b"EXE ORIGINAL");
    write(&env.game_dir.join("Data").join("niveau1.dat"), b"NIVEAU");
    write(&env.game_dir.join("version.dll"), b"DLL ORIGINALE");
    fs::create_dir_all(env.game_dir.join("Vide")).unwrap();
}

/// Photo d'un dossier : chaque chemin relatif → hash du fichier (`None` pour un dossier).
/// `BTreeMap` : ordre trié, donc deux photos se comparent directement avec `==`.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Sha256>> {
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Sha256>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        if path.is_dir() {
            out.insert(rel, None);
            walk(root, &path, out);
        } else {
            out.insert(rel, Some(Sha256::of_bytes(&fs::read(&path).unwrap())));
        }
    }
}

/// Fabrique un zip dans le dossier temporaire et rend (chemin, hash).
fn make_zip(env: &Env, name: &str, entries: &[(&str, &[u8])]) -> (PathBuf, Sha256) {
    let mut writer = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (entry, data) in entries {
        writer
            .start_file(*entry, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let path = env.tmp.path().join("zips").join(name);
    write(&path, &bytes);
    (path, Sha256::of_bytes(&bytes))
}

/// Profil à une étape qui extrait l'archive dans `{game_dir}`.
fn profile(version: &str, url: &str, sha: Sha256, preserve: &[&str]) -> Profile {
    let list: Vec<String> = preserve.iter().map(|p| format!("\"{p}\"")).collect();
    let text = format!(
        "format_version = 1\n[game]\nid = \"{GAME}\"\nname = \"Jeu\"\nap_game = \"Jeu AP\"\n\
         status = \"available\"\n[credits]\nauthors = [\"x\"]\nlicense = \"MIT\"\n\
         [[steps]]\nid = \"loader\"\nkind = \"mod\"\nname = \"M\"\nversion = \"{version}\"\n\
         source = {{ url = \"{url}\" }}\nsha256 = \"{sha}\"\n\
         install = {{ extract = \"zip\", into = \"{{game_dir}}\" }}\n\
         [preserve]\npaths = [{}]\n[launch]\nexe = \"{{game_dir}}/Jeu.exe\"\n\
         connect = {{ method = \"manual\", instructions = \"x\" }}\n",
        list.join(", ")
    );
    Profile::parse(&format!("{GAME}.toml"), &text).unwrap()
}

fn downloader(env: &Env, files: &[(&str, &Path)]) -> Downloader {
    let mut fetcher = DirFetcher::new();
    for (url, path) in files {
        fetcher = fetcher.with_file(url, *path);
    }
    Downloader::new(env.paths.downloads_dir(), Box::new(fetcher))
}

fn run(env: &Env, profile: &Profile, dl: &Downloader) {
    let options = InstallOptions {
        force: false,
        ap_version: "0.6.1".to_string(),
    };
    install(
        &env.paths,
        profile,
        &env.game_dir,
        dl,
        &options,
        &mut |_, _| {},
    )
    .unwrap();
}

/// Aucune sauvegarde ne doit traîner une fois le jeu désinstallé.
fn assert_no_backups(env: &Env) {
    let dir = env.paths.backups_dir(GAME);
    assert!(
        !dir.exists() || snapshot(&dir).values().all(Option::is_none),
        "des sauvegardes restent dans {}",
        dir.display()
    );
}

#[test]
fn installer_puis_desinstaller_rend_un_dossier_identique() {
    let env = env();
    fill_game_dir(&env);
    let before = snapshot(&env.game_dir);

    let (zip, sha) = make_zip(
        &env,
        "v1.zip",
        &[
            ("version.dll", b"DLL DU MOD"),
            ("Mods/Rando/rando.dll", b"RANDO"),
            ("Data/niveau1.dat", b"NIVEAU PATCHE"),
        ],
    );
    let p = profile("1.0", URL_V1, sha, &[]);
    run(&env, &p, &downloader(&env, &[(URL_V1, &zip)]));
    assert_ne!(
        snapshot(&env.game_dir),
        before,
        "l'installation n'a rien changé"
    );
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::Installed);

    let report = uninstall(&env.paths, GAME, false).unwrap();
    assert!(report.conflicts.is_empty());
    assert_eq!(snapshot(&env.game_dir), before);
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::NotInstalled);
    assert_no_backups(&env);
}

#[test]
fn mise_a_jour_v1_v2_garde_les_fichiers_preserves() {
    let env = env();
    fill_game_dir(&env);
    let before = snapshot(&env.game_dir);
    let cfg = env.game_dir.join("UserData").join("cfg.json");
    let preserve = ["{game_dir}/UserData/cfg.json"];

    // v1 pose une config par défaut que le joueur modifie ensuite.
    let (zip1, sha1) = make_zip(
        &env,
        "v1.zip",
        &[
            ("UserData/cfg.json", b"DEFAUT V1"),
            ("version.dll", b"DLL V1"),
            ("ancien.dll", b"SEULEMENT V1"),
        ],
    );
    let p1 = profile("1.0", URL_V1, sha1, &preserve);
    run(&env, &p1, &downloader(&env, &[(URL_V1, &zip1)]));
    assert_eq!(fs::read(&cfg).unwrap(), b"DEFAUT V1");
    fs::write(&cfg, b"REGLAGES DU JOUEUR").unwrap();

    // v2 apporte sa propre config par défaut : elle ne doit pas écraser celle du joueur.
    let (zip2, sha2) = make_zip(
        &env,
        "v2.zip",
        &[
            ("UserData/cfg.json", b"DEFAUT V2"),
            ("version.dll", b"DLL V2"),
            ("nouveau.dll", b"SEULEMENT V2"),
        ],
    );
    let p2 = profile("2.0", URL_V2, sha2, &preserve);
    assert_eq!(
        status(&env.paths, &p2).unwrap(),
        InstallStatus::UpdateAvailable
    );
    run(&env, &p2, &downloader(&env, &[(URL_V2, &zip2)]));

    assert_eq!(fs::read(&cfg).unwrap(), b"REGLAGES DU JOUEUR");
    assert_eq!(
        fs::read(env.game_dir.join("version.dll")).unwrap(),
        b"DLL V2"
    );
    assert!(!env.game_dir.join("ancien.dll").exists());
    assert_eq!(
        fs::read(env.game_dir.join("nouveau.dll")).unwrap(),
        b"SEULEMENT V2"
    );
    assert_eq!(status(&env.paths, &p2).unwrap(), InstallStatus::Installed);

    // Désinstaller : tout revient comme avant, sauf la config du joueur, qui reste.
    let report = uninstall(&env.paths, GAME, false).unwrap();
    assert!(report.conflicts.is_empty());
    assert_eq!(fs::read(&cfg).unwrap(), b"REGLAGES DU JOUEUR");
    let mut after = snapshot(&env.game_dir);
    after.remove(&PathBuf::from("UserData").join("cfg.json"));
    after.remove(&PathBuf::from("UserData"));
    assert_eq!(after, before);
    // La DLL d'origine remplacée par v1 a bien été rendue (et non celle de v1).
    assert_eq!(
        fs::read(env.game_dir.join("version.dll")).unwrap(),
        b"DLL ORIGINALE"
    );
    assert_no_backups(&env);
}

#[test]
fn installation_interrompue_se_nettoie() {
    let env = env();
    fill_game_dir(&env);
    let before = snapshot(&env.game_dir);

    let (zip, sha) = make_zip(
        &env,
        "v1.zip",
        &[
            ("version.dll", b"DLL DU MOD"),
            ("Mods/Rando/rando.dll", b"RANDO"),
        ],
    );
    let p = profile("1.0", URL_V1, sha, &[]);
    run(&env, &p, &downloader(&env, &[(URL_V1, &zip)]));

    // On simule un plantage pendant l'écriture (étape 5) : le reçu est encore
    // `InProgress` et ne connaît pas les hash des fichiers écrits.
    let mut record = InstallRecord::load(&env.paths, GAME).unwrap().unwrap();
    record.state = RecordState::InProgress;
    for step in &mut record.steps {
        for file in &mut step.files {
            file.sha256 = None;
        }
    }
    record.save(&env.paths).unwrap();
    // Et un des fichiers n'a même pas eu le temps d'être écrit.
    fs::remove_file(env.game_dir.join("Mods").join("Rando").join("rando.dll")).unwrap();
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::InProgress);

    let report = uninstall(&env.paths, GAME, false).unwrap();
    assert!(report.conflicts.is_empty());
    assert_eq!(snapshot(&env.game_dir), before);
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::NotInstalled);
    assert_no_backups(&env);
}
