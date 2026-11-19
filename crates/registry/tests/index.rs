//! Format de l'index, validation et signature (ADR 0032).
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::type_complexity)]

use std::io::Cursor;

use registry::{
    check_package, sha256_hex, verify_index, Index, Package, PackageKind, RegistryError,
    FORMAT_VERSION,
};

const URL: &str = "https://github.com/auteur/depot/releases/download/v1/monde.apworld";

fn package(kind: PackageKind, name: &str, data: &[u8]) -> Package {
    Package {
        kind,
        name: name.to_string(),
        version: "0.3.2.1".to_string(),
        game: Some("Slime Rancher".to_string()),
        url: URL.to_string(),
        sha256: sha256_hex(data),
        size: data.len() as u64,
        license: "MIT".to_string(),
        authors: vec!["Auteur".to_string()],
    }
}

fn sample() -> Index {
    let mut index = Index::new("raftup");
    index
        .packages
        .push(package(PackageKind::Profile, "slime-rancher", b"profil"));
    index
        .packages
        .push(package(PackageKind::Apworld, "zeta", b"z"));
    index
        .packages
        .push(package(PackageKind::Apworld, "alpha", b"a"));
    index
}

/// Paire de clés de test ; rend (clé publique, signature des octets).
fn sign(data: &[u8]) -> (String, String) {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    let signature = minisign::sign(
        Some(&pair.pk),
        &pair.sk,
        Cursor::new(data),
        Some("test"),
        Some("test"),
    )
    .unwrap();
    (pair.pk.to_base64(), signature.into_string())
}

#[test]
fn aller_retour_trie_et_deterministe() {
    let bytes = sample().to_bytes().unwrap();
    let relu = Index::parse(&bytes).unwrap();
    let noms: Vec<&str> = relu.packages.iter().map(|p| p.name.as_str()).collect();
    // Apworlds d'abord (ordre de `PackageKind`), puis par nom.
    assert_eq!(noms, ["alpha", "zeta", "slime-rancher"]);
    // Le même contenu, dans un autre ordre, donne les mêmes octets.
    let mut autre = sample();
    autre.packages.reverse();
    assert_eq!(autre.to_bytes().unwrap(), bytes);
    assert_eq!(relu.to_bytes().unwrap(), bytes);
}

#[test]
fn format_inconnu_refuse_avant_le_reste() {
    // Un champ inconnu ne doit pas masquer le vrai problème : le format.
    let texte = br#"{"format_version": 2, "nouveau": true}"#;
    assert!(matches!(
        Index::parse(texte),
        Err(RegistryError::UnsupportedFormat {
            found: 2,
            supported: FORMAT_VERSION
        })
    ));
}

#[test]
fn champ_inconnu_refuse() {
    let mut json: serde_json::Value =
        serde_json::from_slice(&sample().to_bytes().unwrap()).unwrap();
    json["packages"][0]["script"] = serde_json::Value::from("x");
    let bytes = serde_json::to_vec(&json).unwrap();
    assert!(matches!(Index::parse(&bytes), Err(RegistryError::Parse(_))));
}

#[test]
fn sorte_rom_refusee() {
    let mut json: serde_json::Value =
        serde_json::from_slice(&sample().to_bytes().unwrap()).unwrap();
    json["packages"][0]["kind"] = serde_json::Value::from("rom");
    let bytes = serde_json::to_vec(&json).unwrap();
    assert!(matches!(Index::parse(&bytes), Err(RegistryError::Parse(_))));
}

#[test]
fn doublon_refuse() {
    let mut index = sample();
    let mut copie = index.packages[1].clone();
    copie.version = "2.0".to_string();
    index.packages.push(copie);
    assert!(matches!(
        index.validate(),
        Err(RegistryError::InvalidIndex(_))
    ));
    // Même nom mais autre sorte : permis.
    let mut index = sample();
    index
        .packages
        .push(package(PackageKind::Profile, "alpha", b"p"));
    index.validate().unwrap();
}

#[test]
fn champs_invalides_refuses() {
    let cas: Vec<(&str, Box<dyn Fn(&mut Package)>)> = vec![
        ("http", Box::new(|p| p.url = URL.replace("https", "http"))),
        (
            "domaine",
            Box::new(|p| p.url = "https://exemple.com/a.apworld".into()),
        ),
        (
            "identifiants",
            Box::new(|p| p.url = "https://github.com@exemple.com/a".into()),
        ),
        (
            "port",
            Box::new(|p| p.url = "https://github.com:8443/a".into()),
        ),
        (
            "sans chemin",
            Box::new(|p| p.url = "https://github.com/".into()),
        ),
        ("nom majuscule", Box::new(|p| p.name = "Alpha".into())),
        ("nom chemin", Box::new(|p| p.name = "../alpha".into())),
        ("nom vide", Box::new(|p| p.name = String::new())),
        ("version vide", Box::new(|p| p.version = String::new())),
        ("version espace", Box::new(|p| p.version = "1 0".into())),
        ("sha256 court", Box::new(|p| p.sha256 = "abc".into())),
        (
            "sha256 majuscule",
            Box::new(|p| p.sha256 = p.sha256.to_uppercase()),
        ),
        ("taille nulle", Box::new(|p| p.size = 0)),
        ("taille géante", Box::new(|p| p.size = u64::MAX)),
        ("licence vide", Box::new(|p| p.license = " ".into())),
        ("sans auteur", Box::new(|p| p.authors.clear())),
        (
            "auteur contrôle",
            Box::new(|p| p.authors = vec!["a\u{1b}[31m".into()]),
        ),
        ("jeu vide", Box::new(|p| p.game = Some(String::new()))),
    ];
    for (nom, modifier) in cas {
        let mut p = package(PackageKind::Apworld, "alpha", b"a");
        modifier(&mut p);
        assert!(
            matches!(p.validate(), Err(RegistryError::InvalidPackage { .. })),
            "cas « {nom} » accepté"
        );
    }
    // Le paquet de base, lui, est valide (jeu facultatif compris).
    let mut p = package(PackageKind::Apworld, "alpha", b"a");
    p.validate().unwrap();
    p.game = None;
    p.validate().unwrap();
}

#[test]
fn signature_correcte_acceptee() {
    let bytes = sample().to_bytes().unwrap();
    let (cle, signature) = sign(&bytes);
    let index = verify_index(&bytes, &signature, &cle).unwrap();
    assert_eq!(index.packages.len(), 3);
}

#[test]
fn index_modifie_refuse() {
    let bytes = sample().to_bytes().unwrap();
    let (cle, signature) = sign(&bytes);
    let mut modifie = bytes.clone();
    // Un seul octet changé (dans un sha256) suffit.
    let pos = modifie.iter().position(|b| *b == b'"').unwrap() + 1;
    modifie[pos] ^= 1;
    assert!(matches!(
        verify_index(&modifie, &signature, &cle),
        Err(RegistryError::SignatureMismatch)
    ));
}

#[test]
fn autre_cle_refusee() {
    let bytes = sample().to_bytes().unwrap();
    let (_, signature) = sign(&bytes);
    let (autre_cle, _) = sign(b"autre");
    assert!(matches!(
        verify_index(&bytes, &signature, &autre_cle),
        Err(RegistryError::SignatureMismatch)
    ));
}

#[test]
fn signature_verifiee_avant_lecture() {
    // Un JSON faux mais bien signé passe la signature puis échoue à la lecture ;
    // un JSON faux mal signé échoue à la signature, sans être lu.
    let faux = b"pas du json";
    let (cle, signature) = sign(faux);
    assert!(matches!(
        verify_index(faux, &signature, &cle),
        Err(RegistryError::Parse(_))
    ));
    let (_, autre_signature) = sign(b"x");
    assert!(matches!(
        verify_index(faux, &autre_signature, &cle),
        Err(RegistryError::SignatureMismatch)
    ));
}

#[test]
fn cle_absente_ou_fausse() {
    let bytes = sample().to_bytes().unwrap();
    let (_, signature) = sign(&bytes);
    assert!(matches!(
        verify_index(&bytes, &signature, "  "),
        Err(RegistryError::PublicKeyNotConfigured)
    ));
    assert!(matches!(
        verify_index(&bytes, &signature, "RWpasunecle"),
        Err(RegistryError::BadPublicKey(_))
    ));
    let (cle, _) = sign(&bytes);
    assert!(matches!(
        verify_index(&bytes, "n'importe quoi", &cle),
        Err(RegistryError::BadSignature(_))
    ));
}

#[test]
fn index_trop_gros_refuse() {
    let gros = vec![b' '; registry::INDEX_MAX_BYTES as usize + 1];
    let (cle, signature) = sign(&gros);
    assert!(matches!(
        verify_index(&gros, &signature, &cle),
        Err(RegistryError::TooLarge { .. })
    ));
}

#[test]
fn controle_d_un_paquet() {
    let p = package(PackageKind::Apworld, "alpha", b"contenu");
    check_package(&p, b"contenu").unwrap();
    assert!(matches!(
        check_package(&p, b"contenX"),
        Err(RegistryError::HashMismatch { .. })
    ));
    assert!(matches!(
        check_package(&p, b"court"),
        Err(RegistryError::SizeMismatch { .. })
    ));
}

#[test]
fn sha256_connu() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
