//! Publication **locale** d'un index (fonctionnalité `publish`, ADR 0032).
//!
//! On part d'un fichier source TOML relu par la mainteneuse, on calcule le sha256 et la
//! taille de chaque paquet depuis un fichier local, on écrit `index.json`, puis on le signe
//! (`index.json.minisig`). **Rien n'est envoyé** : la mise en ligne reste un geste manuel.
//!
//! Exemple de source :
//!
//! ```toml
//! format_version = 1
//! registry = "raftup"
//!
//! [[package]]
//! kind = "apworld"
//! name = "slime_rancher"
//! version = "0.3.2"
//! game = "Slime Rancher"
//! file = "dist/slime_rancher.apworld"   # relatif au fichier source
//! url = "https://github.com/auteur/depot/releases/download/v0.3.2/slime_rancher.apworld"
//! license = "unlicensed"
//! authors = ["auteur"]
//! ```

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::RegistryError;
use crate::index::{Index, Package, PackageKind, FORMAT_VERSION, PACKAGE_MAX_BYTES};
use crate::verify::{sha256_hex, verify_index};

/// Nom du fichier d'index écrit par [`publish`].
pub const INDEX_FILE: &str = "index.json";
/// Nom du fichier de signature écrit par [`publish`].
pub const SIGNATURE_FILE: &str = "index.json.minisig";
/// Noms des fichiers de clés écrits par [`keygen`].
pub const SECRET_KEY_FILE: &str = "raftup-registry.key";
pub const PUBLIC_KEY_FILE: &str = "raftup-registry.pub";

/// Fichier source d'une publication.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    format_version: u32,
    registry: String,
    /// `rename` : en TOML, chaque `[[package]]` est un élément de cette liste.
    #[serde(default, rename = "package")]
    packages: Vec<SourcePackage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourcePackage {
    kind: PackageKind,
    name: String,
    version: String,
    #[serde(default)]
    game: Option<String>,
    /// Chemin local du paquet, pour calculer son sha256 et sa taille.
    file: PathBuf,
    url: String,
    license: String,
    authors: Vec<String>,
}

/// Résultat d'une publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub index_path: PathBuf,
    pub signature_path: PathBuf,
    /// Nombre de paquets dans l'index.
    pub packages: usize,
    /// Clé publique qui vérifie la signature (à comparer à celle du launcher).
    pub public_key: String,
}

/// Construit l'index depuis un fichier source : lit chaque paquet local pour son sha256
/// et sa taille, valide et trie. N'écrit rien.
pub fn build_index(source_path: &Path) -> Result<Index, RegistryError> {
    let text = read_text(source_path)?;
    let source: Source = toml::from_str(&text).map_err(|e| {
        RegistryError::Parse(format!("{} : {}", source_path.display(), e.message()))
    })?;
    if source.format_version != FORMAT_VERSION {
        return Err(RegistryError::UnsupportedFormat {
            found: source.format_version,
            supported: FORMAT_VERSION,
        });
    }
    // Les chemins `file` sont relatifs au dossier du fichier source.
    let base = source_path.parent().unwrap_or(Path::new("."));
    let mut index = Index::new(&source.registry);
    for item in source.packages {
        let path = base.join(&item.file);
        let data = read_package(&path)?;
        if item.kind == PackageKind::Profile {
            check_profile_text(&item.name, &data)?;
        }
        index.packages.push(Package {
            kind: item.kind,
            name: item.name,
            version: item.version,
            game: item.game,
            url: item.url,
            sha256: sha256_hex(&data),
            size: data.len() as u64,
            license: item.license,
            authors: item.authors,
        });
    }
    index.sort();
    index.validate()?;
    Ok(index)
}

/// Signe les octets d'un index avec une clé secrète minisign (texte du fichier `.key`,
/// sans mot de passe). Rend (signature, clé publique en base64).
pub fn sign_index(index_bytes: &[u8], secret_key: &str) -> Result<(String, String), RegistryError> {
    let signing = |e: minisign::PError| RegistryError::Signing(e.to_string());
    let secret = minisign::SecretKeyBox::from_string(secret_key)
        .and_then(|b| b.into_unencrypted_secret_key())
        .map_err(signing)?;
    let public = minisign::PublicKey::from_secret_key(&secret).map_err(signing)?;
    // Avec la clé publique, `sign` revérifie aussitôt la signature qu'il vient de produire.
    let signature = minisign::sign(
        Some(&public),
        &secret,
        Cursor::new(index_bytes),
        Some("raftup registry"),
        Some("signature du registre Raftup"),
    )
    .map_err(signing)?;
    Ok((signature.into_string(), public.to_base64()))
}

/// Publie en local : construit l'index, l'écrit dans `out_dir` avec sa signature, puis
/// relit les deux fichiers et les vérifie comme le ferait le launcher.
pub fn publish(
    source_path: &Path,
    secret_key_path: &Path,
    out_dir: &Path,
) -> Result<PublishReport, RegistryError> {
    let index = build_index(source_path)?;
    let bytes = index.to_bytes()?;
    let secret = read_text(secret_key_path)?;
    let (signature, public_key) = sign_index(&bytes, &secret)?;

    fs::create_dir_all(out_dir).map_err(io(out_dir))?;
    let index_path = out_dir.join(INDEX_FILE);
    let signature_path = out_dir.join(SIGNATURE_FILE);
    write_atomic(&index_path, &bytes)?;
    write_atomic(&signature_path, signature.as_bytes())?;

    // Relecture : un index que le launcher refuserait n'est jamais laissé tel quel.
    let written = fs::read(&index_path).map_err(io(&index_path))?;
    let written_signature = read_text(&signature_path)?;
    verify_index(&written, &written_signature, &public_key)?;

    Ok(PublishReport {
        index_path,
        signature_path,
        packages: index.packages.len(),
        public_key,
    })
}

/// Crée une paire de clés minisign (sans mot de passe) dans `dir` et rend la clé publique.
/// Refuse d'écraser des clés existantes. La clé secrète n'est jamais rendue ni affichée.
pub fn keygen(dir: &Path) -> Result<String, RegistryError> {
    fs::create_dir_all(dir).map_err(io(dir))?;
    let secret_path = dir.join(SECRET_KEY_FILE);
    let public_path = dir.join(PUBLIC_KEY_FILE);
    if secret_path.exists() || public_path.exists() {
        return Err(RegistryError::Signing(format!(
            "des clés existent déjà dans {} : refus d'écraser",
            dir.display()
        )));
    }
    let signing = |e: minisign::PError| RegistryError::Signing(e.to_string());
    let pair = minisign::KeyPair::generate_unencrypted_keypair().map_err(signing)?;
    let secret_text = pair
        .sk
        .to_box(Some("clé du registre Raftup"))
        .map_err(signing)?
        .into_string();
    let public_text = pair.pk.to_box().map_err(signing)?.into_string();
    // `create_new` échoue si le fichier existe déjà : second garde-fou contre l'écrasement.
    for (path, text) in [(&secret_path, &secret_text), (&public_path, &public_text)] {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(io(path))?;
        file.write_all(text.as_bytes()).map_err(io(path))?;
    }
    Ok(pair.pk.to_base64())
}

/// Un profil doit être un TOML lisible dont `game.id` est le nom du paquet. Le launcher le
/// revalide en entier (`Profile::parse`) ; ici, on arrête les erreurs grossières tôt.
fn check_profile_text(name: &str, data: &[u8]) -> Result<(), RegistryError> {
    let bad = |reason: String| RegistryError::InvalidPackage {
        package: format!("profile {name}"),
        reason,
    };
    let text = std::str::from_utf8(data).map_err(|_| bad("profil non UTF-8".to_string()))?;
    let value: toml::Table = toml::from_str(text).map_err(|e| bad(e.message().to_string()))?;
    let id = value
        .get("game")
        .and_then(|g| g.get("id"))
        .and_then(|id| id.as_str());
    if id != Some(name) {
        return Err(bad(format!(
            "game.id du profil ({}) différent du nom du paquet",
            id.unwrap_or("absent")
        )));
    }
    Ok(())
}

/// Lit un paquet local, borné à [`PACKAGE_MAX_BYTES`].
fn read_package(path: &Path) -> Result<Vec<u8>, RegistryError> {
    let file = fs::File::open(path).map_err(io(path))?;
    let mut data = Vec::new();
    // `take` borne la lecture : un fichier géant ne remplit pas la mémoire.
    file.take(PACKAGE_MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(io(path))?;
    if data.len() as u64 > PACKAGE_MAX_BYTES {
        return Err(RegistryError::TooLarge {
            what: "paquet",
            max: PACKAGE_MAX_BYTES,
        });
    }
    Ok(data)
}

fn read_text(path: &Path) -> Result<String, RegistryError> {
    fs::read_to_string(path).map_err(io(path))
}

/// Écrit dans `<path>.tmp` puis renomme : un fichier n'est jamais laissé à moitié écrit.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), RegistryError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, data).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
}

/// Fabrique une fonction qui transforme une erreur d'entrée-sortie en `RegistryError::Io`.
/// `impl Fn` : on rend une fermeture qui possède sa copie du chemin.
fn io(path: &Path) -> impl Fn(std::io::Error) -> RegistryError {
    let path = path.to_path_buf();
    move |source| RegistryError::Io {
        path: path.clone(),
        source,
    }
}
