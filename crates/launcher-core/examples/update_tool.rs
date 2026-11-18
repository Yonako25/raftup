//! Outil de publication d'une mise à jour du launcher (F12, ADR 0022). À lancer à la main,
//! sur la machine de la mainteneuse : `cargo run -p launcher-core --example update_tool -- <commande>`.
//!
//! - `keygen <dossier>` : crée la paire de clés minisign (sans mot de passe) dans un
//!   dossier HORS du dépôt, et refuse d'écraser des clés existantes. Seule la clé
//!   publique (à coller dans `update::UPDATE_PUBLIC_KEY`) est affichée.
//! - `sign <fichier> <clé-privée>` : écrit `<fichier>.minisig` (minisign pré-haché).
//! - `manifest <fichier> <version> <url> <fichier.minisig>` : écrit `update.json`
//!   (format de l'ADR 0022) à côté du fichier, avec son sha256 et sa taille.
//!
//! La clé privée n'est jamais affichée ni journalisée.

use std::error::Error;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use launcher_core::hash::Sha256;
use launcher_core::update::{parse_manifest, FORMAT_VERSION};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Noms des fichiers de clés écrits par `keygen`.
const SECRET_NAME: &str = "raftup-update.key";
const PUBLIC_NAME: &str = "raftup-update.pub";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["keygen", dir] => {
            let public = keygen(Path::new(dir))?;
            println!("Paire de clés créée dans {dir}");
            println!("Clé publique (pour UPDATE_PUBLIC_KEY) : {public}");
        }
        ["sign", file, key] => {
            let out = sign_file(Path::new(file), Path::new(key))?;
            println!("Signature écrite : {}", out.display());
        }
        ["manifest", file, version, url, minisig] => {
            let out = write_manifest(Path::new(file), version, url, Path::new(minisig))?;
            println!("Manifeste écrit : {}", out.display());
        }
        _ => {
            eprintln!(
                "usage :\n  update_tool keygen <dossier>\n  update_tool sign <fichier> <clé-privée>\n  \
                 update_tool manifest <fichier> <version> <url> <fichier.minisig>"
            );
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Crée la paire de clés dans `dir` et rend la clé publique en base64.
/// Refuse un dossier dans le dépôt, et d'écraser un fichier existant.
fn keygen(dir: &Path) -> Result<String> {
    fs::create_dir_all(dir)?;
    // Le dépôt est celui de la compilation de cet outil : la clé privée n'y a pas sa place.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    if dir.canonicalize()?.starts_with(&repo) {
        return Err("le dossier des clés doit être hors du dépôt".into());
    }
    let secret_path = dir.join(SECRET_NAME);
    let public_path = dir.join(PUBLIC_NAME);
    if secret_path.exists() || public_path.exists() {
        return Err(format!(
            "des clés existent déjà dans {} : refus d'écraser",
            dir.display()
        )
        .into());
    }
    let pair = minisign::KeyPair::generate_unencrypted_keypair()?;
    // `create_new` échoue si le fichier existe déjà : second garde-fou contre l'écrasement.
    let mut secret = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&secret_path)?;
    secret.write_all(
        pair.sk
            .to_box(Some("clé de mise à jour Raftup"))?
            .into_string()
            .as_bytes(),
    )?;
    fs::write(&public_path, pair.pk.to_box()?.into_string())?;
    Ok(pair.pk.to_base64())
}

/// Signe `file` (minisign pré-haché) et écrit `<file>.minisig`.
fn sign_file(file: &Path, key: &Path) -> Result<PathBuf> {
    let secret = minisign::SecretKeyBox::from_string(&fs::read_to_string(key)?)?
        .into_unencrypted_secret_key()?;
    let data = fs::read(file)?;
    let signature = minisign::sign(
        None,
        &secret,
        Cursor::new(data),
        Some("raftup update"),
        Some("signature Raftup"),
    )?;
    let mut name = file.as_os_str().to_owned();
    name.push(".minisig");
    let out = PathBuf::from(name);
    fs::write(&out, signature.into_string())?;
    Ok(out)
}

/// Écrit `update.json` à côté de `file`. Le résultat est relu par `parse_manifest`
/// avant d'être écrit : un manifeste que le launcher refuserait n'est jamais publié.
fn write_manifest(file: &Path, version: &str, url: &str, minisig: &Path) -> Result<PathBuf> {
    let data = fs::read(file)?;
    let manifest = serde_json::json!({
        "format_version": FORMAT_VERSION,
        "version": version,
        "url": url,
        "sha256": Sha256::of_bytes(&data).to_string(),
        "max_bytes": data.len(),
        "signature": fs::read_to_string(minisig)?,
    });
    let text = serde_json::to_string_pretty(&manifest)?;
    parse_manifest(&text)?;
    let out = file.with_file_name("update.json");
    fs::write(&out, text)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::download::{DirFetcher, Fetcher};
    use launcher_core::update::{check, download_verified, fetch_manifest, UpdateStatus};
    use launcher_core::{CancelToken, Downloader};

    const URL: &str = "https://github.com/Yonako25/raftup/releases/download/v9.0.0/raftup.exe";
    const MANIFEST_URL: &str =
        "https://github.com/Yonako25/raftup/releases/latest/download/update.json";

    #[test]
    fn keygen_sign_manifest_puis_verification_par_le_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let keys = dir.path().join("keys");
        let public = keygen(&keys).unwrap();
        // Pas d'écrasement.
        assert!(keygen(&keys).is_err());

        let binary = dir.path().join("raftup.exe");
        fs::write(&binary, b"faux nouveau launcher").unwrap();
        let minisig = sign_file(&binary, &keys.join(SECRET_NAME)).unwrap();
        assert_eq!(minisig, dir.path().join("raftup.exe.minisig"));
        let manifest_path = write_manifest(&binary, "9.0.0", URL, &minisig).unwrap();

        // Le launcher relit le manifeste, voit la mise à jour, télécharge et vérifie.
        let fetcher = DirFetcher::new()
            .with_file(MANIFEST_URL, &manifest_path)
            .with_file(URL, &binary);
        let manifest = fetch_manifest(&fetcher, MANIFEST_URL).unwrap();
        let current = launcher_core::update::current_version().unwrap();
        assert!(matches!(
            check(&manifest, &current).unwrap(),
            UpdateStatus::Available(_)
        ));
        let boxed: Box<dyn Fetcher> = Box::new(fetcher);
        let downloader = Downloader::new(dir.path().join("cache"), boxed);
        let path = download_verified(
            &manifest,
            &public,
            &downloader,
            &mut |_| {},
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), b"faux nouveau launcher");

        // Une autre clé publique rejette la signature.
        let other = keygen(&dir.path().join("other")).unwrap();
        let result = download_verified(
            &manifest,
            &other,
            &downloader,
            &mut |_| {},
            &CancelToken::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn keygen_refuse_un_dossier_dans_le_depot() {
        let inside = Path::new(env!("CARGO_MANIFEST_DIR")).join("target-test-keys");
        let result = keygen(&inside);
        let _ = fs::remove_dir(&inside);
        assert!(result.is_err());
    }
}
