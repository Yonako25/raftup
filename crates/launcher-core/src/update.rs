//! Mise à jour automatique du launcher : cœur sans interface (F12, ADR 0022).
//!
//! Ce module ne fait que **vérifier et télécharger**. Il n'exécute jamais le fichier
//! reçu : l'installation viendra dans une tâche séparée, après accord de l'utilisatrice.
//!
//! Déroulement :
//! 1. `fetch_manifest` / `parse_manifest` lisent le manifeste JSON (versionné) ;
//! 2. `check` le compare à la version courante (semver) ;
//! 3. `download_verified` télécharge (HTTPS, domaine de la liste blanche, taille bornée),
//!    contrôle le sha256 puis la signature minisign, et rend le chemin du fichier.
//!    Un fichier dont la signature est mauvaise est supprimé.
//!
//! Le sha256 protège contre une corruption ; la signature (clé privée gardée hors du
//! dépôt) protège contre un serveur ou un compte GitHub compromis.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

use py_bridge::CancelToken;

use crate::download::{DownloadError, Downloader, Fetcher};
use crate::hash::Sha256;
use crate::profile::ALLOWED_HOSTS;

/// Version du format de manifeste que ce launcher sait lire.
pub const FORMAT_VERSION: u32 = 1;

/// Taille maximale du manifeste lui-même (64 Kio : c'est un petit JSON).
pub const MANIFEST_MAX_BYTES: u64 = 64 * 1024;

/// Plafond absolu de la taille d'une mise à jour, quoi que dise le manifeste (512 Mio).
pub const UPDATE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Clé publique minisign (ligne base64 commençant par « RW »).
///
/// Clé PUBLIQUE de la paire générée par `examples/update_tool.rs keygen` (ADR 0022). Si elle
/// était vide, toute vérification échouerait avec `UpdateError::PublicKeyNotConfigured`
/// plutôt que d'accepter n'importe quoi. Les tests passent leur propre clé aux fonctions.
pub const UPDATE_PUBLIC_KEY: &str = "RWS+YnudIFIVAFDtalsvodpSlxBLmoOCG7qs5tB7VUHc6n+H3dot3FjI";

/// Adresse du manifeste (dernière release GitHub du dépôt, ADR 0022).
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/Yonako25/raftup/releases/latest/download/update.json";

/// Ce qui fait échouer la lecture, la vérification ou le téléchargement d'une mise à jour.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Le manifeste n'est pas un JSON du bon schéma.
    #[error("manifeste de mise à jour illisible : {0}")]
    Manifest(String),
    /// Le manifeste est d'un format plus récent (ou inconnu).
    #[error(
        "format de manifeste {found} non pris en charge (ce launcher lit le format {supported})"
    )]
    UnsupportedFormat { found: u32, supported: u32 },
    /// Numéro de version qui n'est pas du semver.
    #[error("version « {value} » invalide : {reason}")]
    BadVersion { value: String, reason: String },
    /// Adresse refusée (pas HTTPS, domaine hors liste blanche, identifiants, port…).
    #[error("adresse de mise à jour refusée : {0}")]
    BadUrl(String),
    /// Taille maximale nulle ou au-dessus du plafond.
    #[error("taille maximale invalide : {0} octets")]
    BadSize(u64),
    /// Le manifeste ne contient pas de signature.
    #[error("le manifeste ne contient pas de signature")]
    MissingSignature,
    /// Le manifeste dépasse `MANIFEST_MAX_BYTES`.
    #[error("manifeste trop gros (plus de {0} octets)")]
    ManifestTooLarge(u64),
    /// Aucune clé publique n'est compilée dans le launcher.
    #[error("clé publique de mise à jour non configurée")]
    PublicKeyNotConfigured,
    /// La clé publique n'est pas une clé minisign valide.
    #[error("clé publique minisign invalide : {0}")]
    BadPublicKey(String),
    /// La signature n'est pas au format minisign.
    #[error("signature minisign illisible : {0}")]
    BadSignature(String),
    /// La signature ne correspond pas au fichier : il est supprimé.
    #[error("signature minisign incorrecte : fichier rejeté")]
    SignatureMismatch,
    /// Échec du téléchargement (réseau, taille, sha256…).
    // `#[from]` : `?` convertit tout seul une `DownloadError` en `UpdateError`.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Lecture ou suppression de fichier impossible.
    #[error("accès à {path} : {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Manifeste de mise à jour : un petit JSON publié à une adresse fixe (ADR 0022).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpdateManifest {
    /// Version du format du manifeste (`FORMAT_VERSION`).
    pub format_version: u32,
    /// Version proposée, en semver (« 0.2.0 »).
    pub version: String,
    /// Adresse HTTPS du fichier à télécharger.
    pub url: String,
    /// Empreinte sha256 du fichier.
    pub sha256: Sha256,
    /// Taille maximale acceptée en octets.
    pub max_bytes: u64,
    /// Fichier `.minisig` complet (texte) qui signe le fichier téléchargé.
    pub signature: String,
    /// Notes de version affichées au joueur (facultatif).
    #[serde(default)]
    pub notes: Option<String>,
}

/// Résultat de la comparaison avec la version courante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Rien de plus récent (version égale, ou plus ancienne : pas de retour en arrière).
    UpToDate,
    /// Une version plus récente existe.
    Available(Version),
}

/// Lit un manifeste JSON et le valide (format, version, adresse, taille, signature présente).
pub fn parse_manifest(text: &str) -> Result<UpdateManifest, UpdateError> {
    let manifest: UpdateManifest =
        serde_json::from_str(text).map_err(|e| UpdateError::Manifest(e.to_string()))?;
    manifest.validate()?;
    Ok(manifest)
}

impl UpdateManifest {
    /// Contrôles sans réseau ni disque. Rappelés avant tout téléchargement, car la
    /// structure a des champs publics et peut être construite à la main.
    pub fn validate(&self) -> Result<(), UpdateError> {
        if self.format_version != FORMAT_VERSION {
            return Err(UpdateError::UnsupportedFormat {
                found: self.format_version,
                supported: FORMAT_VERSION,
            });
        }
        parse_version(&self.version)?;
        check_url(&self.url)?;
        if self.max_bytes == 0 || self.max_bytes > UPDATE_MAX_BYTES {
            return Err(UpdateError::BadSize(self.max_bytes));
        }
        if self.signature.trim().is_empty() {
            return Err(UpdateError::MissingSignature);
        }
        Ok(())
    }
}

/// Lit un numéro de version semver.
pub fn parse_version(text: &str) -> Result<Version, UpdateError> {
    Version::parse(text).map_err(|e| UpdateError::BadVersion {
        value: text.to_string(),
        reason: e.to_string(),
    })
}

/// Version du launcher en cours d'exécution (celle de `Cargo.toml`).
pub fn current_version() -> Result<Version, UpdateError> {
    parse_version(env!("CARGO_PKG_VERSION"))
}

/// Compare le manifeste à la version courante. Ne touche ni au réseau ni au disque.
pub fn check(manifest: &UpdateManifest, current: &Version) -> Result<UpdateStatus, UpdateError> {
    manifest.validate()?;
    let offered = parse_version(&manifest.version)?;
    // `Version` implémente l'ordre semver : « 1.0.0-beta » < « 1.0.0 ».
    if offered > *current {
        Ok(UpdateStatus::Available(offered))
    } else {
        Ok(UpdateStatus::UpToDate)
    }
}

/// Télécharge le manifeste à `url` (mêmes règles d'adresse que le fichier de mise à jour).
///
/// `&dyn Fetcher` : on emprunte le client HTTP (ou le faux des tests) sans le posséder.
pub fn fetch_manifest(fetcher: &dyn Fetcher, url: &str) -> Result<UpdateManifest, UpdateError> {
    check_url(url)?;
    let reader = fetcher.open(url)?;
    // `take` borne la lecture : un serveur qui envoie un flux infini ne remplit pas la mémoire.
    let mut bytes = Vec::new();
    reader
        .take(MANIFEST_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            UpdateError::Download(DownloadError::Network {
                url: url.to_string(),
                reason: e.to_string(),
            })
        })?;
    if bytes.len() as u64 > MANIFEST_MAX_BYTES {
        return Err(UpdateError::ManifestTooLarge(MANIFEST_MAX_BYTES));
    }
    let text = String::from_utf8(bytes).map_err(|e| UpdateError::Manifest(e.to_string()))?;
    parse_manifest(&text)
}

/// Télécharge le fichier annoncé, vérifie son sha256 puis sa signature minisign.
///
/// Le fichier est rangé dans le dossier de `downloader` (à choisir temporaire côté
/// appelant). Si la signature est mauvaise, le fichier est supprimé. **Rien n'est exécuté.**
pub fn download_verified(
    manifest: &UpdateManifest,
    public_key: &str,
    downloader: &Downloader,
    progress: &mut dyn FnMut(u64),
    cancel: &CancelToken,
) -> Result<PathBuf, UpdateError> {
    manifest.validate()?;
    // La clé est lue avant le téléchargement : inutile de télécharger si elle est absente.
    let key = load_public_key(public_key)?;
    let signature = Signature::decode(&manifest.signature)
        .map_err(|e| UpdateError::BadSignature(e.to_string()))?;

    // `Downloader` contrôle HTTPS, taille (pendant la lecture) et sha256.
    let path = downloader.get_cancellable(
        &manifest.url,
        &manifest.sha256,
        manifest.max_bytes,
        progress,
        cancel,
    )?;

    // Le fichier fait au plus `max_bytes` (≤ 512 Mio) : on le relit en mémoire pour la signature.
    let data = fs::read(&path).map_err(|source| UpdateError::Io {
        path: path.clone(),
        source,
    })?;
    // `false` : on refuse les signatures « legacy » (non pré-hachées), plus faibles.
    if key.verify(&data, &signature, false).is_err() {
        // Le résultat de la suppression est ignoré : l'erreur de signature est plus utile.
        let _ = fs::remove_file(&path);
        return Err(UpdateError::SignatureMismatch);
    }
    Ok(path)
}

/// Chemin de l'ancien exécutable après une mise à jour : `raftup.exe` devient `raftup.exe.old`.
pub fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// Remplace l'exécutable `exe` par le fichier déjà vérifié `new_file`.
///
/// Windows interdit d'écraser un `.exe` en cours d'exécution, mais autorise à le
/// **renommer** : l'ancien devient `exe.old` (supprimé au démarrage suivant par
/// [`cleanup_old`]) et le nouveau prend sa place. Si une étape échoue, l'ancien
/// exécutable est remis en place. Rien n'est lancé ; `new_file` n'est pas supprimé.
pub fn apply_update(exe: &Path, new_file: &Path) -> Result<(), UpdateError> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source: std::io::Error| UpdateError::Io { path, source }
    };
    let old = old_path(exe);
    let staged = {
        let mut name = exe.as_os_str().to_owned();
        name.push(".new");
        PathBuf::from(name)
    };
    // 1. Le nouveau fichier est copié à côté de l'exécutable (même disque).
    fs::copy(new_file, &staged).map_err(io(&staged))?;
    // 2. Un `.old` laissé par une mise à jour précédente est enlevé s'il le peut.
    if old.exists() {
        fs::remove_file(&old).map_err(io(&old)).inspect_err(|_| {
            let _ = fs::remove_file(&staged);
        })?;
    }
    // 3. L'exécutable en cours est renommé, puis le nouveau prend sa place.
    fs::rename(exe, &old).map_err(io(exe)).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;
    if let Err(source) = fs::rename(&staged, exe) {
        // Retour en arrière : sans exécutable, le launcher ne redémarrerait plus.
        let _ = fs::rename(&old, exe);
        let _ = fs::remove_file(&staged);
        return Err(UpdateError::Io {
            path: exe.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// Supprime l'ancien exécutable (`.old`) laissé par une mise à jour. Silencieux : s'il
/// est encore verrouillé, on réessaiera au démarrage suivant.
pub fn cleanup_old(exe: &Path) {
    let _ = fs::remove_file(old_path(exe));
}

/// Lit la clé publique minisign (ligne base64).
fn load_public_key(public_key: &str) -> Result<PublicKey, UpdateError> {
    let trimmed = public_key.trim();
    if trimmed.is_empty() {
        return Err(UpdateError::PublicKeyNotConfigured);
    }
    PublicKey::from_base64(trimmed).map_err(|e| UpdateError::BadPublicKey(e.to_string()))
}

/// HTTPS obligatoire, hôte dans la liste blanche (GitHub inclus), ni identifiants ni port.
fn check_url(url: &str) -> Result<(), UpdateError> {
    let bad = |why: &str| UpdateError::BadUrl(format!("« {url} » : {why}"));
    let Some(rest) = url.strip_prefix("https://") else {
        return Err(bad("HTTPS obligatoire"));
    };
    if url.contains([' ', '\0', '\n', '\r', '{', '}']) {
        return Err(bad("caractère interdit"));
    }
    // L'hôte s'arrête au premier « / », « ? » ou « # ». Un « @ » (identifiants) ou
    // un « : » (port) servirait à tromper la liste blanche : refusés.
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.contains(['@', ':']) {
        return Err(bad("hôte invalide"));
    }
    if !ALLOWED_HOSTS.contains(&host.to_ascii_lowercase().as_str()) {
        return Err(bad("domaine hors de la liste blanche"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::Path;

    use super::*;
    use crate::download::DirFetcher;

    const URL: &str = "https://github.com/Yonako25/raftup/releases/download/v0.2.0/raftup.zip";
    const MANIFEST_URL: &str =
        "https://github.com/Yonako25/raftup/releases/latest/download/update.json";

    /// Fabrique une paire de clés de test et signe `data`. Rend (clé publique, signature).
    fn sign(data: &[u8]) -> (String, String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(
            Some(&pair.pk),
            &pair.sk,
            Cursor::new(data),
            Some("test"),
            Some("signature de test"),
        )
        .unwrap();
        (pair.pk.to_base64(), signature.into_string())
    }

    fn manifest_for(data: &[u8], signature: &str) -> UpdateManifest {
        UpdateManifest {
            format_version: FORMAT_VERSION,
            version: "0.2.0".into(),
            url: URL.into(),
            sha256: Sha256::of_bytes(data),
            max_bytes: 1024,
            signature: signature.into(),
            notes: None,
        }
    }

    /// Un dossier avec le faux binaire, et un `Downloader` qui le sert à `URL`.
    fn downloader(dir: &Path, data: &[u8]) -> Downloader {
        let source = dir.join("source.bin");
        fs::write(&source, data).unwrap();
        let fetcher = DirFetcher::new().with_file(URL, source);
        Downloader::new(dir.join("cache"), Box::new(fetcher))
    }

    fn run(manifest: &UpdateManifest, key: &str, dl: &Downloader) -> Result<PathBuf, UpdateError> {
        download_verified(manifest, key, dl, &mut |_| {}, &CancelToken::new())
    }

    fn json(version: &str, url: &str) -> String {
        format!(
            r#"{{"format_version":1,"version":"{version}","url":"{url}",
            "sha256":"{}","max_bytes":1024,"signature":"sig"}}"#,
            Sha256::of_bytes(b"x")
        )
    }

    #[test]
    fn lit_un_manifeste_valide() {
        let manifest = parse_manifest(&json("0.2.0", URL)).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.notes, None);
    }

    #[test]
    fn refuse_un_format_inconnu() {
        let text = json("0.2.0", URL).replace("\"format_version\":1", "\"format_version\":2");
        assert!(matches!(
            parse_manifest(&text),
            Err(UpdateError::UnsupportedFormat { found: 2, .. })
        ));
    }

    #[test]
    fn refuse_json_et_version_invalides() {
        assert!(matches!(
            parse_manifest("pas du json"),
            Err(UpdateError::Manifest(_))
        ));
        assert!(matches!(
            parse_manifest(&json("1.x", URL)),
            Err(UpdateError::BadVersion { .. })
        ));
    }

    #[test]
    fn refuse_les_adresses_dangereuses() {
        for url in [
            "http://github.com/a/b",
            "https://evil.example.com/a",
            "https://github.com@evil.example.com/a",
            "https://github.com:8443/a",
            "https://github.com.evil.example.com/a",
            "ftp://github.com/a",
        ] {
            assert!(
                matches!(
                    parse_manifest(&json("0.2.0", url)),
                    Err(UpdateError::BadUrl(_))
                ),
                "{url} aurait dû être refusée"
            );
        }
        // Les redirections GitHub vivent sur ce domaine, il est autorisé.
        assert!(parse_manifest(&json("0.2.0", "https://objects.githubusercontent.com/x")).is_ok());
    }

    #[test]
    fn refuse_taille_nulle_trop_grande_et_signature_vide() {
        let mut manifest = parse_manifest(&json("0.2.0", URL)).unwrap();
        manifest.max_bytes = 0;
        assert!(matches!(manifest.validate(), Err(UpdateError::BadSize(0))));
        manifest.max_bytes = UPDATE_MAX_BYTES + 1;
        assert!(matches!(manifest.validate(), Err(UpdateError::BadSize(_))));
        manifest.max_bytes = 10;
        manifest.signature = "  ".into();
        assert!(matches!(
            manifest.validate(),
            Err(UpdateError::MissingSignature)
        ));
    }

    #[test]
    fn compare_les_versions() {
        let manifest = parse_manifest(&json("0.2.0", URL)).unwrap();
        let v = |s: &str| parse_version(s).unwrap();
        assert_eq!(
            check(&manifest, &v("0.1.0")).unwrap(),
            UpdateStatus::Available(v("0.2.0"))
        );
        // Égale ou plus ancienne : rien à faire, jamais de retour en arrière.
        assert_eq!(
            check(&manifest, &v("0.2.0")).unwrap(),
            UpdateStatus::UpToDate
        );
        assert_eq!(
            check(&manifest, &v("1.0.0")).unwrap(),
            UpdateStatus::UpToDate
        );
        // Une préversion est plus ancienne que la version finale.
        assert_eq!(
            check(&manifest, &v("0.2.0-beta.1")).unwrap(),
            UpdateStatus::Available(v("0.2.0"))
        );
    }

    #[test]
    fn la_version_courante_se_lit() {
        assert!(current_version().is_ok());
    }

    #[test]
    fn telecharge_et_verifie_un_fichier_signe() {
        let data = b"faux binaire du launcher";
        let (key, signature) = sign(data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), data);
        let path = run(&manifest_for(data, &signature), &key, &dl).unwrap();
        assert_eq!(fs::read(&path).unwrap(), data);
        assert!(path.starts_with(dir.path()));
    }

    #[test]
    fn rejette_une_mauvaise_signature_et_supprime_le_fichier() {
        let data = b"faux binaire du launcher";
        // Signé par une autre clé : le sha256 est bon mais la signature ne l'est pas.
        let (_, other_signature) = sign(data);
        let (key, _) = sign(data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), data);
        let result = run(&manifest_for(data, &other_signature), &key, &dl);
        assert!(matches!(result, Err(UpdateError::SignatureMismatch)));
        assert!(!dl.dir.join(Sha256::of_bytes(data).to_string()).exists());
    }

    #[test]
    fn rejette_un_fichier_modifie_dont_le_sha256_a_ete_recalcule() {
        // Cas d'un serveur compromis : il change le fichier ET le sha256 du manifeste,
        // mais ne peut pas refaire la signature.
        let (key, signature) = sign(b"version authentique");
        let tampered = b"version piegee";
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), tampered);
        let result = run(&manifest_for(tampered, &signature), &key, &dl);
        assert!(matches!(result, Err(UpdateError::SignatureMismatch)));
    }

    #[test]
    fn rejette_un_mauvais_sha256() {
        let data = b"contenu";
        let (key, signature) = sign(data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), b"autre contenu");
        let result = run(&manifest_for(data, &signature), &key, &dl);
        assert!(matches!(
            result,
            Err(UpdateError::Download(DownloadError::HashMismatch { .. }))
        ));
    }

    #[test]
    fn rejette_un_fichier_trop_gros() {
        let data = vec![7u8; 2048];
        let (key, signature) = sign(&data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), &data);
        // `max_bytes` vaut 1024 dans `manifest_for`.
        let result = run(&manifest_for(&data, &signature), &key, &dl);
        assert!(matches!(
            result,
            Err(UpdateError::Download(DownloadError::TooLarge { .. }))
        ));
    }

    #[test]
    fn clef_absente_ou_invalide() {
        let data = b"x";
        let (_, signature) = sign(data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), data);
        let manifest = manifest_for(data, &signature);
        assert!(matches!(
            // Clé vide passée explicitement : la vraie clé est compilée dans `UPDATE_PUBLIC_KEY`.
            run(&manifest, "", &dl),
            Err(UpdateError::PublicKeyNotConfigured)
        ));
        assert!(matches!(
            run(&manifest, "pas une clé", &dl),
            Err(UpdateError::BadPublicKey(_))
        ));
        // Rien n'a été téléchargé dans ces deux cas.
        assert!(!dl.dir.exists());
    }

    #[test]
    fn remplace_l_executable_et_garde_l_ancien_en_old() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("raftup.exe");
        let new_file = dir.path().join("nouveau.bin");
        fs::write(&exe, b"ancien").unwrap();
        fs::write(&new_file, b"nouveau").unwrap();
        apply_update(&exe, &new_file).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"nouveau");
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"ancien");
        assert!(!dir.path().join("raftup.exe.new").exists());
        // Au démarrage suivant, l'ancien est nettoyé.
        cleanup_old(&exe);
        assert!(!old_path(&exe).exists());
        // Deuxième mise à jour avec un `.old` déjà là : ça passe aussi.
        fs::write(old_path(&exe), b"reste").unwrap();
        apply_update(&exe, &new_file).unwrap();
        assert_eq!(fs::read(old_path(&exe)).unwrap(), b"nouveau");
    }

    #[test]
    fn echec_de_mise_a_jour_laisse_l_executable_intact() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("raftup.exe");
        fs::write(&exe, b"ancien").unwrap();
        // Fichier à installer introuvable : rien ne bouge.
        let result = apply_update(&exe, &dir.path().join("absent.bin"));
        assert!(matches!(result, Err(UpdateError::Io { .. })));
        assert_eq!(fs::read(&exe).unwrap(), b"ancien");
        assert!(!old_path(&exe).exists());
    }

    #[test]
    fn signature_illisible() {
        let data = b"x";
        let (key, _) = sign(data);
        let dir = tempfile::tempdir().unwrap();
        let dl = downloader(dir.path(), data);
        let result = run(&manifest_for(data, "n'importe quoi"), &key, &dl);
        assert!(matches!(result, Err(UpdateError::BadSignature(_))));
    }

    #[test]
    fn lit_le_manifeste_via_le_telechargeur() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("update.json");
        fs::write(&file, json("0.3.0", URL)).unwrap();
        let fetcher = DirFetcher::new().with_file(MANIFEST_URL, &file);
        let manifest = fetch_manifest(&fetcher, MANIFEST_URL).unwrap();
        assert_eq!(manifest.version, "0.3.0");

        // Manifeste trop gros.
        fs::write(&file, vec![b' '; (MANIFEST_MAX_BYTES + 10) as usize]).unwrap();
        assert!(matches!(
            fetch_manifest(&fetcher, MANIFEST_URL),
            Err(UpdateError::ManifestTooLarge(_))
        ));
        // Adresse du manifeste hors liste blanche : refusée avant tout accès.
        assert!(matches!(
            fetch_manifest(&fetcher, "https://evil.example.com/update.json"),
            Err(UpdateError::BadUrl(_))
        ));
    }
}
