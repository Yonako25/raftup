//! Tests de la configuration utilisateur.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use launcher_core::config::{load, ConfigError, GameSettings, UserConfig};

fn sample() -> UserConfig {
    let mut config = UserConfig {
        player_name: Some("Ioka".into()),
        last_port: Some(38281),
        ..UserConfig::default()
    };
    config.games.insert(
        "slime-rancher".into(),
        GameSettings {
            game_dir: Some("C:/Jeux/Été et Slimes".into()),
        },
    );
    config
}

#[test]
fn fichier_absent_donne_les_defauts() {
    let tmp = tempfile::tempdir().unwrap();
    let loaded = load(&tmp.path().join("config.toml")).unwrap();
    assert_eq!(loaded.config, UserConfig::default());
    assert!(loaded.set_aside.is_none());
}

#[test]
fn aller_retour_sans_tmp_restant() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sous").join("config.toml");
    sample().save(&path).unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.config, sample());
    assert!(loaded.set_aside.is_none());
    let names: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["config.toml"]);
    // Écrire deux fois de suite (le fichier existe déjà) fonctionne et reste stable.
    let first = fs::read_to_string(&path).unwrap();
    sample().save(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), first);
}

#[test]
fn ancien_fichier_sans_les_nouveaux_champs() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    fs::write(&path, "format_version = 1\nplayer_name = \"Ioka\"\n").unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.config.player_name.as_deref(), Some("Ioka"));
    assert!(loaded.config.games.is_empty());
    assert_eq!(loaded.config.last_port, None);
    // Même sans format_version, et avec un champ inconnu d'un futur launcher.
    fs::write(&path, "champ_futur = 3\n").unwrap();
    assert_eq!(load(&path).unwrap().config, UserConfig::default());
}

#[test]
fn fichier_illisible_est_mis_de_cote() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    fs::write(&path, "ceci = pas [du toml").unwrap();
    let loaded = load(&path).unwrap();
    assert_eq!(loaded.config, UserConfig::default());
    let aside = loaded.set_aside.unwrap();
    assert_eq!(aside, tmp.path().join("config.toml.illisible"));
    assert_eq!(fs::read_to_string(&aside).unwrap(), "ceci = pas [du toml");
    assert!(!path.exists());

    // Un deuxième fichier illisible ne l'écrase pas : il est numéroté.
    fs::write(&path, "last_port = \"pas un nombre\"").unwrap();
    let second = load(&path).unwrap().set_aside.unwrap();
    assert_eq!(second, tmp.path().join("config.toml.illisible.1"));
    assert_eq!(fs::read_to_string(&aside).unwrap(), "ceci = pas [du toml");
}

#[test]
fn version_future_refusee_sans_toucher_au_fichier() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    fs::write(&path, "format_version = 99\n").unwrap();
    let err = load(&path).unwrap_err();
    assert!(
        matches!(err, ConfigError::UnsupportedVersion { found: 99, .. }),
        "{err:?}"
    );
    assert!(path.exists());
}

#[test]
fn aucun_mot_de_passe_ecrit() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    sample().save(&path).unwrap();
    let text = fs::read_to_string(&path).unwrap().to_lowercase();
    assert!(!text.contains("password"), "{text}");
    assert!(!text.contains("passwd"), "{text}");
    assert!(!text.contains("mot_de_passe"), "{text}");
    // Même si un fichier écrit à la main en contient un, il est ignoré puis jamais réécrit.
    fs::write(&path, "password = \"secret-123\"\n").unwrap();
    let loaded = load(&path).unwrap();
    loaded.config.save(&path).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("secret-123"));
}
