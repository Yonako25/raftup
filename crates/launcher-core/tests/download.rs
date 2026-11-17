//! Tests du téléchargement : aucun réseau (sauf le test `#[ignore]`), tout passe par `DirFetcher`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

use launcher_core::download::{DirFetcher, DownloadError, Downloader, HttpFetcher};
use launcher_core::hash::Sha256;

const URL: &str = "https://exemple.test/mod.zip";

/// Prépare un dossier de test : une source contenant `content`, un cache vide.
fn setup(content: &[u8]) -> (tempfile::TempDir, DirFetcher) {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source.bin");
    fs::write(&source, content).unwrap();
    (tmp, DirFetcher::new().with_file(URL, source))
}

fn downloader(tmp: &tempfile::TempDir, fetcher: DirFetcher) -> Downloader {
    Downloader::new(tmp.path().join("cache"), Box::new(fetcher))
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn bon_hash_range_dans_le_cache() {
    let (tmp, fetcher) = setup(b"contenu du mod");
    let hash = Sha256::of_bytes(b"contenu du mod");
    let dl = downloader(&tmp, fetcher);
    let mut last = 0;
    let path = dl.get(URL, &hash, 1024, &mut |n| last = n).unwrap();
    assert_eq!(path, tmp.path().join("cache").join(hash.to_string()));
    assert_eq!(fs::read(&path).unwrap(), b"contenu du mod");
    assert_eq!(last, 14);
    // Seul le fichier final reste : pas de `.part`.
    assert_eq!(names(&tmp.path().join("cache")), vec![hash.to_string()]);
}

#[test]
fn mauvais_hash_ne_laisse_rien() {
    let (tmp, fetcher) = setup(b"contenu du mod");
    let faux = Sha256::of_bytes(b"autre chose");
    let dl = downloader(&tmp, fetcher);
    let err = dl.get(URL, &faux, 1024, &mut |_| {}).unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }), "{err:?}");
    assert!(names(&tmp.path().join("cache")).is_empty());
}

#[test]
fn au_dela_de_la_taille_maximale() {
    let (tmp, fetcher) = setup(&[7u8; 5000]);
    let hash = Sha256::of_bytes(&[7u8; 5000]);
    let dl = downloader(&tmp, fetcher);
    let err = dl.get(URL, &hash, 4999, &mut |_| {}).unwrap_err();
    assert!(
        matches!(err, DownloadError::TooLarge { max: 4999, .. }),
        "{err:?}"
    );
    assert!(names(&tmp.path().join("cache")).is_empty());
    // Pile à la limite : accepté.
    assert!(dl.get(URL, &hash, 5000, &mut |_| {}).is_ok());
}

#[test]
fn fichier_deja_en_cache_pas_de_nouveau_telechargement() {
    let (tmp, fetcher) = setup(b"abc");
    let opens = fetcher.open_counter();
    let hash = Sha256::of_bytes(b"abc");
    let dl = downloader(&tmp, fetcher);
    dl.get(URL, &hash, 1024, &mut |_| {}).unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    let mut progress = 0;
    dl.get(URL, &hash, 1024, &mut |n| progress = n).unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 1, "le cache doit suffire");
    assert_eq!(progress, 3);
}

#[test]
fn cache_corrompu_est_retelecharge() {
    let (tmp, fetcher) = setup(b"abc");
    let opens = fetcher.open_counter();
    let hash = Sha256::of_bytes(b"abc");
    let dl = downloader(&tmp, fetcher);
    let path = dl.get(URL, &hash, 1024, &mut |_| {}).unwrap();
    fs::write(&path, b"abXX").unwrap();
    let path = dl.get(URL, &hash, 1024, &mut |_| {}).unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 2);
    assert_eq!(fs::read(&path).unwrap(), b"abc");
}

#[test]
fn http_refuse() {
    let (tmp, fetcher) = setup(b"abc");
    let opens = fetcher.open_counter();
    let hash = Sha256::of_bytes(b"abc");
    let dl = downloader(&tmp, fetcher);
    let err = dl
        .get("http://exemple.test/mod.zip", &hash, 1024, &mut |_| {})
        .unwrap_err();
    assert!(matches!(err, DownloadError::NotHttps(_)), "{err:?}");
    assert_eq!(opens.load(Ordering::SeqCst), 0);
    // Le vrai client refuse aussi, sans toucher au réseau.
    let err = launcher_core::download::Fetcher::open(&HttpFetcher::new(), "http://a.test/x");
    assert!(matches!(err, Err(DownloadError::NotHttps(_))));
}

#[test]
fn url_inconnue_donne_une_erreur_http() {
    let (tmp, fetcher) = setup(b"abc");
    let hash = Sha256::of_bytes(b"abc");
    let dl = downloader(&tmp, fetcher);
    let err = dl
        .get("https://exemple.test/autre", &hash, 1024, &mut |_| {})
        .unwrap_err();
    assert!(
        matches!(err, DownloadError::Http { status: 404, .. }),
        "{err:?}"
    );
    assert!(names(&tmp.path().join("cache")).is_empty());
}

/// Test réseau réel : `cargo test -p launcher-core --test download -- --ignored`.
#[test]
#[ignore = "accède à Internet"]
fn telechargement_reel() {
    let tmp = tempfile::tempdir().unwrap();
    let dl = Downloader::new(tmp.path().join("cache"), Box::new(HttpFetcher::new()));
    // Mauvais hash volontaire : on vérifie que la requête HTTPS aboutit jusqu'au contrôle.
    let faux = Sha256::of_bytes(b"pas le bon");
    let err = dl
        .get(
            "https://www.rust-lang.org/robots.txt",
            &faux,
            1 << 20,
            &mut |_| {},
        )
        .unwrap_err();
    assert!(matches!(err, DownloadError::HashMismatch { .. }), "{err:?}");
    assert!(names(&tmp.path().join("cache")).is_empty());
}
