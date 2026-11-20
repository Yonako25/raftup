//! Registre signé lu par le launcher (ADR 0032) : publication locale, puis téléchargement
//! (faux client `DirFetcher`, aucun réseau), vérification et ajout au catalogue.
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use launcher_core::download::DirFetcher;
use launcher_core::profile::StepKind;
use launcher_core::registry::{
    add_registry_profiles, apply_registry, apworld_path, fetch_package, fetch_registry,
    fetch_verified_index, install_apworld, verify_apworld, ApworldInstall, RegistryLoadError,
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
    publish(&source_path, &cles.join(SECRET_KEY_FILE), &sortie, &[]).unwrap();
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
fn cle_du_launcher_refuse_un_index_signe_par_une_autre_cle() {
    // La vraie clé embarquée se lit (sinon ce serait `BadPublicKey`) et refuse un index
    // signé par une clé de test : seule la clé secrète de la mainteneuse fait foi.
    let p = publier();
    let resultat = fetch_verified_index(
        &fetcher(&p),
        &format!("{BASE}/index.json"),
        REGISTRY_PUBLIC_KEY,
    );
    assert!(matches!(
        resultat,
        Err(RegistryLoadError::Registry(
            RegistryError::SignatureMismatch
        ))
    ));
}

#[test]
fn registre_lu_pour_la_bibliotheque() {
    let p = publier();
    let mut catalogue = Catalog::embedded().unwrap();
    let lu = fetch_registry(
        &fetcher(&p),
        &format!("{BASE}/index.json"),
        &p.cle,
        &catalogue,
    )
    .unwrap();
    // Slime Rancher est embarqué : son profil du registre n'est même pas téléchargé.
    assert_eq!(
        lu.files.keys().collect::<Vec<_>>(),
        ["autre-jeu", "troisieme"]
    );
    assert!(lu.failed.is_empty());

    let rapport = apply_registry(&mut catalogue, &lu);
    assert_eq!(rapport.added, ["autre-jeu", "troisieme"]);
    assert_eq!(rapport.kept_embedded, ["slime-rancher"]);
    assert!(rapport.missing.is_empty() && rapport.rejected.is_empty());
    assert!(catalogue.get("troisieme").is_some());
}

#[test]
fn profil_introuvable_chez_l_auteur_n_empeche_pas_le_reste() {
    let p = publier();
    // Le fichier de `troisieme` a disparu : seul ce jeu manque, avec sa raison.
    fs::remove_file(p.dist.join("troisieme.toml")).unwrap();
    let mut catalogue = Catalog::embedded().unwrap();
    let lu = fetch_registry(
        &fetcher(&p),
        &format!("{BASE}/index.json"),
        &p.cle,
        &catalogue,
    )
    .unwrap();
    assert_eq!(lu.failed.len(), 1);
    let rapport = apply_registry(&mut catalogue, &lu);
    assert_eq!(rapport.added, ["autre-jeu"]);
    assert!(rapport.missing.is_empty());
    assert_eq!(rapport.rejected.len(), 1);
    assert_eq!(rapport.rejected[0].0, "troisieme");
}

#[test]
fn registre_injoignable_ou_faux_rend_une_erreur() {
    let p = publier();
    let catalogue = Catalog::embedded().unwrap();
    // Pas de réseau : le faux client ne connaît aucune adresse.
    let rien = DirFetcher::new();
    let resultat = fetch_registry(&rien, &format!("{BASE}/index.json"), &p.cle, &catalogue);
    assert!(matches!(resultat, Err(RegistryLoadError::Download(_))));
    // Mauvaise clé : erreur de signature, le catalogue embarqué n'est pas touché.
    let resultat = fetch_registry(
        &fetcher(&p),
        &format!("{BASE}/index.json"),
        REGISTRY_PUBLIC_KEY,
        &catalogue,
    );
    assert!(resultat.is_err());
}

#[test]
fn apworld_du_registre_installee_puis_gardee() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    let mondes = tempfile::tempdir().unwrap();
    let dossier = mondes.path().join("0.6.8");

    let pose = install_apworld(&fetcher(&p), &index, "monde", &dossier).unwrap();
    let chemin = apworld_path(&dossier, "monde");
    assert_eq!(pose, ApworldInstall::Installed(chemin.clone()));
    assert_eq!(fs::read(&chemin).unwrap(), b"faux zip");
    // Pas de fichier temporaire laissé derrière.
    assert_eq!(fs::read_dir(&dossier).unwrap().count(), 1);

    // Deuxième fois : déjà là, rien n'est téléchargé (le faux client compte les accès).
    let f = fetcher(&p);
    let acces = f.open_counter();
    let encore = install_apworld(&f, &index, "monde", &dossier).unwrap();
    assert_eq!(encore, ApworldInstall::AlreadyThere(chemin));
    assert_eq!(acces.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn apworld_differente_jamais_ecrasee() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    let mondes = tempfile::tempdir().unwrap();
    let chemin = apworld_path(mondes.path(), "monde");
    fs::write(&chemin, b"autre version").unwrap();
    assert!(matches!(
        install_apworld(&fetcher(&p), &index, "monde", mondes.path()),
        Err(RegistryLoadError::OtherApworldPresent { .. })
    ));
    assert_eq!(fs::read(&chemin).unwrap(), b"autre version");
    assert!(matches!(
        install_apworld(&fetcher(&p), &index, "absente", mondes.path()),
        Err(RegistryLoadError::UnknownApworld(_))
    ));
}

#[test]
fn apworld_modifiee_chez_l_auteur_pas_posee() {
    let p = publier();
    let index = fetch_verified_index(&fetcher(&p), &format!("{BASE}/index.json"), &p.cle).unwrap();
    fs::write(p.dist.join("monde.apworld"), b"faux zap").unwrap();
    let mondes = tempfile::tempdir().unwrap();
    assert!(install_apworld(&fetcher(&p), &index, "monde", mondes.path()).is_err());
    assert!(!apworld_path(mondes.path(), "monde").exists());
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

/// Le vrai fichier source du registre (`registre/registre.toml`) reste cohérent avec le
/// catalogue embarqué : adresses acceptées, et une apworld déjà fournie par un profil a la
/// même adresse et la même version que lui.
#[test]
fn source_du_registre_coherente_avec_le_catalogue() {
    let racine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../registre");
    let texte = fs::read_to_string(racine.join("registre.toml")).unwrap();
    let source: toml::Table = toml::from_str(&texte).unwrap();
    assert_eq!(source["format_version"].as_integer(), Some(1));
    let paquets = source["package"].as_array().unwrap();
    assert!(!paquets.is_empty(), "au moins une entrée réelle");

    let catalogue = Catalog::embedded().unwrap();
    let mut verifies = 0;
    for paquet in paquets {
        let nom = paquet["name"].as_str().unwrap();
        let url = paquet["url"].as_str().unwrap();
        let version = paquet["version"].as_str().unwrap();
        registry::index::check_url(url).unwrap();
        if paquet["kind"].as_str() != Some("apworld") {
            continue;
        }
        // Le fichier posé s'appelle `<nom>.apworld` : l'adresse chez l'auteur aussi.
        assert!(url.ends_with(&format!("/{nom}.apworld")), "{url}");
        let etapes = catalogue.games().flat_map(|p| p.steps.iter());
        for etape in etapes.filter(|e| e.kind == StepKind::Apworld) {
            if etape.source.resolve(&etape.version) != url {
                continue;
            }
            assert_eq!(etape.version, version, "version de {nom}");
            // Si la copie locale est là (poste de la mainteneuse), son sha256 est celui
            // du profil : l'index publié ne contredira pas le catalogue embarqué.
            let copie = racine.join(paquet["file"].as_str().unwrap());
            if let Ok(octets) = fs::read(&copie) {
                assert_eq!(registry::sha256_hex(&octets), etape.sha256.to_string());
            }
            verifies += 1;
        }
    }
    assert!(
        verifies >= 1,
        "l'apworld de Slime Rancher est comparée au profil"
    );
}
