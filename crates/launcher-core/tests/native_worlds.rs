//! Mondes natifs de la Bibliothèque (ADR 0046) : liste, installation par le registre signé
//! (faux client `DirFetcher`, aucun réseau), désinstallation et conversion d'un export.
// Helpers de test : un test qui échoue doit simplement paniquer.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use launcher_core::download::DirFetcher;
use launcher_core::native_worlds::{
    apworld_game, default_player_yaml, install_converted_world, install_native_world,
    list_native_worlds, read_native_package, read_receipt, uninstall_native_world, NativeInstall,
    NativeWorldError, EXPORT_FILE, RECEIPT_FILE, WORLD_FILE,
};
use launcher_core::registry::{fetch_verified_index, Index, RegistryLoadError, TrustLevel};
use registry::publish::{keygen, publish, INDEX_FILE, SECRET_KEY_FILE, SIGNATURE_FILE};
use registry::RegistryError;

const BASE: &str = "https://github.com/auteur/registre/releases/download/r1";
const APQUEST: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");
const MINIMAL: &str = include_str!("../../../spec/monde-natif/exemples/minimal/world.toml");
const APQUEST_EXPORT: &[u8] = include_bytes!("../../generate/tests/data/apquest-export.json");

/// Zip en mémoire : `(chemin dans l'archive, contenu)`.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

struct Publie {
    _tmp: tempfile::TempDir,
    dist: PathBuf,
    index: Index,
}

/// Publie un registre signé de deux mondes natifs : `apquest` (un `world.toml` seul) et
/// `minimal` (un zip avec sa licence), puis relit l'index par sa signature.
fn publier() -> Publie {
    let tmp = tempfile::tempdir().unwrap();
    let dist = tmp.path().join("dist");
    fs::create_dir_all(&dist).unwrap();
    fs::write(dist.join("apquest.toml"), APQUEST).unwrap();
    let archive = zip_of(&[
        ("minimal/world.toml", MINIMAL.as_bytes()),
        ("minimal/LICENSE", b"MIT"),
    ]);
    fs::write(dist.join("minimal.zip"), archive).unwrap();
    let mut source = String::from("format_version = 1\nregistry = \"raftup\"\n");
    for (name, game, file) in [
        ("apquest", "APQuest", "apquest.toml"),
        ("minimal", "Minimal", "minimal.zip"),
    ] {
        source.push_str(&format!(
            "\n[[package]]\nkind = \"native_world\"\nname = \"{name}\"\nversion = \"1.0\"\n\
             game = \"{game}\"\nfile = \"dist/{file}\"\nurl = \"{BASE}/{file}\"\n\
             license = \"MIT\"\nauthors = [\"Auteur\"]\n"
        ));
    }
    let source_path = tmp.path().join("registre.toml");
    fs::write(&source_path, source).unwrap();
    let cles = tmp.path().join("cles");
    let cle = keygen(&cles).unwrap();
    let sortie = tmp.path().join("sortie");
    publish(&source_path, &cles.join(SECRET_KEY_FILE), &sortie, &[]).unwrap();
    let fetcher = DirFetcher::new()
        .with_file(&format!("{BASE}/index.json"), sortie.join(INDEX_FILE))
        .with_file(
            &format!("{BASE}/index.json.minisig"),
            sortie.join(SIGNATURE_FILE),
        );
    let index = fetch_verified_index(&fetcher, &format!("{BASE}/index.json"), &cle).unwrap();
    Publie {
        _tmp: tmp,
        dist,
        index,
    }
}

fn fetcher(p: &Publie) -> DirFetcher {
    DirFetcher::new()
        .with_file(&format!("{BASE}/apquest.toml"), p.dist.join("apquest.toml"))
        .with_file(&format!("{BASE}/minimal.zip"), p.dist.join("minimal.zip"))
}

fn noms(dir: &Path) -> Vec<String> {
    let mut noms: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    noms.sort();
    noms
}

#[test]
fn liste_avec_fiche() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");
    fs::create_dir_all(dir.join("apquest")).unwrap();
    fs::write(dir.join("apquest").join(WORLD_FILE), APQUEST).unwrap();
    fs::write(dir.join("minimal.toml"), MINIMAL).unwrap();
    fs::write(dir.join("casse.toml"), "format_version = 1\n").unwrap();

    let library = list_native_worlds(&dir);
    let cles: Vec<&str> = library.worlds.iter().map(|w| w.key.as_str()).collect();
    assert_eq!(cles, ["apquest", "minimal.toml"]);
    let apquest = library.get("apquest").unwrap();
    assert_eq!(apquest.game, "APQuest");
    assert_eq!(apquest.format_version, 1);
    assert!(apquest.locations > 0 && apquest.items > 0);
    assert_eq!(apquest.version(), None);
    assert_eq!(apquest.receipt, None);
    assert!(apquest.origin_label().contains("à la main"));
    assert_eq!(library.rejected.len(), 1);
    // Dossier absent : liste vide, sans erreur.
    assert!(list_native_worlds(&tmp.path().join("absent"))
        .worlds
        .is_empty());
}

#[test]
fn installation_par_le_registre_puis_deja_la() {
    let p = publier();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");

    let pose = install_native_world(&fetcher(&p), &p.index, "apquest", &dir).unwrap();
    assert_eq!(pose, NativeInstall::Installed(dir.join("apquest")));
    assert_eq!(noms(&dir.join("apquest")), [RECEIPT_FILE, WORLD_FILE]);
    let library = list_native_worlds(&dir);
    let monde = library.get("apquest").unwrap();
    assert_eq!(monde.version(), Some("1.0"));
    let origine = monde.receipt.as_ref().unwrap().registry.as_ref().unwrap();
    assert_eq!(origine.trust, TrustLevel::Unverified);
    assert_eq!(origine.sha256, p.index.packages[0].sha256);
    assert!(monde.origin_label().starts_with("Registre signé"));
    // Aucun dossier de préparation laissé à côté.
    assert_eq!(noms(tmp.path()), ["mondes-natifs"]);

    // Deuxième fois : rien n'est téléchargé (un client vide ne sait rien servir).
    let encore = install_native_world(&DirFetcher::new(), &p.index, "apquest", &dir).unwrap();
    assert_eq!(encore, NativeInstall::AlreadyThere(dir.join("apquest")));

    // Le zip, avec sa licence à côté du monde.
    install_native_world(&fetcher(&p), &p.index, "minimal", &dir).unwrap();
    assert_eq!(
        noms(&dir.join("minimal")),
        ["LICENSE", RECEIPT_FILE, WORLD_FILE]
    );
    assert_eq!(list_native_worlds(&dir).worlds.len(), 2);
}

#[test]
fn empreinte_fausse_rien_n_est_pose() {
    let p = publier();
    fs::write(
        p.dist.join("apquest.toml"),
        APQUEST.replace("APQuest", "Autre"),
    )
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");
    let erreur = install_native_world(&fetcher(&p), &p.index, "apquest", &dir).unwrap_err();
    assert!(
        matches!(
            erreur,
            NativeWorldError::Registry(RegistryLoadError::Registry(
                RegistryError::SizeMismatch { .. } | RegistryError::HashMismatch { .. }
            ))
        ),
        "{erreur}"
    );
    assert!(!dir.exists());
}

#[test]
fn conflits_refuses() {
    let p = publier();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");
    fs::create_dir_all(&dir).unwrap();
    // Un autre fichier porte le nom : jamais écrasé.
    fs::write(dir.join("apquest.toml"), "à moi").unwrap();
    assert!(matches!(
        install_native_world(&fetcher(&p), &p.index, "apquest", &dir),
        Err(NativeWorldError::OtherPresent { .. })
    ));
    fs::remove_file(dir.join("apquest.toml")).unwrap();
    // Le jeu est déjà fourni par un monde posé à la main sous un autre nom.
    fs::write(dir.join("mon-apquest.toml"), APQUEST).unwrap();
    let erreur = install_native_world(&fetcher(&p), &p.index, "apquest", &dir).unwrap_err();
    assert!(
        matches!(&erreur, NativeWorldError::GameAlreadyProvided { key, .. } if key == "mon-apquest.toml"),
        "{erreur}"
    );
    // Paquet absent de l'index.
    assert!(matches!(
        install_native_world(&fetcher(&p), &p.index, "absent", &dir),
        Err(NativeWorldError::UnknownPackage(_))
    ));
}

#[test]
fn paquets_dangereux_ou_faux_refuses() {
    let monde = MINIMAL.as_bytes();
    let cas: Vec<(&str, Vec<u8>)> = vec![
        (
            "remonte",
            zip_of(&[("world.toml", monde), ("../evil.txt", b"x")]),
        ),
        ("profond", zip_of(&[("a/b/world.toml", monde)])),
        ("autre dossier", zip_of(&[("autre/world.toml", monde)])),
        (
            "reçu fourni",
            zip_of(&[("world.toml", monde), (RECEIPT_FILE, b"x")]),
        ),
        ("sans monde", zip_of(&[("LICENSE", b"MIT")])),
        ("caché", zip_of(&[("world.toml", monde), (".git", b"x")])),
        ("pas UTF-8", vec![0xff, 0xfe, 0x00]),
        ("pas un monde", b"format_version = 1\n".to_vec()),
    ];
    for (nom, data) in cas {
        assert!(
            read_native_package("minimal", None, &data).is_err(),
            "{nom} accepté"
        );
    }
    // Le jeu annoncé par le registre doit être celui du monde.
    let erreur = read_native_package("minimal", Some("APQuest"), monde).unwrap_err();
    assert!(erreur.contains("Minimal"), "{erreur}");
    // À la racine du zip ou dans `<nom>/` : les deux vont.
    for chemin in ["world.toml", "minimal/world.toml"] {
        let paquet =
            read_native_package("minimal", Some("Minimal"), &zip_of(&[(chemin, monde)])).unwrap();
        assert_eq!(paquet.files[0].0, WORLD_FILE);
    }
}

#[test]
fn desinstallation_bornee_aux_fichiers_poses() {
    let p = publier();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");
    install_native_world(&fetcher(&p), &p.index, "minimal", &dir).unwrap();
    install_native_world(&fetcher(&p), &p.index, "apquest", &dir).unwrap();

    // Tout ce que Raftup a posé part, et le dossier avec.
    let rapport = uninstall_native_world(&dir, "minimal").unwrap();
    assert_eq!(rapport.removed, 3);
    assert!(rapport.left.is_empty());
    assert!(!dir.join("minimal").exists());

    // Un fichier ajouté à la main reste, et le dossier avec lui.
    fs::write(dir.join("apquest").join("mes-notes.txt"), "à garder").unwrap();
    let rapport = uninstall_native_world(&dir, "apquest").unwrap();
    assert_eq!(rapport.left, ["mes-notes.txt"]);
    assert_eq!(noms(&dir.join("apquest")), ["mes-notes.txt"]);
    // Le monde n'est plus lu (plus de world.toml).
    assert!(list_native_worlds(&dir).worlds.is_empty());

    // Un fichier `.toml` posé à la main.
    fs::write(dir.join("seul.toml"), MINIMAL).unwrap();
    assert_eq!(
        uninstall_native_world(&dir, "seul.toml").unwrap().removed,
        1
    );
    assert!(!dir.join("seul.toml").exists());

    // Noms refusés ou absents : rien n'est touché.
    for cle in ["..", "../mondes-natifs", "a/b", ""] {
        assert!(matches!(
            uninstall_native_world(&dir, cle),
            Err(NativeWorldError::BadName(_))
        ));
    }
    assert!(matches!(
        uninstall_native_world(&dir, "absent"),
        Err(NativeWorldError::NotInstalled(_))
    ));
}

#[test]
fn conversion_d_un_export_posee_avec_l_export() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("mondes-natifs");
    let converti =
        install_converted_world(&dir, "apquest", "apquest.apworld", "0.6.8", APQUEST_EXPORT)
            .unwrap();
    assert_eq!(converti.game, "APQuest");
    assert_eq!(converti.path, dir.join("apquest"));
    assert_eq!(
        noms(&converti.path),
        [EXPORT_FILE, RECEIPT_FILE, WORLD_FILE]
    );
    // L'export gardé est exactement celui du worker.
    assert_eq!(
        fs::read(converti.path.join(EXPORT_FILE)).unwrap(),
        APQUEST_EXPORT
    );
    let recu = read_receipt(&converti.path).unwrap();
    let origine = recu.conversion.unwrap();
    assert_eq!(origine.apworld, "apquest.apworld");
    assert_eq!(origine.notes, converti.notes.len());
    // La Bibliothèque le voit comme un monde natif.
    let library = list_native_worlds(&dir);
    let monde = library.get("apquest").unwrap();
    assert!(monde.origin_label().contains("apquest.apworld"));
    // Une deuxième conversion ne l'écrase pas.
    assert!(matches!(
        install_converted_world(&dir, "apquest", "apquest.apworld", "0.6.8", APQUEST_EXPORT),
        Err(NativeWorldError::OtherPresent { .. })
    ));
    // Sous une autre clé, le même jeu est refusé.
    assert!(matches!(
        install_converted_world(&dir, "copie", "apquest.apworld", "0.6.8", APQUEST_EXPORT),
        Err(NativeWorldError::GameAlreadyProvided { .. })
    ));
    // Un export illisible ne pose rien.
    assert!(matches!(
        install_converted_world(&dir, "casse", "x.apworld", "0.6.8", b"{}"),
        Err(NativeWorldError::Convert(_))
    ));
    assert!(!dir.join("casse").exists());
}

#[test]
fn yaml_par_defaut_et_jeu_d_un_apworld() {
    assert_eq!(
        default_player_yaml("APQuest"),
        "name: Raftup\ngame: \"APQuest\"\n\"APQuest\": {}\n"
    );
    // Un nom avec guillemets ou deux-points reste une seule chaîne.
    assert!(default_player_yaml("A: \"B\"").contains(r#"game: "A: \"B\"""#));

    let tmp = tempfile::tempdir().unwrap();
    let avec = tmp.path().join("monde.apworld");
    fs::write(
        &avec,
        zip_of(&[
            ("monde/__init__.py", b""),
            ("monde/archipelago.json", br#"{"game": "Mon Jeu"}"#),
        ]),
    )
    .unwrap();
    assert_eq!(apworld_game(&avec).as_deref(), Some("Mon Jeu"));
    let sans = tmp.path().join("sans.apworld");
    fs::write(&sans, zip_of(&[("sans/__init__.py", b"")])).unwrap();
    assert_eq!(apworld_game(&sans), None);
}
