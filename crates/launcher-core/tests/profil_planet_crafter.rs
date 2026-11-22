//! Profil The Planet Crafter (ADR 0048, étape 3) installé sur un faux dossier de jeu.
//!
//! Test ignoré : aucun réseau, mais il lui faut les deux vrais zips du profil, déjà
//! téléchargés dans le dossier désigné par `RAFTUP_TPC_ZIPS`
//! (`BepInEx_win_x64_5.4.23.5.zip` et `TPCA.zip`). Le sha256 du profil est contrôlé comme
//! dans une vraie installation.
//!
//! ```text
//! $env:RAFTUP_TPC_ZIPS = "<dossier des deux zips>"
//! cargo test -p launcher-core --test profil_planet_crafter -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::PathBuf;

use launcher_core::download::{DirFetcher, Downloader};
use launcher_core::install::{install, status, InstallOptions, InstallStatus};
use launcher_core::paths::Paths;
use launcher_core::profile::Profile;
use launcher_core::uninstall::uninstall;

const PROFILE: &str = include_str!("../../../profiles/the-planet-crafter.toml");

#[test]
#[ignore = "demande les vrais zips : RAFTUP_TPC_ZIPS"]
fn installer_reinstaller_desinstaller_sur_un_faux_jeu() {
    let zips = PathBuf::from(std::env::var("RAFTUP_TPC_ZIPS").expect("RAFTUP_TPC_ZIPS manquante"));
    let profile = Profile::parse("the-planet-crafter.toml", PROFILE).unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::under(&tmp.path().join("app"));
    let game_dir = tmp.path().join("The Planet Crafter");
    fs::create_dir_all(&game_dir).unwrap();
    // Le faux jeu : l'exécutable attendu, et un fichier du joueur qui ne doit pas bouger.
    fs::write(game_dir.join("Planet Crafter.exe"), b"faux").unwrap();
    fs::write(game_dir.join("UnityPlayer.dll"), b"faux").unwrap();

    // Chaque URL du profil est servie par le zip local du même nom (dernier segment).
    let mut fetcher = DirFetcher::new();
    for step in &profile.steps {
        let url = step.source.resolve(&step.version);
        let name = url.rsplit('/').next().unwrap().to_string();
        fetcher = fetcher.with_file(&url, zips.join(name));
    }
    let downloader = Downloader::new(paths.downloads_dir(), Box::new(fetcher));
    let options = InstallOptions {
        force: false,
        ap_version: "0.6.8".to_string(),
    };

    assert_eq!(
        status(&paths, &profile).unwrap(),
        InstallStatus::NotInstalled
    );
    let report = install(
        &paths,
        &profile,
        &game_dir,
        &downloader,
        &options,
        &mut |_, _| {},
    )
    .unwrap();
    println!(
        "installé : {} étapes, {} fichiers",
        report.steps_installed, report.files_written
    );
    assert_eq!(report.steps_installed, 2);
    assert!(game_dir.join("winhttp.dll").is_file());
    assert!(game_dir
        .join("BepInEx")
        .join("core")
        .join("BepInEx.dll")
        .is_file());
    let tpca = game_dir.join("BepInEx").join("plugins").join("TPCA");
    for f in [
        "TPCA.dll",
        "Archipelago.MultiClient.Net.dll",
        "Newtonsoft.Json.dll",
    ] {
        assert!(tpca.join(f).is_file(), "{f} absent");
    }
    assert_eq!(status(&paths, &profile).unwrap(), InstallStatus::Installed);

    // Réinstallation : rien à faire.
    let again = install(
        &paths,
        &profile,
        &game_dir,
        &downloader,
        &options,
        &mut |_, _| {},
    )
    .unwrap();
    assert!(again.already_up_to_date);

    // Désinstallation : le jeu redevient tel qu'il était.
    uninstall(&paths, "the-planet-crafter", false).unwrap();
    assert!(!game_dir.join("winhttp.dll").exists());
    assert!(!tpca.exists());
    assert_eq!(
        fs::read(game_dir.join("Planet Crafter.exe")).unwrap(),
        b"faux"
    );
    assert_eq!(
        status(&paths, &profile).unwrap(),
        InstallStatus::NotInstalled
    );
}
