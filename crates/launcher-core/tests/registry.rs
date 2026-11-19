//! Registre signé lu par le launcher (ADR 0032) : publication locale, puis téléchargement
//! (faux client `DirFetcher`, aucun réseau), vérification et ajout au catalogue.
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use launcher_core::download::DirFetcher;
use launcher_core::registry::{
    add_registry_profiles, fetch_package, fetch_verified_index, verify_apworld, RegistryLoadError,
    REGISTRY_PUBLIC_KEY,
};
use launcher_core::Catalog;
use registry::publish::{keygen, publish, INDEX_FILE, SECRET_KEY_FILE, SIGNATURE_FILE};
use registry::{PackageKind, RegistryError};

const BASE: &str = "https://github.com/auteur/registre/releases/download/r1";
const SLIME: &str = include_str!("../../../profiles/slime-rancher.toml");

/// Profil valide d'un autre jeu, tiré de celui de Slime Rancher.
fn autre_profil() -> String {
    SLIME.replace("id = \"slime-rancher\"", "id = \"autre-jeu\"")
}

struct Publie {
    _tmp: tempfile::TempDir,
    dist: PathBuf,
    sortie: PathBuf,
    cle: String,
}

/// Publie un registre de trois profils et une apworld dans un dossier temporaire.
fn publier() -> Publie {
    let tmp = tempfile::tempdir().unwrap();
    let dist = tmp.path().join("dist");
    fs::create_dir_all(&dist).unwrap();
    fs::write(dist.join("slime-rancher.toml"), SLIME).unwrap();
    fs::write(dist.join("autre-jeu.toml"), autre_profil()).unwrap();
    let troisieme = SLIME.replace("id = \"slime-rancher\"", "id = \"troisieme\"");
    fs::write(dist.join("troisieme.toml"), troisieme).unwrap();
    fs::write(dist.join("monde.apworld"), b"faux zip").unwrap();

    let mut source = String::from("format_version = 1\nregistry = \"raftup\"\n");
    for (kind, name, file) in [
        ("profile", "slime-rancher", "slime-rancher.toml"),
        ("profile", "autre-jeu", "autre-jeu.toml"),
        ("profile", "troisieme", "troisieme.toml"),
        ("apworld", "monde", "monde.apworld"),
    ] {
        source.push_str(&format!(
            "\n[[package]]\nkind = \"{kind}\"\nname = \"{name}\"\nversion = \"1.0\"\n\
             file = \"dist/{file}\"\nurl = \"{BASE}/{file}\"\nlicense = \"MIT\"\nauthors = [\"Auteur\"]\n"
        ));
    }
    let source_path = tmp.path().join("registre.toml");
    fs::write(&source_path, source).unwrap();
    let cles = tmp.path().join("cles");
    let cle = keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");
    publish(&source_path, &cles.join(SECRET_KEY_FILE), &sortie).unwrap();
    Publie {
        _tmp: tmp,
        dist,
        sortie,
        cle,
    }
}

fn fetcher(p: &Publie) -> DirFetcher {
    let mut f = DirFetcher::new()
        .with_file(&format!("{BASE}/index.json"), p.sortie.join(INDEX_FILE))
        .with_file(
            &format!("{BASE}/index.json.minisig"),
            p.sortie.join(SIGNATURE_FILE),
        );
    for file in [
        "slime-rancher.toml",
        "autre-jeu.toml",
        "troisieme.toml",
        "monde.apworld",
    ] {
        f = f.with_file(&format!("{BASE}/{file}"), p.dist.join(file));
    }
    f
}

#[test]
fn index_publie_puis_verifie_par_le_launcher() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    assert_eq!(index.packages.len(), 4);

    // Apworld : téléchargée, taille et sha256 contrôlés.
    let monde = index.get(PackageKind::Apworld, "monde").unwrap();
    let data = fetch_package(&fetcher(&p), monde).unwrap();
    assert_eq!(data, b"faux zip");
    verify_apworld(&index, "monde", &data).unwrap();
    assert!(verify_apworld(&index, "monde", b"autre").is_err());
    assert!(verify_apworld(&index, "absente", &data).is_err());
}

#[test]
fn cle_vide_du_launcher_refuse_tout() {
    // Tant que la vraie clé n'est pas générée, le launcher n'accepte aucun index.
    let p = publier();
    let resultat = fetch_verified_index(
        &fetcher(&p),
        &format!("{BASE}/index.json"),
        REGISTRY_PUBLIC_KEY,
    );
    assert!(matches!(
        resultat,
        Err(RegistryLoadError::Registry(
            RegistryError::PublicKeyNotConfigured
        ))
    ));
}

#[test]
fn index_remplace_refuse() {
    let p = publier();
    // Un attaquant remplace l'index sans pouvoir le signer.
    let mut bytes = fs::read(p.sortie.join(INDEX_FILE)).unwrap();
    let texte = String::from_utf8(bytes.clone())
        .unwrap()
        .replace("\"1.0\"", "\"6.6\"");
    bytes = texte.into_bytes();
    fs::write(p.sortie.join(INDEX_FILE), bytes).unwrap();
    let resultat = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle);
    assert!(matches!(
        resultat,
        Err(RegistryLoadError::Registry(
            RegistryError::SignatureMismatch
        ))
    ));
}

#[test]
fn signature_absente_refusee() {
    let p = publier();
    let f = DirFetcher::new().with_file(&format!("{BASE}/index.json"), p.sortie.join(INDEX_FILE));
    let resultat = fetch_verified_index(&f, &format!("{BASE}/index.json"), &p.cle);
    assert!(matches!(resultat, Err(RegistryLoadError::Download(_))));
}

#[test]
fn adresse_hors_liste_blanche_refusee() {
    let p = publier();
    let resultat = fetch_verified_index(&fetcher(&p), "https://exemple.com/index.json", &p.cle);
    assert!(matches!(resultat, Err(RegistryLoadError::BadUrl { .. })));
    let resultat = fetch_verified_index(&fetcher(&p), "http://github.com/a/index.json", &p.cle);
    assert!(matches!(resultat, Err(RegistryLoadError::BadUrl { .. })));
}

#[test]
fn paquet_trop_gros_ou_modifie_refuse() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    let monde = index.get(PackageKind::Apworld, "monde").unwrap();
    // Le fichier chez l'auteur a changé après la publication : même taille, autre contenu.
    fs::write(p.dist.join("monde.apworld"), b"faux zap").unwrap();
    assert!(matches!(
        fetch_package(&fetcher(&p), monde),
        Err(RegistryLoadError::Registry(
            RegistryError::HashMismatch { .. }
        ))
    ));
    // Plus gros que la taille annoncée : la lecture s'arrête et refuse.
    fs::write(p.dist.join("monde.apworld"), vec![0u8; 1000]).unwrap();
    assert!(matches!(
        fetch_package(&fetcher(&p), monde),
        Err(RegistryLoadError::Download(_))
    ));
}

/// Lit les octets d'un profil de `dist` (ce que le launcher aurait téléchargé).
fn octets(dist: &Path, name: &str) -> Vec<u8> {
    fs::read(dist.join(format!("{name}.toml"))).unwrap()
}

#[test]
fn profils_du_registre_ajoutes_sans_remplacer_l_embarque() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    let mut catalogue = Catalog::embedded().unwrap();
    let embarque = catalogue.get("slime-rancher").unwrap().clone();

    let mut files = BTreeMap::new();
    files.insert("autre-jeu".to_string(), octets(&p.dist, "autre-jeu"));
    // Le registre propose un autre texte pour Slime Rancher : il doit être ignoré.
    files.insert("slime-rancher".to_string(), b"n'importe quoi".to_vec());
    // `troisieme` n'est pas téléchargé.

    let rapport = add_registry_profiles(&mut catalogue, &index, &files);
    assert_eq!(rapport.added, ["autre-jeu"]);
    assert_eq!(rapport.kept_embedded, ["slime-rancher"]);
    assert_eq!(rapport.missing, ["troisieme"]);
    assert!(rapport.rejected.is_empty());
    assert_eq!(catalogue.get("slime-rancher"), Some(&embarque));
    assert_eq!(catalogue.get("autre-jeu").unwrap().game.id, "autre-jeu");
}

#[test]
fn profil_modifie_refuse_sans_casser_le_reste() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    let mut catalogue = Catalog::embedded().unwrap();
    let mut files = BTreeMap::new();
    files.insert("autre-jeu".to_string(), octets(&p.dist, "autre-jeu"));
    let mut faux = octets(&p.dist, "troisieme");
    faux[0] ^= 1;
    files.insert("troisieme".to_string(), faux);

    let rapport = add_registry_profiles(&mut catalogue, &index, &files);
    assert_eq!(rapport.added, ["autre-jeu"]);
    assert_eq!(rapport.rejected.len(), 1);
    assert_eq!(rapport.rejected[0].0, "troisieme");
    assert!(rapport.rejected[0].1.contains("sha256"));
    assert!(catalogue.get("troisieme").is_none());
}
