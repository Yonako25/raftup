//! Launcher relié à un Rafthub (ADR 0036) : fichier `rafthub.toml` posé à côté de
//! l'exécutable, et liens `raftup://hub/<hôte>`.
//!
//! - [`read_bundled`] lit `rafthub.toml` (`format_version = 1`, `hub_url = "…"`), que le
//!   hub ajoute au zip du launcher qu'il sert ;
//! - [`adopt_bundled`] l'enregistre dans la configuration **une seule fois** : quand un
//!   hub est déjà réglé (ou que le fichier a déjà été adopté), le fichier n'est plus lu ;
//! - [`parse_hub_link`] lit un lien `raftup://hub/<hôte>` et rend l'adresse
//!   `https://<hôte>` ; l'interface demande confirmation avant de l'utiliser.
//!
//! Les deux chemins finissent par [`HubUrl::parse`] : mêmes règles que l'adresse tapée à
//! la main (HTTPS, sauf vers ce PC ; pas d'identifiants dans l'adresse).

use std::fs;
use std::io::Read;
use std::path::Path;

use percent_encoding::percent_decode_str;
use serde::Deserialize;
use thiserror::Error;

use crate::config::UserConfig;
use crate::hub_client::HubUrl;

/// Nom du fichier posé à côté de l'exécutable.
pub const BUNDLED_FILE: &str = "rafthub.toml";

/// Seule version connue du format de `rafthub.toml`.
pub const BUNDLED_FORMAT_VERSION: u32 = 1;

/// Taille maximale lue de `rafthub.toml` : il tient en deux lignes.
const MAX_BUNDLED_BYTES: u64 = 4096;

/// Taille maximale d'un lien (comme les liens d'invitation, ADR 0020).
const MAX_LINK_BYTES: usize = 2048;

/// Forme principale : `raftup://hub/<hôte>`.
const LINK_PREFIX_PATH: &str = "raftup://hub/";

/// Forme avec requête : `raftup://hub?host=<hôte>` (et `raftup://hub/?host=…`, la
/// réécriture de Chrome et Edge, comme `join/?` dans l'ADR 0020).
const LINK_PREFIX_QUERY: &str = "raftup://hub?";

/// Pourquoi `rafthub.toml` n'a pas été adopté.
#[derive(Debug, Error)]
pub enum BundledError {
    #[error("lecture de {BUNDLED_FILE} impossible : {0}")]
    Io(#[from] std::io::Error),
    #[error("{BUNDLED_FILE} est trop gros (4 Kio au plus)")]
    TooLarge,
    #[error("{BUNDLED_FILE} illisible : {0}")]
    Syntax(String),
    #[error("{BUNDLED_FILE} en version {0}, ce launcher ne connaît que la version 1 : mettez Raftup à jour")]
    UnsupportedVersion(u32),
    #[error("adresse de {BUNDLED_FILE} refusée : {0}")]
    BadUrl(String),
}

/// Contenu de `rafthub.toml`. Les champs inconnus sont ignorés : un hub plus récent
/// pourra en ajouter (son nom, par exemple) sans casser les launchers déjà livrés.
#[derive(Debug, Deserialize)]
struct BundledFile {
    format_version: u32,
    hub_url: String,
}

/// Lit `<dir>/rafthub.toml`. Fichier absent : `Ok(None)`.
pub fn read_bundled(dir: &Path) -> Result<Option<HubUrl>, BundledError> {
    let path = dir.join(BUNDLED_FILE);
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(BundledError::Io(e)),
    };
    // `take` borne la lecture : un fichier énorme ne remplit pas la mémoire.
    let mut text = String::new();
    file.take(MAX_BUNDLED_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_BUNDLED_BYTES {
        return Err(BundledError::TooLarge);
    }
    let parsed: BundledFile =
        toml::from_str(&text).map_err(|e| BundledError::Syntax(e.message().to_string()))?;
    if parsed.format_version != BUNDLED_FORMAT_VERSION {
        return Err(BundledError::UnsupportedVersion(parsed.format_version));
    }
    let url = HubUrl::parse(&parsed.hub_url).map_err(|e| BundledError::BadUrl(e.to_string()))?;
    Ok(Some(url))
}

/// Ce que [`adopt_bundled`] a fait.
#[derive(Debug)]
pub enum Adoption {
    /// Un hub est déjà réglé, ou le fichier a déjà été adopté : rien n'est lu.
    AlreadySet,
    /// Pas de `rafthub.toml` : on garde l'adresse par défaut (ADR 0035).
    Absent,
    /// Fichier présent mais refusé : on garde l'adresse par défaut.
    Invalid(BundledError),
    /// Adresse adoptée : la configuration a changé, il faut l'enregistrer.
    Adopted(HubUrl),
}

/// Adopte l'adresse de `<exe_dir>/rafthub.toml` si aucun hub n'est réglé.
///
/// Après une adoption, `config.rafthub_file_adopted` vaut vrai : même si le joueur
/// revient plus tard à l'adresse par défaut, le fichier n'est plus relu.
pub fn adopt_bundled(config: &mut UserConfig, exe_dir: &Path) -> Adoption {
    if config.hub_url.is_some() || config.rafthub_file_adopted {
        return Adoption::AlreadySet;
    }
    match read_bundled(exe_dir) {
        Ok(None) => Adoption::Absent,
        Err(e) => Adoption::Invalid(e),
        Ok(Some(url)) => {
            config.hub_url = Some(url.to_string());
            config.rafthub_file_adopted = true;
            Adoption::Adopted(url)
        }
    }
}

/// Erreurs d'analyse d'un lien `raftup://hub/…`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HubLinkError {
    #[error("le lien est trop long (2 048 octets au plus)")]
    TooLong,
    #[error("ce n'est pas un lien Rafthub")]
    NotAHubLink,
    #[error("le lien ne contient pas d'hôte")]
    MissingHost,
    #[error("hôte refusé : {0}")]
    BadHost(&'static str),
    #[error("le paramètre « {0} » apparaît plusieurs fois")]
    Duplicate(String),
}

/// Vrai si le texte ressemble à un lien `raftup://hub…` : c'est alors [`parse_hub_link`]
/// qui doit le lire, et non l'analyse des liens d'invitation.
pub fn is_hub_link(text: &str) -> bool {
    let text = text.trim();
    text.starts_with(LINK_PREFIX_PATH) || text.starts_with(LINK_PREFIX_QUERY)
}

/// Analyse un lien Rafthub et rend l'adresse `https://<hôte>`.
///
/// Formes acceptées :
/// - `raftup://hub/<hôte>` (forme principale), avec un `/` final toléré ;
/// - `raftup://hub?host=<hôte>` et `raftup://hub/?host=<hôte>` (paramètres inconnus
///   ignorés, `host` en double refusé).
///
/// L'hôte est un nom ASCII (`[A-Za-z0-9.-]`), éventuellement suivi de `:port` ; ni
/// chemin, ni `user@`, ni IPv6, ni nom accentué. Il est remis en minuscules.
pub fn parse_hub_link(text: &str) -> Result<HubUrl, HubLinkError> {
    let text = text.trim();
    if text.len() > MAX_LINK_BYTES {
        return Err(HubLinkError::TooLong);
    }
    let host = if let Some(query) = text
        .strip_prefix(LINK_PREFIX_QUERY)
        .or_else(|| text.strip_prefix("raftup://hub/?"))
    {
        host_from_query(query)?
    } else if let Some(rest) = text.strip_prefix(LINK_PREFIX_PATH) {
        // Un seul `/` final toléré (certains navigateurs l'ajoutent).
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['?', '#']) {
            return Err(HubLinkError::BadHost("requête ou « # » après l'hôte"));
        }
        rest.to_string()
    } else {
        return Err(HubLinkError::NotAHubLink);
    };
    let host = check_host(&host)?;
    // Toujours en HTTPS : `parse` ne devrait plus rien refuser, mais on garde sa
    // validation comme dernier filet plutôt que de fabriquer un `HubUrl` à la main.
    HubUrl::parse(&format!("https://{host}")).map_err(|_| HubLinkError::BadHost("adresse refusée"))
}

/// Valeur du paramètre `host` d'une requête (`%XX` décodé).
fn host_from_query(query: &str) -> Result<String, HubLinkError> {
    let mut host = None;
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let Some((key, raw)) = part.split_once('=') else {
            continue;
        };
        if key != "host" {
            continue;
        }
        if host.is_some() {
            return Err(HubLinkError::Duplicate(key.to_string()));
        }
        let decoded = percent_decode_str(raw)
            .decode_utf8()
            .map_err(|_| HubLinkError::BadHost("encodage invalide"))?;
        host = Some(decoded.into_owned());
    }
    host.ok_or(HubLinkError::MissingHost)
}

/// Vérifie `nom[:port]` et le rend en minuscules.
fn check_host(text: &str) -> Result<String, HubLinkError> {
    if text.is_empty() {
        return Err(HubLinkError::MissingHost);
    }
    if text.contains('@') {
        return Err(HubLinkError::BadHost("identifiants « user@ » interdits"));
    }
    if text.contains(['/', '\\']) {
        return Err(HubLinkError::BadHost("chemin interdit"));
    }
    let (name, port) = match text.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (text, None),
    };
    if name.is_empty() {
        return Err(HubLinkError::MissingHost);
    }
    if name.len() > 253 {
        return Err(HubLinkError::BadHost("hôte trop long"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err(HubLinkError::BadHost(
            "caractères interdits (lettres ASCII, chiffres, « . » et « - » seulement)",
        ));
    }
    if name.starts_with(['.', '-']) || name.ends_with(['.', '-']) || name.contains("..") {
        return Err(HubLinkError::BadHost("hôte mal formé"));
    }
    let name = name.to_ascii_lowercase();
    match port {
        None => Ok(name),
        Some(port) => {
            let valid = !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|p| p >= 1);
            if !valid {
                return Err(HubLinkError::BadHost("port invalide (1 à 65535)"));
            }
            Ok(format!("{name}:{port}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, text: &str) {
        fs::write(dir.join(BUNDLED_FILE), text).unwrap();
    }

    #[test]
    fn fichier_valide_adopte() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "format_version = 1\nhub_url = \"https://hub.exemple.org/\"\nnom = \"Chez Mimi\"\n",
        );
        let mut config = UserConfig::default();
        let adoption = adopt_bundled(&mut config, dir.path());
        assert!(
            matches!(&adoption, Adoption::Adopted(url) if url.as_str() == "https://hub.exemple.org")
        );
        assert_eq!(config.hub_url.as_deref(), Some("https://hub.exemple.org"));
        assert!(config.rafthub_file_adopted);
    }

    #[test]
    fn local_en_http_accepte() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "format_version = 1\nhub_url = \"http://127.0.0.1:8787\"\n",
        );
        let url = read_bundled(dir.path()).unwrap().unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8787");
    }

    #[test]
    fn fichier_absent() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = UserConfig::default();
        assert!(matches!(
            adopt_bundled(&mut config, dir.path()),
            Adoption::Absent
        ));
        assert_eq!(config, UserConfig::default());
    }

    #[test]
    fn fichiers_invalides_refuses() {
        let cases = [
            // HTTP vers un autre PC : le mot de passe passerait en clair.
            (
                "format_version = 1\nhub_url = \"http://hub.exemple.org\"\n",
                "BadUrl",
            ),
            // Identifiants dans l'adresse.
            (
                "format_version = 1\nhub_url = \"https://a:b@hub.exemple.org\"\n",
                "BadUrl",
            ),
            (
                "format_version = 2\nhub_url = \"https://hub.exemple.org\"\n",
                "UnsupportedVersion",
            ),
            ("hub_url = \"https://hub.exemple.org\"\n", "Syntax"),
            ("format_version = 1\n", "Syntax"),
            ("ceci n'est pas du toml", "Syntax"),
        ];
        for (text, expected) in cases {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), text);
            let mut config = UserConfig::default();
            let adoption = adopt_bundled(&mut config, dir.path());
            let Adoption::Invalid(error) = adoption else {
                panic!("{text:?} aurait dû être refusé : {adoption:?}");
            };
            assert!(
                format!("{error:?}").starts_with(expected),
                "{text:?} : {error:?}"
            );
            // Rien n'a changé dans la configuration.
            assert_eq!(config, UserConfig::default());
        }
    }

    #[test]
    fn fichier_trop_gros_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "format_version = 1\nhub_url = \"https://hub.exemple.org\"\n# {}\n",
            "x".repeat(5000)
        );
        write(dir.path(), &text);
        assert!(matches!(
            read_bundled(dir.path()),
            Err(BundledError::TooLarge)
        ));
    }

    #[test]
    fn deja_regle_pas_relu() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "format_version = 1\nhub_url = \"https://autre.org\"\n",
        );
        // Un hub choisi à la main gagne.
        let mut config = UserConfig {
            hub_url: Some("https://mien.org".to_string()),
            ..UserConfig::default()
        };
        assert!(matches!(
            adopt_bundled(&mut config, dir.path()),
            Adoption::AlreadySet
        ));
        assert_eq!(config.hub_url.as_deref(), Some("https://mien.org"));
        // Fichier déjà adopté puis adresse remise par défaut : on ne le relit pas.
        let mut config = UserConfig {
            rafthub_file_adopted: true,
            ..UserConfig::default()
        };
        assert!(matches!(
            adopt_bundled(&mut config, dir.path()),
            Adoption::AlreadySet
        ));
        assert_eq!(config.hub_url, None);
    }

    #[test]
    fn liens_valides() {
        let cases = [
            ("raftup://hub/hub.exemple.org", "https://hub.exemple.org"),
            ("raftup://hub/hub.exemple.org/", "https://hub.exemple.org"),
            (
                "  raftup://hub/Hub.Exemple.ORG  ",
                "https://hub.exemple.org",
            ),
            (
                "raftup://hub/hub.exemple.org:8443",
                "https://hub.exemple.org:8443",
            ),
            (
                "raftup://hub?host=hub.exemple.org",
                "https://hub.exemple.org",
            ),
            (
                "raftup://hub/?host=hub.exemple.org",
                "https://hub.exemple.org",
            ),
            (
                "raftup://hub/?v=1&host=hub.exemple.org&x=y",
                "https://hub.exemple.org",
            ),
            (
                "raftup://hub/?host=hub.exemple.org%3A8443",
                "https://hub.exemple.org:8443",
            ),
        ];
        for (link, expected) in cases {
            assert!(is_hub_link(link), "{link}");
            let url = parse_hub_link(link).unwrap_or_else(|e| panic!("{link} : {e}"));
            assert_eq!(url.as_str(), expected, "{link}");
        }
    }

    #[test]
    fn liens_refuses() {
        use HubLinkError::*;
        let long = format!("raftup://hub/{}.org", "a".repeat(2100));
        let cases: Vec<(&str, HubLinkError)> = vec![
            ("raftup://hub/", MissingHost),
            ("raftup://hub/?v=1", MissingHost),
            ("raftup://hub/:443", MissingHost),
            ("raftup://hub", NotAHubLink),
            ("raftup://join?v=1&addr=ws://h", NotAHubLink),
            ("https://hub.exemple.org", NotAHubLink),
            ("RAFTUP://hub/hub.exemple.org", NotAHubLink),
            (
                "raftup://hub/hub.exemple.org/admin",
                BadHost("chemin interdit"),
            ),
            (
                "raftup://hub/moi@hub.exemple.org",
                BadHost("identifiants « user@ » interdits"),
            ),
            (
                "raftup://hub/hub.exemple.org?x=1",
                BadHost("requête ou « # » après l'hôte"),
            ),
            (
                "raftup://hub/hub.exemple.org:0",
                BadHost("port invalide (1 à 65535)"),
            ),
            (
                "raftup://hub/hub.exemple.org:99999",
                BadHost("port invalide (1 à 65535)"),
            ),
            (
                "raftup://hub/hub.exemple.org:",
                BadHost("port invalide (1 à 65535)"),
            ),
            ("raftup://hub/-hub.org", BadHost("hôte mal formé")),
            ("raftup://hub/hub..org", BadHost("hôte mal formé")),
            (
                "raftup://hub/[::1]",
                BadHost("caractères interdits (lettres ASCII, chiffres, « . » et « - » seulement)"),
            ),
            (
                "raftup://hub/hüb.org",
                BadHost("caractères interdits (lettres ASCII, chiffres, « . » et « - » seulement)"),
            ),
            (
                "raftup://hub/?host=a.org&host=b.org",
                Duplicate("host".to_string()),
            ),
            ("raftup://hub/?host=%FF", BadHost("encodage invalide")),
            (
                "raftup://hub/?host=a.org%2Fadmin",
                BadHost("chemin interdit"),
            ),
        ];
        for (link, expected) in cases {
            assert_eq!(parse_hub_link(link), Err(expected), "{link}");
        }
        assert_eq!(parse_hub_link(&long), Err(TooLong));
        // Les liens d'invitation ne sont pas pris pour des liens Rafthub.
        assert!(!is_hub_link("raftup://join/?v=1&addr=ws://h"));
    }
}
