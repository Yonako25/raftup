//! Tests de l'installation. Aucun réseau : `DirFetcher` sert des fichiers locaux.
//! Les archives zip sont fabriquées ici, jamais commitées.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use launcher_core::download::{DirFetcher, Downloader};
use launcher_core::hash::Sha256;
use launcher_core::install::{
    install, status, InstallError, InstallOptions, InstallReport, InstallStatus,
};
use launcher_core::paths::Paths;
use launcher_core::profile::Profile;
use launcher_core::record::{InstallRecord, RecordState};
use launcher_core::uninstall::uninstall;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const GAME: &str = "jeu";

// Le verrou « un seul install par jeu » est global au processus et tous les tests
// utilisent le même `game_id` : on les fait passer l'un après l'autre.
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
    let game_dir = tmp.path().join("Mon Jeu é");
    fs::create_dir_all(&game_dir).unwrap();
    Env {
        _serial: serial,
        tmp,
        paths,
        game_dir,
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
    let dir = env.tmp.path().join("zips");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, &bytes).unwrap();
    (path, Sha256::of_bytes(&bytes))
}

/// Une étape du profil : (id, version, url, sha256, ligne `install`).
struct StepSpec {
    id: &'static str,
    version: &'static str,
    url: String,
    sha: Sha256,
    install: &'static str,
}

fn profile(steps: &[StepSpec], preserve: &[&str], status: &str) -> Profile {
    let mut text = format!(
        "format_version = 1\n[game]\nid = \"{GAME}\"\nname = \"Jeu\"\nap_game = \"Jeu AP\"\n\
         status = \"{status}\"\n[credits]\nauthors = [\"x\"]\nlicense = \"MIT\"\n"
    );
    for s in steps {
        text.push_str(&format!(
            "[[steps]]\nid = \"{}\"\nkind = \"mod\"\nname = \"M\"\nversion = \"{}\"\n\
             source = {{ url = \"{}\" }}\nsha256 = \"{}\"\ninstall = {}\n",
            s.id, s.version, s.url, s.sha, s.install
        ));
    }
    let list: Vec<String> = preserve.iter().map(|p| format!("\"{p}\"")).collect();
    text.push_str(&format!(
        "[preserve]\npaths = [{}]\n[launch]\nexe = \"{{game_dir}}/Jeu.exe\"\n\
         connect = {{ method = \"manual\", instructions = \"x\" }}\n",
        list.join(", ")
    ));
    Profile::parse(&format!("{GAME}.toml"), &text).unwrap()
}

fn extract_step(version: &'static str, url: &str, sha: Sha256) -> StepSpec {
    StepSpec {
        id: "loader",
        version,
        url: url.to_string(),
        sha,
        install: "{ extract = \"zip\", into = \"{game_dir}\" }",
    }
}

fn opts(force: bool) -> InstallOptions {
    InstallOptions {
        force,
        ap_version: "0.6.1".to_string(),
    }
}

fn downloader(env: &Env, files: &[(&str, &Path)]) -> Downloader {
    let mut fetcher = DirFetcher::new();
    for (url, path) in files {
        fetcher = fetcher.with_file(url, *path);
    }
    Downloader::new(env.paths.downloads_dir(), Box::new(fetcher))
}

fn run(
    env: &Env,
    profile: &Profile,
    dl: &Downloader,
    force: bool,
) -> Result<InstallReport, InstallError> {
    install(
        &env.paths,
        profile,
        &env.game_dir,
        dl,
        &opts(force),
        &mut |_, _| {},
    )
}

const URL: &str = "https://github.com/x/y/mod.zip";

#[test]
fn installation_simple_recu_et_statut() {
    let env = env();
    let (zip, sha) = make_zip(
        &env,
        "a.zip",
        &[("Mods/a.dll", b"AAA"), ("loader.dll", b"LL")],
    );
    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::NotInstalled);

    let report = run(&env, &p, &dl, false).unwrap();
    assert_eq!(report.steps_installed, 1);
    assert_eq!(report.files_written, 2);
    assert_eq!(
        fs::read(env.game_dir.join("Mods").join("a.dll")).unwrap(),
        b"AAA"
    );

    let record = InstallRecord::load(&env.paths, GAME).unwrap().unwrap();
    assert_eq!(record.state, RecordState::Complete);
    assert_eq!(record.steps[0].root, env.game_dir);
    assert_eq!(record.steps[0].dirs, vec![PathBuf::from("Mods")]);
    assert!(record.steps[0].files.iter().all(|f| f.sha256.is_some()));
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::Installed);
}

#[test]
fn deja_a_jour_ne_touche_a_rien() {
    let env = env();
    let (zip, sha) = make_zip(&env, "a.zip", &[("a.dll", b"A")]);
    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);
    run(&env, &p, &dl, false).unwrap();
    let report = run(&env, &p, &dl, false).unwrap();
    assert!(report.already_up_to_date);
    assert_eq!(report.steps_installed, 0);
}

#[test]
fn aller_retour_installer_puis_desinstaller_avec_sauvegarde() {
    let env = env();
    fs::write(env.game_dir.join("loader.dll"), b"ORIGINAL").unwrap();
    let (zip, sha) = make_zip(&env, "a.zip", &[("loader.dll", b"MOD"), ("x.dll", b"X")]);
    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);

    let report = run(&env, &p, &dl, false).unwrap();
    assert_eq!(report.backed_up, vec![env.game_dir.join("loader.dll")]);
    assert_eq!(fs::read(env.game_dir.join("loader.dll")).unwrap(), b"MOD");

    let un = uninstall(&env.paths, GAME, false).unwrap();
    assert_eq!(un.restored, 1);
    assert_eq!(un.removed, 1);
    assert_eq!(
        fs::read(env.game_dir.join("loader.dll")).unwrap(),
        b"ORIGINAL"
    );
    assert!(!env.game_dir.join("x.dll").exists());
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::NotInstalled);
}

#[test]
fn copy_to_installe_un_fichier() {
    let env = env();
    let (zip, sha) = make_zip(&env, "w.zip", &[("seul.txt", b"x")]);
    let spec = StepSpec {
        id: "monde",
        version: "2.0",
        url: URL.to_string(),
        sha,
        install: "{ copy_to = \"{worlds_dir}/jeu.apworld\" }",
    };
    let p = profile(&[spec], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);
    run(&env, &p, &dl, false).unwrap();
    let target = env.paths.worlds_dir("0.6.1").join("jeu.apworld");
    assert_eq!(Sha256::of_bytes(&fs::read(&target).unwrap()), sha);
    let record = InstallRecord::load(&env.paths, GAME).unwrap().unwrap();
    assert_eq!(record.steps[0].root, env.paths.worlds_dir("0.6.1"));
    uninstall(&env.paths, GAME, false).unwrap();
    assert!(!target.exists());
}

#[test]
fn preserve_garde_le_fichier_du_joueur_et_la_mise_a_jour_aussi() {
    let env = env();
    let cfg = env.game_dir.join("UserData").join("cfg.json");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, b"REGLAGES DU JOUEUR").unwrap();

    let (zip1, sha1) = make_zip(
        &env,
        "v1.zip",
        &[("UserData/cfg.json", b"DEFAUT"), ("old.dll", b"v1")],
    );
    let p1 = profile(
        &[extract_step("1.0", URL, sha1)],
        &["{game_dir}/UserData/cfg.json"],
        "available",
    );
    let dl1 = downloader(&env, &[(URL, &zip1)]);
    let report = run(&env, &p1, &dl1, false).unwrap();
    assert_eq!(report.kept_preserved, vec![cfg.clone()]);
    assert_eq!(fs::read(&cfg).unwrap(), b"REGLAGES DU JOUEUR");
    let record = InstallRecord::load(&env.paths, GAME).unwrap().unwrap();
    assert!(record.steps[0]
        .files
        .iter()
        .all(|f| f.path != Path::new("UserData").join("cfg.json")));

    // Mise à jour : nouvelle version, le joueur a encore modifié sa config.
    fs::write(&cfg, b"REGLAGES V2").unwrap();
    let (zip2, sha2) = make_zip(
        &env,
        "v2.zip",
        &[("UserData/cfg.json", b"DEFAUT 2"), ("new.dll", b"v2")],
    );
    let url2 = "https://github.com/x/y/mod2.zip";
    let p2 = profile(
        &[extract_step("2.0", url2, sha2)],
        &["{game_dir}/UserData/cfg.json"],
        "available",
    );
    assert_eq!(
        status(&env.paths, &p2).unwrap(),
        InstallStatus::UpdateAvailable
    );
    let dl2 = downloader(&env, &[(url2, &zip2)]);
    run(&env, &p2, &dl2, false).unwrap();
    assert_eq!(fs::read(&cfg).unwrap(), b"REGLAGES V2");
    assert!(!env.game_dir.join("old.dll").exists());
    assert_eq!(fs::read(env.game_dir.join("new.dll")).unwrap(), b"v2");
    assert_eq!(status(&env.paths, &p2).unwrap(), InstallStatus::Installed);
}

#[test]
fn mise_a_jour_bloquee_par_un_fichier_modifie_puis_force() {
    let env = env();
    let (zip1, sha1) = make_zip(&env, "v1.zip", &[("a.dll", b"v1")]);
    let p1 = profile(&[extract_step("1.0", URL, sha1)], &[], "available");
    run(&env, &p1, &downloader(&env, &[(URL, &zip1)]), false).unwrap();
    fs::write(env.game_dir.join("a.dll"), b"BIDOUILLE").unwrap();

    let (zip2, sha2) = make_zip(&env, "v2.zip", &[("a.dll", b"v2")]);
    let url2 = "https://github.com/x/y/mod2.zip";
    let p2 = profile(&[extract_step("2.0", url2, sha2)], &[], "available");
    let dl2 = downloader(&env, &[(url2, &zip2)]);

    let err = run(&env, &p2, &dl2, false).unwrap_err();
    assert!(matches!(err, InstallError::Conflicts(ref c) if c.len() == 1));
    assert_eq!(fs::read(env.game_dir.join("a.dll")).unwrap(), b"BIDOUILLE");

    run(&env, &p2, &dl2, true).unwrap();
    assert_eq!(fs::read(env.game_dir.join("a.dll")).unwrap(), b"v2");
}

#[test]
fn mauvais_hash_ne_touche_pas_au_jeu() {
    let env = env();
    let (zip, _) = make_zip(&env, "a.zip", &[("a.dll", b"A")]);
    let faux = Sha256::of_bytes(b"autre chose");
    let p = profile(&[extract_step("1.0", URL, faux)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);
    let err = run(&env, &p, &dl, false).unwrap_err();
    assert!(matches!(err, InstallError::Download(_)));
    assert!(!env.game_dir.join("a.dll").exists());
    assert!(InstallRecord::load(&env.paths, GAME).unwrap().is_none());
}

#[test]
fn installation_interrompue_demande_force() {
    let env = env();
    let (zip, sha) = make_zip(&env, "a.zip", &[("a.dll", b"A")]);
    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);

    // Un reçu InProgress laissé par un plantage.
    let mut record = InstallRecord::new(GAME, env.game_dir.clone());
    record.state = RecordState::InProgress;
    record.save(&env.paths).unwrap();
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::InProgress);

    let err = run(&env, &p, &dl, false).unwrap_err();
    assert!(matches!(err, InstallError::Interrupted { .. }));
    run(&env, &p, &dl, true).unwrap();
    assert_eq!(status(&env.paths, &p).unwrap(), InstallStatus::Installed);
}

#[test]
fn profil_casse_et_dossier_invalide_refuses() {
    let env = env();
    let (zip, sha) = make_zip(&env, "a.zip", &[("a.dll", b"A")]);
    let dl = downloader(&env, &[(URL, &zip)]);

    let broken = profile(&[extract_step("1.0", URL, sha)], &[], "broken");
    assert!(matches!(
        run(&env, &broken, &dl, false).unwrap_err(),
        InstallError::GameBroken { .. }
    ));

    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let err = install(
        &env.paths,
        &p,
        Path::new("relatif"),
        &dl,
        &opts(false),
        &mut |_, _| {},
    )
    .unwrap_err();
    assert!(matches!(err, InstallError::InvalidArgument(_)));
}

#[test]
fn une_seule_installation_a_la_fois_par_jeu() {
    let env = env();
    let (zip, sha) = make_zip(&env, "a.zip", &[("a.dll", b"A")]);
    let p = profile(&[extract_step("1.0", URL, sha)], &[], "available");
    let dl = downloader(&env, &[(URL, &zip)]);

    // Pendant le téléchargement (rappel de progression), on relance : refusé.
    let mut nested: Option<Result<InstallReport, InstallError>> = None;
    install(
        &env.paths,
        &p,
        &env.game_dir,
        &dl,
        &opts(false),
        &mut |_, _| {
            if nested.is_none() {
                nested = Some(run(&env, &p, &dl, false));
            }
        },
    )
    .unwrap();
    assert!(matches!(
        nested.unwrap().unwrap_err(),
        InstallError::Busy { .. }
    ));
    // Le verrou est libéré : une autre installation est possible.
    assert!(run(&env, &p, &dl, false).unwrap().already_up_to_date);
}

/// Archive dans une archive (ADR 0050), comme l'installateur de SMAPI : seule l'archive
/// intérieure est extraite, puis la désinstallation remet le jeu dans son état d'origine.
#[test]
fn archive_interieure_installee_puis_retiree() {
    let env = env();
    // L'archive intérieure, puis l'extérieure qui la contient sous un nom en `.dat`.
    let (inner, _) = make_zip(
        &env,
        "install.dat",
        &[
            ("StardewModdingAPI.exe", b"SMAPI"),
            ("smapi-internal/a.dll", b"A"),
        ],
    );
    let inner_bytes = fs::read(&inner).unwrap();
    let (outer, sha) = make_zip(
        &env,
        "installer.zip",
        &[
            ("SMAPI installer/install on Windows.bat", b"bat"),
            ("SMAPI installer/internal/windows/install.dat", &inner_bytes),
        ],
    );
    let step = StepSpec {
        id: "smapi",
        version: "4.5.2",
        url: URL.to_string(),
        sha,
        install: "{ extract = \"zip\", inner = \"SMAPI installer/internal/windows/install.dat\", into = \"{game_dir}\" }",
    };
    let p = profile(&[step], &[], "untested");
    let dl = downloader(&env, &[(URL, &outer)]);
    let report = run(&env, &p, &dl, false).unwrap();
    assert_eq!(report.files_written, 2);
    assert_eq!(
        fs::read(env.game_dir.join("StardewModdingAPI.exe")).unwrap(),
        b"SMAPI"
    );
    assert!(env.game_dir.join("smapi-internal").join("a.dll").exists());
    // Rien de l'installateur lui-même n'arrive dans le jeu.
    assert!(!env.game_dir.join("SMAPI installer").exists());

    uninstall(&env.paths, GAME, false).unwrap();
    assert!(!env.game_dir.join("StardewModdingAPI.exe").exists());
    assert!(!env.game_dir.join("smapi-internal").exists());
}

/// Une archive intérieure absente fait échouer l'installation sans toucher au jeu.
#[test]
fn archive_interieure_absente_refusee() {
    let env = env();
    let (outer, sha) = make_zip(&env, "installer.zip", &[("autre.txt", b"x")]);
    let step = StepSpec {
        id: "smapi",
        version: "1",
        url: URL.to_string(),
        sha,
        install: "{ extract = \"zip\", inner = \"internal/install.dat\", into = \"{game_dir}\" }",
    };
    let p = profile(&[step], &[], "untested");
    let dl = downloader(&env, &[(URL, &outer)]);
    assert!(run(&env, &p, &dl, false).is_err());
    assert_eq!(fs::read_dir(&env.game_dir).unwrap().count(), 0);
}

/// Images du créateur (ADR 0051) : rangées à l'installation, hash vérifié ; une image
/// qui ne vient pas ne fait pas échouer l'installation.
#[test]
fn images_du_createur_installees_avec_le_jeu() {
    let env = env();
    let (zip, sha) = make_zip(&env, "mod.zip", &[("BepInEx/core.dll", b"dll")]);
    let jpeg: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
    let cover = env.tmp.path().join("cover.jpg");
    fs::write(&cover, jpeg).unwrap();
    let cover_url = "https://raw.githubusercontent.com/x/y/main/cover.jpg";
    // L'image large est déclarée mais absente du serveur : le jeu s'installe quand même.
    let text = format!(
        "format_version = 1\n[game]\nid = \"{GAME}\"\nname = \"Jeu\"\nap_game = \"Jeu AP\"\n\
         status = \"available\"\n[credits]\nauthors = [\"x\"]\nlicense = \"MIT\"\n\
         [[steps]]\nid = \"loader\"\nkind = \"mod\"\nname = \"M\"\nversion = \"1\"\n\
         source = {{ url = \"{URL}\" }}\nsha256 = \"{sha}\"\n\
         install = {{ extract = \"zip\", into = \"{{game_dir}}\" }}\n\
         [launch]\nexe = \"{{game_dir}}/Jeu.exe\"\n\
         connect = {{ method = \"manual\", instructions = \"x\" }}\n\
         [art]\ncover = {{ url = \"{cover_url}\", sha256 = \"{}\" }}\n",
        Sha256::of_bytes(jpeg)
    );
    let profile = Profile::parse(&format!("{GAME}.toml"), &text).unwrap();
    let dl = downloader(&env, &[(URL, &zip), (cover_url, &cover)]);
    let report = run(&env, &profile, &dl, false).unwrap();
    assert_eq!(report.art_installed, 1);
    let installed = env.paths.art_dir(GAME).join("cover.jpg");
    assert_eq!(fs::read(installed).unwrap(), jpeg);

    // Une image de mauvaise empreinte n'est pas rangée, et l'installation réussit.
    let env2_text = text.replace(&Sha256::of_bytes(jpeg).to_string(), &sha.to_string());
    let bad = Profile::parse(&format!("{GAME}.toml"), &env2_text).unwrap();
    fs::remove_dir_all(env.paths.art_dir(GAME)).unwrap();
    let report = run(&env, &bad, &dl, false).unwrap();
    assert!(report.already_up_to_date);
    assert_eq!(report.art_installed, 0);
    assert!(!env.paths.art_dir(GAME).join("cover.jpg").exists());
}

/// Une image hors des domaines permis est refusée à la lecture du profil.
#[test]
fn image_hors_liste_blanche_refusee() {
    let text = format!(
        "format_version = 1\nsteps = []\n[game]\nid = \"{GAME}\"\nname = \"Jeu\"\nap_game = \"Jeu AP\"\n\
         status = \"available\"\n[credits]\nauthors = [\"x\"]\nlicense = \"MIT\"\n\
         [launch]\nexe = \"{{game_dir}}/Jeu.exe\"\n\
         connect = {{ method = \"manual\", instructions = \"x\" }}\n\
         [art]\nhero = {{ url = \"https://exemple.com/hero.jpg\", sha256 = \"{}\" }}\n",
        Sha256::of_bytes(b"x")
    );
    let error = Profile::parse(&format!("{GAME}.toml"), &text).unwrap_err();
    assert!(error.to_string().contains("art.hero"), "{error}");
}
