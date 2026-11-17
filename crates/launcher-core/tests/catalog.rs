//! Tests du catalogue.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use launcher_core::catalog::{Catalog, ALLOWED_HOSTS, EMBEDDED};

const VALID: &str = include_str!("data/profile/jeu-test.toml");

/// Un second profil, fabriqué à partir du premier.
fn second() -> (String, String) {
    let text = VALID
        .replace("id = \"jeu-test\"", "id = \"autre-jeu\"")
        .replace("ap_game = \"Test Game\"", "ap_game = \"Another Game\"");
    ("autre-jeu.toml".to_string(), text)
}

#[test]
fn catalogue_embarque_se_charge() {
    let catalog = Catalog::embedded().unwrap();
    assert_eq!(catalog.profiles.len(), EMBEDDED.len());
}

/// Le dossier réel `profiles/` et la liste `include_str!` doivent dire la même chose.
#[test]
fn liste_embarquee_egale_au_dossier_profiles() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../profiles");
    let mut on_disk = BTreeSet::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // Le contenu embarqué doit aussi être celui du disque.
            let text = fs::read_to_string(&path).unwrap();
            let embedded = EMBEDDED.iter().find(|(n, _)| *n == name);
            assert_eq!(
                embedded.map(|(_, t)| t.replace("\r\n", "\n")),
                Some(text.replace("\r\n", "\n")),
                "profil {name} absent ou différent dans EMBEDDED (catalog.rs)"
            );
            on_disk.insert(name);
        }
    }
    let listed: BTreeSet<String> = EMBEDDED.iter().map(|(n, _)| (*n).to_string()).collect();
    assert_eq!(listed, on_disk, "EMBEDDED et profiles/*.toml diffèrent");
}

#[test]
fn recherche_et_ordre_deterministe() {
    let (name2, text2) = second();
    // Donnés dans le désordre : le résultat est trié par id.
    let catalog = Catalog::from_texts(&[(&name2, &text2), ("jeu-test.toml", VALID)]).unwrap();
    let ids: Vec<&str> = catalog.games().map(|p| p.game.id.as_str()).collect();
    assert_eq!(ids, vec!["autre-jeu", "jeu-test"]);
    assert_eq!(catalog.get("jeu-test").unwrap().game.ap_game, "Test Game");
    assert!(catalog.get("absent").is_none());
    assert_eq!(
        catalog.by_ap_game("Another Game").unwrap().game.id,
        "autre-jeu"
    );
    assert!(catalog.by_ap_game("Nope").is_none());
}

#[test]
fn refuse_deux_profils_de_meme_id() {
    let err =
        Catalog::from_texts(&[("jeu-test.toml", VALID), ("jeu-test.toml", VALID)]).unwrap_err();
    assert!(err.to_string().contains("déjà présent"), "{err}");
}

#[test]
fn un_profil_faux_arrete_le_catalogue() {
    assert!(Catalog::from_texts(&[("jeu-test.toml", "format_version = 1")]).is_err());
}

#[test]
fn liste_blanche_des_domaines() {
    assert_eq!(
        ALLOWED_HOSTS,
        &[
            "github.com",
            "objects.githubusercontent.com",
            "thunderstore.io",
            "gcdn.thunderstore.io"
        ]
    );
}
