//! Tests de l'extraction zip sécurisée. Les archives sont fabriquées ici, jamais commitées.

// unwrap() est permis dans les tests, y compris dans leurs fonctions d'aide.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use launcher_core::archive::{
    extract_tar_gz, extract_zip, plan_zip, safe_relative_path, ArchiveError, ArchiveLimits,
};
use launcher_core::hash::Sha256;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Fabrique un zip en mémoire : une liste de (nom, contenu).
fn build_zip(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(method);
    for (name, data) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Écrit des octets de zip dans un fichier du dossier temporaire.
fn write_zip(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("test.zip");
    fs::write(&path, bytes).unwrap();
    path
}

fn zip_of(dir: &Path, entries: &[(&str, &[u8])]) -> PathBuf {
    write_zip(dir, &build_zip(entries, CompressionMethod::Deflated))
}

/// Extrait avec les limites par défaut en ignorant les hash.
fn extract(zip: &Path, strip: &str, root: &Path) -> Result<(), ArchiveError> {
    extract_zip(zip, strip, root, &ArchiveLimits::default(), &mut |_, _| {})
}

fn is_unsafe(result: Result<(), ArchiveError>) -> bool {
    matches!(result, Err(ArchiveError::UnsafePath(_)))
}

#[test]
fn chemins_dangereux_refuses_par_safe_relative_path() {
    for bad in [
        "../x",
        "a/../../x",
        "/abs",
        "C:\\x",
        "\\\\serveur\\x",
        "fichier:flux",
        "CON",
        "dir/nul.txt",
        "COM1",
        "a\\..\\..\\x",
    ] {
        assert!(
            matches!(safe_relative_path(bad), Err(ArchiveError::UnsafePath(_))),
            "{bad} aurait dû être refusé"
        );
    }
}

#[test]
fn chemins_sains_acceptes() {
    assert_eq!(
        safe_relative_path("a/b.txt").unwrap(),
        Path::new("a").join("b.txt")
    );
    // « a/../b » reste dans le dossier mais on le refuse quand même : plus simple et plus sûr.
    assert!(safe_relative_path("a/../b").is_err());
    assert!(safe_relative_path("é accentué/fichier.cfg").is_ok());
}

#[test]
fn entrees_dangereuses_refusees_a_l_extraction() {
    for bad in [
        "../x",
        "a/../../x",
        "/abs",
        "C:\\x",
        "\\\\serveur\\x",
        "fichier:flux",
        "CON",
        "NUL",
        "COM1",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let zip = zip_of(tmp.path(), &[("ok.txt", b"ok"), (bad, b"mal")]);
        let root = tmp.path().join("root");
        assert!(is_unsafe(extract(&zip, "", &root)), "{bad} non refusé");
        // Rien n'est écrit si une entrée est mauvaise, même la bonne.
        assert!(!root.join("ok.txt").exists(), "{bad} : écriture partielle");
        assert!(!tmp.path().join("x").exists());
    }
}

#[test]
fn lien_symbolique_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("lien", "/etc/passwd", SimpleFileOptions::default())
        .unwrap();
    let zip = write_zip(tmp.path(), &writer.finish().unwrap().into_inner());
    let root = tmp.path().join("root");

    assert!(matches!(
        extract(&zip, "", &root),
        Err(ArchiveError::Link(_))
    ));
    assert!(!root.join("lien").exists());
    assert!(matches!(
        plan_zip(&zip, "", &ArchiveLimits::default()),
        Err(ArchiveError::Link(_))
    ));
}

#[test]
fn trop_d_entrees_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..11).map(|i| format!("f{i}.txt")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let zip = zip_of(tmp.path(), &entries);
    let limits = ArchiveLimits {
        max_entries: 10,
        ..ArchiveLimits::default()
    };
    let root = tmp.path().join("root");

    assert!(matches!(
        extract_zip(&zip, "", &root, &limits, &mut |_, _| {}),
        Err(ArchiveError::TooManyEntries)
    ));
    assert!(matches!(
        plan_zip(&zip, "", &limits),
        Err(ArchiveError::TooManyEntries)
    ));
    assert!(!root.exists());
}

#[test]
fn bombe_zip_taille_annoncee_refusee() {
    let tmp = tempfile::tempdir().unwrap();
    // 4 Mio de zéros : quelques Kio compressés.
    let big = vec![0u8; 4 * 1024 * 1024];
    let zip = zip_of(tmp.path(), &[("bombe.bin", &big)]);
    let limits = ArchiveLimits {
        max_total_bytes: 1024 * 1024,
        ..ArchiveLimits::default()
    };
    let root = tmp.path().join("root");

    assert!(matches!(
        extract_zip(&zip, "", &root, &limits, &mut |_, _| {}),
        Err(ArchiveError::TooLarge)
    ));
    assert!(!root.join("bombe.bin").exists());
}

/// Patche la taille décompressée annoncée (en-tête local et répertoire central).
fn lie_sur_la_taille(bytes: &mut [u8], fake: u32) {
    // En-tête local : signature à l'offset 0, taille décompressée à +22.
    assert_eq!(&bytes[0..4], b"PK\x03\x04");
    bytes[22..26].copy_from_slice(&fake.to_le_bytes());
    // Répertoire central : taille décompressée à +24.
    let central = bytes.windows(4).rposition(|w| w == b"PK\x01\x02").unwrap();
    bytes[central + 24..central + 28].copy_from_slice(&fake.to_le_bytes());
}

#[test]
fn bombe_zip_taille_reelle_comptee() {
    let tmp = tempfile::tempdir().unwrap();
    // Contenu stocké (non compressé) de 64 Kio dont l'en-tête annonce 10 octets.
    let mut bytes = build_zip(
        &[("menteur.bin", &vec![7u8; 64 * 1024])],
        CompressionMethod::Stored,
    );
    lie_sur_la_taille(&mut bytes, 10);
    let zip = write_zip(tmp.path(), &bytes);
    let limits = ArchiveLimits {
        max_total_bytes: 1024,
        ..ArchiveLimits::default()
    };
    let root = tmp.path().join("root");

    // La taille annoncée (10) passe le contrôle ; seul le comptage réel peut refuser.
    assert!(plan_zip(&zip, "", &limits).is_ok());
    let result = extract_zip(&zip, "", &root, &limits, &mut |_, _| {});
    // La crate `zip` détecte déjà un fichier plus grand que sa taille annoncée (erreur Zip) ;
    // notre comptage est la seconde barrière. Les deux sont acceptables : l'essentiel est
    // l'échec et la borne sur les octets écrits.
    assert!(
        matches!(result, Err(ArchiveError::TooLarge | ArchiveError::Zip(_))),
        "obtenu : {result:?}"
    );
    // Au plus la limite a été écrite, jamais les 64 Kio.
    let written = fs::metadata(root.join("menteur.bin"))
        .map(|m| m.len())
        .unwrap_or(0);
    assert!(written <= 1024, "{written} octets écrits");
}

#[test]
fn strip_prefix_retire_le_dossier_racine() {
    let tmp = tempfile::tempdir().unwrap();
    let zip = zip_of(
        tmp.path(),
        &[
            ("mod-1.0/", b""),
            ("mod-1.0/lib/a.dll", b"AAA"),
            ("mod-1.0/b.txt", b"BBB"),
            ("autre/c.txt", b"CCC"),
        ],
    );
    let root = tmp.path().join("root");

    let planned = plan_zip(&zip, "mod-1.0/", &ArchiveLimits::default()).unwrap();
    assert_eq!(
        planned,
        vec![Path::new("lib").join("a.dll"), PathBuf::from("b.txt")]
    );

    extract(&zip, "mod-1.0", &root).unwrap();
    assert_eq!(fs::read(root.join("lib").join("a.dll")).unwrap(), b"AAA");
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"BBB");
    // Ce qui est hors du préfixe est ignoré.
    assert!(!root.join("autre").exists());
    assert!(!root.join("mod-1.0").exists());
}

#[test]
fn extraction_normale_avec_les_bons_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let zip = zip_of(
        tmp.path(),
        &[
            ("dossier/", b""),
            ("dossier/sous/a.txt", b"contenu A"),
            ("b.txt", b"contenu B"),
            ("vide/", b""),
            ("accent é/c.txt", b"contenu C"),
        ],
    );
    let root = tmp.path().join("root");

    let mut seen: Vec<(PathBuf, Sha256)> = Vec::new();
    extract_zip(
        &zip,
        "",
        &root,
        &ArchiveLimits::default(),
        &mut |path, hash| seen.push((path.to_path_buf(), hash)),
    )
    .unwrap();

    assert_eq!(seen.len(), 3);
    for (path, hash) in &seen {
        let on_disk = fs::read(root.join(path)).unwrap();
        assert_eq!(*hash, Sha256::of_bytes(&on_disk), "hash de {path:?}");
    }
    assert_eq!(
        fs::read(root.join("dossier").join("sous").join("a.txt")).unwrap(),
        b"contenu A"
    );
    assert!(root.join("vide").is_dir());
    assert_eq!(
        plan_zip(&zip, "", &ArchiveLimits::default()).unwrap().len(),
        3
    );
}

#[test]
fn doublon_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let zip = zip_of(tmp.path(), &[("a.txt", b"1"), ("A.TXT", b"2")]);
    assert!(is_unsafe(extract(&zip, "", &tmp.path().join("root"))));
}

#[test]
fn archive_illisible_donne_une_erreur_zip() {
    let tmp = tempfile::tempdir().unwrap();
    let zip = write_zip(tmp.path(), b"ceci n'est pas un zip");
    assert!(matches!(
        extract(&zip, "", &tmp.path().join("root")),
        Err(ArchiveError::Zip(_))
    ));
    assert!(matches!(
        extract(&tmp.path().join("absent.zip"), "", &tmp.path().join("root")),
        Err(ArchiveError::Io { .. })
    ));
}

/// Crée un lien de dossier : lien symbolique sous Unix, jonction sous Windows
/// (une jonction ne demande aucun droit particulier). `false` si impossible.
fn make_dir_link(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (link, target);
        false
    }
}

#[test]
fn dossier_lien_deja_present_dans_la_racine_pas_suivi() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let outside = tmp.path().join("dehors");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    assert!(
        make_dir_link(&root.join("sub"), &outside),
        "création du lien impossible : test non valide"
    );

    let zip = zip_of(tmp.path(), &[("sub/evil.txt", b"mal")]);
    let result = extract(&zip, "", &root);

    assert!(
        matches!(result, Err(ArchiveError::Link(_))),
        "obtenu : {result:?}"
    );
    assert!(
        !outside.join("evil.txt").exists(),
        "écriture hors de la racine"
    );
}

#[test]
fn fichier_lien_deja_present_dans_la_racine_pas_suivi() {
    // Lien symbolique de fichier : seulement testable sous Unix (sous Windows il faut des droits).
    #[cfg(unix)]
    {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("victime.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, b"intact").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("f.txt")).unwrap();

        let zip = zip_of(tmp.path(), &[("f.txt", b"mal")]);
        assert!(matches!(
            extract(&zip, "", &root),
            Err(ArchiveError::Link(_))
        ));
        assert_eq!(fs::read(&outside).unwrap(), b"intact");
    }
}

#[test]
fn ecrase_un_fichier_normal_existant() {
    // Une mise à jour réécrit les fichiers déjà installés : ce n'est pas un lien, c'est permis.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), b"ancien").unwrap();
    let zip = zip_of(tmp.path(), &[("a.txt", b"nouveau")]);
    extract(&zip, "", &root).unwrap();
    assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"nouveau");
}

// ---------------------------------------------------------------------------
// .tar.gz
// ---------------------------------------------------------------------------

/// Fabrique un .tar.gz : (nom, type, contenu, cible du lien). Le nom est écrit tel quel
/// dans l'en-tête, sans les contrôles de `tar::Builder` (qui refuserait « ../x »).
fn build_tar_gz(entries: &[(&str, tar::EntryType, &[u8], &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind, data, link) in entries {
        let mut header = tar::Header::new_gnu();
        let old = header.as_old_mut();
        old.name[..name.len()].copy_from_slice(name.as_bytes());
        old.linkname[..link.len()].copy_from_slice(link.as_bytes());
        header.set_entry_type(*kind);
        header.set_mode(0o644);
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    let tar_bytes = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar_bytes).unwrap();
    gz.finish().unwrap()
}

fn files(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let list: Vec<_> = entries
        .iter()
        .map(|(n, d)| (*n, tar::EntryType::Regular, *d, ""))
        .collect();
    build_tar_gz(&list)
}

fn write_tgz(dir: &Path, bytes: &[u8]) -> PathBuf {
    let path = dir.join("test.tar.gz");
    fs::write(&path, bytes).unwrap();
    path
}

fn extract_tgz(file: &Path, strip: &str, root: &Path) -> Result<(), ArchiveError> {
    extract_tar_gz(file, strip, root, &ArchiveLimits::default(), &mut |_, _| {})
}

#[test]
fn tar_extraction_normale_avec_strip_prefix_et_hash() {
    let tmp = tempfile::tempdir().unwrap();
    let tgz = write_tgz(
        tmp.path(),
        &build_tar_gz(&[
            ("python/", tar::EntryType::Directory, b"", ""),
            ("python/lib/a.txt", tar::EntryType::Regular, b"AAA", ""),
            ("python/b.txt", tar::EntryType::Regular, b"BBB", ""),
            ("autre/c.txt", tar::EntryType::Regular, b"CCC", ""),
        ]),
    );
    let root = tmp.path().join("root");

    let mut seen: Vec<(PathBuf, Sha256)> = Vec::new();
    extract_tar_gz(
        &tgz,
        "python",
        &root,
        &ArchiveLimits::default(),
        &mut |path, hash| seen.push((path.to_path_buf(), hash)),
    )
    .unwrap();

    assert_eq!(seen.len(), 2);
    for (path, hash) in &seen {
        assert_eq!(*hash, Sha256::of_bytes(&fs::read(root.join(path)).unwrap()));
    }
    assert_eq!(fs::read(root.join("lib").join("a.txt")).unwrap(), b"AAA");
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"BBB");
    assert!(!root.join("autre").exists());
    assert!(!root.join("python").exists());
}

#[test]
fn tar_chemins_dangereux_refuses_sans_rien_ecrire() {
    for bad in ["../x", "a/../../x", "/abs", "C:\\x", "CON", "fichier:flux"] {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = write_tgz(tmp.path(), &files(&[("ok.txt", b"ok"), (bad, b"mal")]));
        let root = tmp.path().join("root");
        assert!(is_unsafe(extract_tgz(&tgz, "", &root)), "{bad} non refusé");
        assert!(!root.join("ok.txt").exists(), "{bad} : écriture partielle");
        assert!(!tmp.path().join("x").exists());
    }
}

#[test]
fn tar_liens_refuses() {
    for kind in [tar::EntryType::Symlink, tar::EntryType::Link] {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = write_tgz(
            tmp.path(),
            &build_tar_gz(&[
                ("ok.txt", tar::EntryType::Regular, b"ok", ""),
                ("lien", kind, b"", "/etc/passwd"),
            ]),
        );
        let root = tmp.path().join("root");
        assert!(matches!(
            extract_tgz(&tgz, "", &root),
            Err(ArchiveError::Link(_))
        ));
        assert!(!root.join("lien").exists());
        assert!(!root.join("ok.txt").exists());
    }
}

#[test]
fn tar_fichiers_speciaux_refuses() {
    for kind in [
        tar::EntryType::Fifo,
        tar::EntryType::Char,
        tar::EntryType::Block,
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let tgz = write_tgz(tmp.path(), &build_tar_gz(&[("dev", kind, b"", "")]));
        assert!(matches!(
            extract_tgz(&tgz, "", &tmp.path().join("root")),
            Err(ArchiveError::SpecialFile(_))
        ));
    }
}

#[test]
fn tar_bombe_refusee() {
    let tmp = tempfile::tempdir().unwrap();
    let big = vec![0u8; 4 * 1024 * 1024];
    let tgz = write_tgz(tmp.path(), &files(&[("bombe.bin", &big)]));
    let limits = ArchiveLimits {
        max_total_bytes: 1024 * 1024,
        ..ArchiveLimits::default()
    };
    let root = tmp.path().join("root");
    assert!(matches!(
        extract_tar_gz(&tgz, "", &root, &limits, &mut |_, _| {}),
        Err(ArchiveError::TooLarge)
    ));
    assert!(!root.join("bombe.bin").exists());
}

#[test]
fn tar_trop_d_entrees_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..11).map(|i| format!("f{i}.txt")).collect();
    let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"x"[..])).collect();
    let tgz = write_tgz(tmp.path(), &files(&entries));
    let limits = ArchiveLimits {
        max_entries: 10,
        ..ArchiveLimits::default()
    };
    let root = tmp.path().join("root");
    assert!(matches!(
        extract_tar_gz(&tgz, "", &root, &limits, &mut |_, _| {}),
        Err(ArchiveError::TooManyEntries)
    ));
    assert!(!root.exists());
}

#[test]
fn tar_doublon_insensible_a_la_casse_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let tgz = write_tgz(tmp.path(), &files(&[("a.txt", b"1"), ("A.TXT", b"2")]));
    assert!(is_unsafe(extract_tgz(&tgz, "", &tmp.path().join("root"))));
}

#[test]
fn tar_archive_tronquee_ou_corrompue() {
    let tmp = tempfile::tempdir().unwrap();
    // Contenu peu compressible pour que la coupure tombe au milieu des données.
    let noise: Vec<u8> = (0..200_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    let good = files(&[("a.bin", &noise)]);

    let cut = write_tgz(tmp.path(), &good[..good.len() / 2]);
    assert!(matches!(
        extract_tgz(&cut, "", &tmp.path().join("r1")),
        Err(ArchiveError::Tar(_))
    ));
    assert!(!tmp.path().join("r1").exists());

    let junk = write_tgz(tmp.path(), b"ceci n'est pas un tar.gz");
    assert!(matches!(
        extract_tgz(&junk, "", &tmp.path().join("r2")),
        Err(ArchiveError::Tar(_))
    ));
    assert!(matches!(
        extract_tgz(
            &tmp.path().join("absent.tar.gz"),
            "",
            &tmp.path().join("r3")
        ),
        Err(ArchiveError::Io { .. })
    ));
}

#[test]
fn tar_dossier_lien_deja_present_pas_suivi() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    let outside = tmp.path().join("dehors");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    assert!(make_dir_link(&root.join("sub"), &outside));

    let tgz = write_tgz(tmp.path(), &files(&[("sub/evil.txt", b"mal")]));
    assert!(matches!(
        extract_tgz(&tgz, "", &root),
        Err(ArchiveError::Link(_))
    ));
    assert!(!outside.join("evil.txt").exists());
}
