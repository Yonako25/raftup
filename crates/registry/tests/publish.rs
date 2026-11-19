//! Publication locale : source TOML → index.json + index.json.minisig (ADR 0032).
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::fs;
use std::path::Path;

use registry::publish::{
    build_index, keygen, publish, sign_index, INDEX_FILE, PUBLIC_KEY_FILE, SECRET_KEY_FILE,
    SIGNATURE_FILE,
};
use registry::{check_package, verify_index, PackageKind, RegistryError};

const PROFIL: &str = "format_version = 1\n[game]\nid = \"demo\"\n";

/// Prépare un dossier avec deux paquets locaux et un fichier source.
fn chantier(dir: &Path, profil: &str) -> std::path::PathBuf {
    fs::create_dir_all(dir.join("dist")).unwrap();
    fs::write(dir.join("dist/monde.apworld"), b"faux zip").unwrap();
    fs::write(dir.join("dist/demo.toml"), profil).unwrap();
    let source = dir.join("registre.toml");
    fs::write(
        &source,
        r#"
format_version = 1
registry = "raftup"

[[package]]
kind = "profile"
name = "demo"
version = "1"
file = "dist/demo.toml"
url = "https://github.com/auteur/depot/releases/download/v1/demo.toml"
license = "MIT"
authors = ["Auteur"]

[[package]]
kind = "apworld"
name = "monde"
version = "0.3.2.1"
game = "Monde"
file = "dist/monde.apworld"
url = "https://github.com/auteur/depot/releases/download/v1/monde.apworld"
license = "unlicensed"
authors = ["Auteur", "Autre"]
"#,
    )
    .unwrap();
    source
}

#[test]
fn publication_complete_puis_verification() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    let cles = tmp.path().join("cles");
    let publique = keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");

    let rapport = publish(&source, &cles.join(SECRET_KEY_FILE), &sortie).unwrap();
    assert_eq!(rapport.packages, 2);
    assert_eq!(rapport.public_key, publique);
    assert_eq!(rapport.index_path, sortie.join(INDEX_FILE));

    // Le launcher relit exactement ces deux fichiers.
    let bytes = fs::read(sortie.join(INDEX_FILE)).unwrap();
    let signature = fs::read_to_string(sortie.join(SIGNATURE_FILE)).unwrap();
    let index = verify_index(&bytes, &signature, &publique).unwrap();
    let monde = index.get(PackageKind::Apworld, "monde").unwrap();
    check_package(monde, b"faux zip").unwrap();
    assert_eq!(monde.authors, ["Auteur", "Autre"]);
    // Trié : apworld avant profil, quel que soit l'ordre du fichier source.
    assert_eq!(index.packages[0].name, "monde");
    // Aucun fichier temporaire laissé.
    let mut noms: Vec<String> = fs::read_dir(&sortie)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    noms.sort();
    assert_eq!(noms, [INDEX_FILE, SIGNATURE_FILE]);
}

#[test]
fn republier_donne_le_meme_index() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    let cles = tmp.path().join("cles");
    keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");
    publish(&source, &cles.join(SECRET_KEY_FILE), &sortie).unwrap();
    let premier = fs::read(sortie.join(INDEX_FILE)).unwrap();
    publish(&source, &cles.join(SECRET_KEY_FILE), &sortie).unwrap();
    assert_eq!(fs::read(sortie.join(INDEX_FILE)).unwrap(), premier);
}

#[test]
fn keygen_refuse_d_ecraser() {
    let tmp = tempfile::tempdir().unwrap();
    keygen(tmp.path()).unwrap();
    let avant = fs::read(tmp.path().join(SECRET_KEY_FILE)).unwrap();
    assert!(matches!(keygen(tmp.path()), Err(RegistryError::Signing(_))));
    assert_eq!(fs::read(tmp.path().join(SECRET_KEY_FILE)).unwrap(), avant);
    assert!(tmp.path().join(PUBLIC_KEY_FILE).exists());
}

#[test]
fn profil_dont_l_id_differe_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), "format_version = 1\n[game]\nid = \"autre\"\n");
    assert!(matches!(
        build_index(&source),
        Err(RegistryError::InvalidPackage { .. })
    ));
}

#[test]
fn fichier_local_absent_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    fs::remove_file(tmp.path().join("dist/monde.apworld")).unwrap();
    assert!(matches!(
        build_index(&source),
        Err(RegistryError::Io { .. })
    ));
}

#[test]
fn adresse_hors_liste_blanche_refusee_a_la_publication() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    let texte = fs::read_to_string(&source).unwrap().replace(
        "https://github.com/auteur/depot/releases/download/v1/monde.apworld",
        "https://exemple.com/monde.apworld",
    );
    fs::write(&source, texte).unwrap();
    assert!(matches!(
        build_index(&source),
        Err(RegistryError::InvalidPackage { .. })
    ));
}

#[test]
fn signature_avec_cle_illisible_refusee() {
    assert!(matches!(
        sign_index(b"{}", "pas une clé"),
        Err(RegistryError::Signing(_))
    ));
}
