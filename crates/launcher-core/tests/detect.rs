//! Tests de la détection des jeux, avec de faux dossiers Steam et Epic.

// Dans les tests (y compris les fonctions d'aide), un `unwrap` qui échoue est un échec de test voulu.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use launcher_core::detect::{
    detect_games, validate_manual_dir, DetectError, DetectRule, DetectionRoots, GameState, Store,
};
use tempfile::TempDir;

fn rule(id: &str, appid: Option<u32>, epic: Option<&str>, exe: Option<&str>) -> DetectRule {
    DetectRule {
        game_id: id.to_string(),
        steam_appid: appid,
        epic_app_name: epic.map(str::to_string),
        expected_exe: exe.map(str::to_string),
    }
}

/// Chemin écrit comme dans un fichier VDF : les antislashs sont doublés.
fn vdf_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "\\\\")
}

/// Écrit `appmanifest_<appid>.acf` dans `library` et crée le dossier du jeu si `create_dir`.
fn write_manifest(library: &Path, appid: u32, installdir: &str, flags: u32, create_dir: bool) {
    let steamapps = library.join("steamapps");
    fs::create_dir_all(&steamapps).unwrap();
    fs::write(
        steamapps.join(format!("appmanifest_{appid}.acf")),
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"Un jeu\"\n\
             \t\"StateFlags\"\t\t\"{flags}\"\n\t\"installdir\"\t\t\"{installdir}\"\n\
             \t\"buildid\"\t\t\"1234\"\n}}\n"
        ),
    )
    .unwrap();
    if create_dir {
        fs::create_dir_all(steamapps.join("common").join(installdir)).unwrap();
    }
}

fn write_libraryfolders(steam: &Path, libs: &[&Path]) {
    let mut text = String::from("\"libraryfolders\"\n{\n");
    for (i, lib) in libs.iter().enumerate() {
        text.push_str(&format!(
            "\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"label\"\t\t\"\"\n\t}}\n",
            vdf_path(lib)
        ));
    }
    text.push_str("}\n");
    fs::create_dir_all(steam.join("steamapps")).unwrap();
    fs::write(steam.join("steamapps").join("libraryfolders.vdf"), text).unwrap();
}

fn steam_roots(steam: &Path) -> DetectionRoots {
    DetectionRoots {
        steam_dir: Some(steam.to_path_buf()),
        epic_manifests_dir: None,
    }
}

#[test]
fn steam_plusieurs_bibliotheques() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    let lib2 = tmp.path().join("Bibliothèque Jeux é");
    write_libraryfolders(&steam, &[&steam, &lib2]);
    write_manifest(&steam, 111, "JeuA", 4, true);
    write_manifest(&lib2, 222, "Jeu B é", 4, true);

    let rules = [
        rule("b", Some(222), None, None),
        rule("a", Some(111), None, None),
    ];
    let games = detect_games(&steam_roots(&steam), &rules);
    assert_eq!(games.len(), 2);
    // Trié par game_id.
    assert_eq!(games[0].game_id, "a");
    assert_eq!(games[1].game_id, "b");
    assert_eq!(games[1].store, Store::Steam);
    assert_eq!(
        games[1].install_dir,
        lib2.join("steamapps").join("common").join("Jeu B é")
    );
    assert_eq!(games[0].build_id.as_deref(), Some("1234"));
    assert_eq!(games[0].state, GameState::Ready);
}

#[test]
fn steam_bibliotheque_absente_ignoree() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    let absente = tmp.path().join("DisqueDebranche");
    write_libraryfolders(&steam, &[&steam, &absente]);
    write_manifest(&steam, 111, "JeuA", 4, true);
    let games = detect_games(&steam_roots(&steam), &[rule("a", Some(111), None, None)]);
    assert_eq!(games.len(), 1);
}

#[test]
fn steam_sans_libraryfolders_utilise_le_dossier_steam() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    write_manifest(&steam, 111, "JeuA", 4, true);
    let games = detect_games(&steam_roots(&steam), &[rule("a", Some(111), None, None)]);
    assert_eq!(games.len(), 1);
}

#[test]
fn steam_dossier_inexistant_ne_plante_pas() {
    let tmp = TempDir::new().unwrap();
    let games = detect_games(
        &steam_roots(&tmp.path().join("nulle part")),
        &[rule("a", Some(1), None, None)],
    );
    assert!(games.is_empty());
}

#[test]
fn steam_manifeste_casse_ignore_sans_gener_les_autres() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    write_manifest(&steam, 111, "JeuA", 4, true);
    write_manifest(&steam, 222, "JeuB", 4, true);
    fs::write(
        steam.join("steamapps").join("appmanifest_111.acf"),
        "\"AppState\" { pas fini",
    )
    .unwrap();
    let rules = [
        rule("a", Some(111), None, None),
        rule("b", Some(222), None, None),
    ];
    let games = detect_games(&steam_roots(&steam), &rules);
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].game_id, "b");
}

#[test]
fn steam_etats_selon_stateflags() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    write_manifest(&steam, 1, "Pret", 4, true);
    write_manifest(&steam, 2, "MiseAJour", 6, true); // installé + mise à jour en attente
    write_manifest(&steam, 3, "Installation", 1026, true); // 2 + 1024, bit 4 absent
    write_manifest(&steam, 4, "Lance", 68, true); // 4 + 64
    write_manifest(&steam, 5, "PasInstalle", 0, true);
    let rules: Vec<DetectRule> = (1..=5)
        .map(|i| rule(&format!("g{i}"), Some(i), None, None))
        .collect();
    let games = detect_games(&steam_roots(&steam), &rules);
    let state_of = |id: &str| games.iter().find(|g| g.game_id == id).map(|g| g.state);
    assert_eq!(state_of("g1"), Some(GameState::Ready));
    assert_eq!(state_of("g2"), Some(GameState::Updating));
    assert_eq!(state_of("g3"), Some(GameState::Updating));
    assert_eq!(state_of("g4"), Some(GameState::Running));
    assert_eq!(state_of("g5"), None);
}

#[test]
fn steam_manifeste_obsolete_sans_dossier_ignore() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    write_manifest(&steam, 111, "JeuA", 4, false);
    let games = detect_games(&steam_roots(&steam), &[rule("a", Some(111), None, None)]);
    assert!(games.is_empty());
}

#[test]
fn steam_installdir_qui_sort_du_dossier_refuse() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    fs::create_dir_all(tmp.path().join("Steam").join("steamapps").join("cible")).unwrap();
    write_manifest(&steam, 111, "..\\\\cible", 4, false);
    let games = detect_games(&steam_roots(&steam), &[rule("a", Some(111), None, None)]);
    assert!(games.is_empty());
}

fn write_epic(dir: &Path, file: &str, app_name: &str, location: &Path) {
    fs::create_dir_all(dir).unwrap();
    let json = format!(
        "{{\"FormatVersion\":0,\"AppName\":\"{app_name}\",\"InstallLocation\":{},\
         \"AppVersionString\":\"1.2.3\",\"ChampInconnu\":[1,2]}}",
        serde_json::to_string(&location.to_string_lossy()).unwrap()
    );
    fs::write(dir.join(file), json).unwrap();
}

#[test]
fn epic_manifestes_et_manifeste_casse() {
    let tmp = TempDir::new().unwrap();
    let manifests = tmp.path().join("Manifests");
    let game_dir = tmp.path().join("Epic Games").join("Mon Jeu é");
    fs::create_dir_all(&game_dir).unwrap();
    write_epic(&manifests, "A.item", "AppA", &game_dir);
    fs::write(manifests.join("B.item"), "{ pas du json").unwrap();
    fs::write(manifests.join("C.item"), "{\"AppName\": 12}").unwrap();
    fs::write(manifests.join("note.txt"), "ignoré").unwrap();

    let roots = DetectionRoots {
        steam_dir: None,
        epic_manifests_dir: Some(manifests),
    };
    let games = detect_games(&roots, &[rule("jeu", None, Some("AppA"), None)]);
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].store, Store::Epic);
    assert_eq!(games[0].install_dir, game_dir);
    assert_eq!(games[0].build_id.as_deref(), Some("1.2.3"));
    assert_eq!(games[0].state, GameState::Ready);
}

#[test]
fn epic_dossier_du_jeu_absent_ignore() {
    let tmp = TempDir::new().unwrap();
    let manifests = tmp.path().join("Manifests");
    write_epic(&manifests, "A.item", "AppA", &tmp.path().join("supprime"));
    let roots = DetectionRoots {
        steam_dir: None,
        epic_manifests_dir: Some(manifests),
    };
    assert!(detect_games(&roots, &[rule("jeu", None, Some("AppA"), None)]).is_empty());
}

#[test]
fn ordre_deterministe_steam_puis_epic() {
    let tmp = TempDir::new().unwrap();
    let steam = tmp.path().join("Steam");
    write_manifest(&steam, 1, "J", 4, true);
    let manifests = tmp.path().join("Manifests");
    let epic_dir = tmp.path().join("EpicJ");
    fs::create_dir_all(&epic_dir).unwrap();
    write_epic(&manifests, "A.item", "AppJ", &epic_dir);
    write_epic(&manifests, "B.item", "AppZ", &epic_dir);
    let roots = DetectionRoots {
        steam_dir: Some(steam),
        epic_manifests_dir: Some(manifests),
    };
    let rules = [
        rule("z", None, Some("AppZ"), None),
        rule("j", Some(1), Some("AppJ"), None),
    ];
    let games = detect_games(&roots, &rules);
    let keys: Vec<(&str, Store)> = games
        .iter()
        .map(|g| (g.game_id.as_str(), g.store))
        .collect();
    assert_eq!(
        keys,
        vec![("j", Store::Steam), ("j", Store::Epic), ("z", Store::Epic)]
    );
    assert_eq!(games, detect_games(&roots, &rules));
}

#[test]
fn aucun_lanceur_donne_une_liste_vide() {
    assert!(detect_games(
        &DetectionRoots::default(),
        &[rule("a", Some(1), None, None)]
    )
    .is_empty());
}

#[test]
fn dossier_manuel_valide() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("Mon dossier é");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("Jeu.exe"), b"MZ").unwrap();
    let game = validate_manual_dir(&rule("a", None, None, Some("Jeu.exe")), &dir).unwrap();
    assert_eq!(game.store, Store::Manual);
    assert_eq!(game.install_dir, dir);
    assert_eq!(game.state, GameState::Ready);
}

#[test]
fn dossier_manuel_invalide() {
    let tmp = TempDir::new().unwrap();
    let r = rule("a", None, None, Some("Jeu.exe"));
    assert!(matches!(
        validate_manual_dir(&r, &tmp.path().join("absent")).unwrap_err(),
        DetectError::NotADirectory(_)
    ));
    assert!(matches!(
        validate_manual_dir(&r, tmp.path()).unwrap_err(),
        DetectError::ExeMissing { .. }
    ));
    assert!(matches!(
        validate_manual_dir(&rule("a", None, None, None), tmp.path()).unwrap_err(),
        DetectError::NoExpectedExe { .. }
    ));
    assert!(matches!(
        validate_manual_dir(&rule("a", None, None, Some("..\\x.exe")), tmp.path()).unwrap_err(),
        DetectError::UnsafeExe(_)
    ));
}
