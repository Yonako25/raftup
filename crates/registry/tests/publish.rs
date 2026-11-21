//! Publication locale : source TOML → index.json + index.json.minisig (ADR 0032).
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::fs;
use std::path::Path;

use registry::publish::{
    build_index, keygen, publish, sign_index, TrustClaim, INDEX_FILE, PUBLIC_KEY_FILE,
    SECRET_KEY_FILE, SIGNATURE_FILE,
};
use registry::{
    check_package, sha256_hex, verify_index, PackageKind, RegistryError, Trust, TrustLevel,
};

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

    let rapport = publish(&source, &cles.join(SECRET_KEY_FILE), &sortie, &[]).unwrap();
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
    publish(&source, &cles.join(SECRET_KEY_FILE), &sortie, &[]).unwrap();
    let premier = fs::read(sortie.join(INDEX_FILE)).unwrap();
    publish(&source, &cles.join(SECRET_KEY_FILE), &sortie, &[]).unwrap();
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

// --- Niveaux de confiance repris d'un rapport de CI (ADR 0045) ---

fn claim(name: &str, version: &str, data: &[u8], level: TrustLevel) -> TrustClaim {
    TrustClaim {
        kind: PackageKind::Apworld,
        name: name.to_string(),
        version: version.to_string(),
        sha256: sha256_hex(data),
        trust: Trust {
            level,
            detail: Some("test".to_string()),
        },
    }
}

#[test]
fn niveau_du_rapport_ecrit_dans_l_index_signe() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    let cles = tmp.path().join("cles");
    let publique = keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");
    let claims = [claim(
        "monde",
        "0.3.2.1",
        b"faux zip",
        TrustLevel::Generates,
    )];
    let rapport = publish(&source, &cles.join(SECRET_KEY_FILE), &sortie, &claims).unwrap();
    assert_eq!(
        rapport.levels,
        [
            ("apworld monde".to_string(), TrustLevel::Generates),
            ("profile demo".to_string(), TrustLevel::Unverified),
        ]
    );
    // Le profil n'a pas de rapport : un seul avertissement, pour lui.
    assert_eq!(rapport.warnings.len(), 1);
    assert!(rapport.warnings[0].contains("profile demo"));

    let bytes = fs::read(sortie.join(INDEX_FILE)).unwrap();
    let signature = fs::read_to_string(sortie.join(SIGNATURE_FILE)).unwrap();
    let index = verify_index(&bytes, &signature, &publique).unwrap();
    let monde = index.get(PackageKind::Apworld, "monde").unwrap();
    assert_eq!(monde.trust.level, TrustLevel::Generates);
    assert_eq!(monde.trust.detail.as_deref(), Some("test"));
}

#[test]
fn rapport_d_un_autre_fichier_ignore() {
    let tmp = tempfile::tempdir().unwrap();
    let source = chantier(tmp.path(), PROFIL);
    let mut index = build_index(&source).unwrap();
    // Même nom, autre contenu, puis autre version : aucun des deux ne fait monter le niveau.
    let faux = [
        claim("monde", "0.3.2.1", b"autre zip", TrustLevel::Fuzzed),
        claim("monde", "9.9", b"faux zip", TrustLevel::Fuzzed),
        claim("absent", "1", b"x", TrustLevel::Fuzzed),
    ];
    let warnings = registry::publish::apply_trust(&mut index, &faux);
    assert!(index
        .packages
        .iter()
        .all(|p| p.trust.level == TrustLevel::Unverified));
    assert!(warnings.iter().any(|w| w.contains("absent")));
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.contains("pas le fichier publié"))
            .count(),
        2
    );
}
