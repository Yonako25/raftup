//! Liens d'invitation `raftup://join?…` : analyse et fabrication, en fonctions pures (ADR 0020).
//!
//! Un lien ne contient jamais de mot de passe. Tout ce qui vient d'un lien est
//! traité comme non fiable : on valide strictement avant de rendre une valeur.

use std::net::Ipv6Addr;

use client_core::Fingerprint;
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use thiserror::Error;

/// Version du format de lien que ce code sait lire et écrire.
pub const FORMAT_VERSION: u32 = 1;

/// Taille maximale d'un lien, en octets.
const MAX_LINK_BYTES: usize = 2048;
/// Port par défaut d'un serveur Archipelago.
const DEFAULT_PORT: u16 = 38281;
/// Longueur maximale d'un nom de slot, en caractères.
/// Source : `Generate.py` d'Archipelago 0.6.8, ligne 387 (`new_name.strip()[:16].strip()`).
const MAX_SLOT_CHARS: usize = 16;
/// Longueur maximale du nom d'un jeu, en caractères.
const MAX_GAME_CHARS: usize = 64;
/// Préfixe exact d'un lien (schéma, action et début de la requête).
const LINK_PREFIX: &str = "raftup://join?";
/// Même préfixe avec un `/` avant le `?` : Chrome et Edge réécrivent ainsi le lien
/// (`raftup://join/?…`) avant de le passer à Windows, puis à Raftup.
const LINK_PREFIX_SLASH: &str = "raftup://join/?";

/// Caractères encodés en `%XX` dans une valeur de paramètre. Les caractères non ASCII
/// sont toujours encodés par `utf8_percent_encode`. On laisse `:`, `/`, `[` et `]`
/// lisibles : ils sont permis dans une requête et rendent `addr` plus clair.
const VALUE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'`');

/// Erreurs d'analyse ou de fabrication d'un lien.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("le lien est trop long (2 048 octets au plus)")]
    TooLong,
    #[error("ce n'est pas un lien d'invitation Raftup")]
    NotAnInvite,
    #[error("version de lien inconnue ({0}) : mettez Raftup à jour")]
    UnsupportedVersion(u32),
    #[error("le lien ne contient pas d'adresse de serveur")]
    MissingAddress,
    #[error("adresse de serveur refusée : {0}")]
    BadAddress(&'static str),
    #[error("nom de joueur refusé : {0}")]
    BadSlot(&'static str),
    #[error("nom de jeu refusé")]
    BadGame,
    #[error("le lien contient un encodage invalide")]
    BadEncoding,
    #[error("le paramètre « {0} » apparaît plusieurs fois")]
    Duplicate(String),
    #[error("empreinte du certificat refusée (64 chiffres hexadécimaux attendus)")]
    BadFingerprint,
}

/// Adresse d'un serveur Archipelago (`ws://` ou `wss://`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    pub secure: bool,
    /// Nom ASCII ou adresse IPv6 entre crochets, tel qu'écrit.
    pub host: String,
    pub port: u16,
}

impl ServerAddress {
    /// Analyse `ws://hôte[:port]` ou `wss://hôte[:port]` (port par défaut : 38281).
    pub fn parse(text: &str) -> Result<ServerAddress, InviteError> {
        let (secure, rest) = if let Some(rest) = text.strip_prefix("wss://") {
            (true, rest)
        } else if let Some(rest) = text.strip_prefix("ws://") {
            (false, rest)
        } else {
            return Err(InviteError::BadAddress("schéma ws:// ou wss:// attendu"));
        };
        if rest.contains('@') {
            return Err(InviteError::BadAddress("identifiant « user@ » interdit"));
        }
        if rest.contains(['/', '?', '#', '\\']) {
            return Err(InviteError::BadAddress("chemin ou requête interdits"));
        }

        let (host, port_text) = if rest.starts_with('[') {
            // IPv6 entre crochets : le port éventuel suit le `]`.
            let end = rest
                .find(']')
                .ok_or(InviteError::BadAddress("crochet fermant manquant"))?;
            let inner = &rest[1..end];
            if inner.parse::<Ipv6Addr>().is_err() {
                return Err(InviteError::BadAddress("adresse IPv6 invalide"));
            }
            let after = &rest[end + 1..];
            let port_text = match after {
                "" => None,
                _ => Some(
                    after
                        .strip_prefix(':')
                        .ok_or(InviteError::BadAddress("texte après l'adresse IPv6"))?,
                ),
            };
            (&rest[..=end], port_text)
        } else {
            match rest.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (rest, None),
            }
        };

        if !host.starts_with('[') {
            check_hostname(host)?;
        }
        let port = match port_text {
            None => DEFAULT_PORT,
            Some(p) => parse_port(p)?,
        };
        Ok(ServerAddress {
            secure,
            host: host.to_string(),
            port,
        })
    }

    /// Forme texte `ws(s)://hôte:port` (le port est toujours écrit).
    pub fn to_url(&self) -> String {
        let scheme = if self.secure { "wss" } else { "ws" };
        format!("{scheme}://{}:{}", self.host, self.port)
    }

    /// Forme `hôte:port`, sans schéma : celle à donner à un mod de jeu non modifié
    /// quand le serveur a un certificat généré par Raftup (ADR 0043). Sans schéma, les
    /// clients officiels essaient `wss://` puis retombent sur `ws://`.
    pub fn without_scheme(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Vérifie un nom d'hôte : ASCII `[A-Za-z0-9.-]`, sans IDN.
fn check_hostname(host: &str) -> Result<(), InviteError> {
    if host.is_empty() {
        return Err(InviteError::BadAddress("hôte vide"));
    }
    if host.len() > 253 {
        return Err(InviteError::BadAddress("hôte trop long"));
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    {
        return Err(InviteError::BadAddress(
            "caractères d'hôte interdits (ASCII, chiffres, « . » et « - » seulement)",
        ));
    }
    if host.starts_with(['.', '-']) || host.ends_with(['.', '-']) || host.contains("..") {
        return Err(InviteError::BadAddress("hôte mal formé"));
    }
    Ok(())
}

/// Port de 1 à 65535, chiffres seulement.
fn parse_port(text: &str) -> Result<u16, InviteError> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(InviteError::BadAddress("port invalide"));
    }
    match text.parse::<u16>() {
        Ok(p) if p >= 1 => Ok(p),
        _ => Err(InviteError::BadAddress("port hors de 1 à 65535")),
    }
}

/// Informations contenues dans un lien d'invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub address: ServerAddress,
    pub slot: Option<String>,
    pub game: Option<String>,
    /// Empreinte SHA-256 du certificat généré par Raftup (paramètre `fp`, ADR 0043).
    /// Un ancien launcher l'ignore (paramètre inconnu), le lien reste en `v=1`.
    pub fingerprint: Option<Fingerprint>,
}

impl Invite {
    /// L'adresse à donner au mod du jeu : sans schéma si le serveur a un certificat
    /// généré par Raftup (le mod retombera sur `ws`), telle quelle sinon.
    pub fn address_for_game(&self) -> String {
        if self.fingerprint.is_some() {
            self.address.without_scheme()
        } else {
            self.address.to_url()
        }
    }
}

/// Analyse un lien `raftup://join?v=1&addr=…[&slot=…][&game=…][&fp=…]` (espaces autour ignorés).
pub fn parse_invite(text: &str) -> Result<Invite, InviteError> {
    let text = text.trim();
    if text.len() > MAX_LINK_BYTES {
        return Err(InviteError::TooLong);
    }
    let query = text
        .strip_prefix(LINK_PREFIX)
        .or_else(|| text.strip_prefix(LINK_PREFIX_SLASH))
        .ok_or(InviteError::NotAnInvite)?;

    let (mut v, mut addr, mut slot, mut game, mut fp) = (None, None, None, None, None);
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (key, raw) = part.split_once('=').ok_or(InviteError::NotAnInvite)?;
        // Les paramètres inconnus sont ignorés ; les connus ne peuvent pas être doublés.
        let target = match key {
            "v" => &mut v,
            "addr" => &mut addr,
            "slot" => &mut slot,
            "game" => &mut game,
            "fp" => &mut fp,
            _ => continue,
        };
        if target.is_some() {
            return Err(InviteError::Duplicate(key.to_string()));
        }
        // Pas de « + » = espace : seul %XX est décodé.
        let decoded = percent_decode_str(raw)
            .decode_utf8()
            .map_err(|_| InviteError::BadEncoding)?;
        *target = Some(decoded.into_owned());
    }

    let version: u32 = v
        .ok_or(InviteError::NotAnInvite)?
        .parse()
        .map_err(|_| InviteError::NotAnInvite)?;
    if version != FORMAT_VERSION {
        return Err(InviteError::UnsupportedVersion(version));
    }
    let address = ServerAddress::parse(&addr.ok_or(InviteError::MissingAddress)?)?;
    if let Some(s) = &slot {
        check_name(s, MAX_SLOT_CHARS).map_err(InviteError::BadSlot)?;
    }
    if let Some(g) = &game {
        check_name(g, MAX_GAME_CHARS).map_err(|_| InviteError::BadGame)?;
    }
    let fingerprint = match &fp {
        None => None,
        Some(text) => Some(Fingerprint::parse(text).ok_or(InviteError::BadFingerprint)?),
    };
    // L'empreinte n'a de sens qu'en `wss` : un `ws://` avec `fp` est incohérent.
    if fingerprint.is_some() && !address.secure {
        return Err(InviteError::BadFingerprint);
    }
    Ok(Invite {
        address,
        slot,
        game,
        fingerprint,
    })
}

/// Fabrique le lien d'une invitation, après les mêmes validations que `parse_invite`.
/// L'ordre des paramètres est fixe : `v`, `addr`, `slot`, `game`, `fp`.
pub fn make_invite(invite: &Invite) -> Result<String, InviteError> {
    let url = invite.address.to_url();
    // Réanalyser l'adresse garantit qu'on ne fabrique jamais un lien que l'on refuserait.
    if ServerAddress::parse(&url)? != invite.address {
        return Err(InviteError::BadAddress("adresse incohérente"));
    }
    let mut link = format!(
        "{LINK_PREFIX}v={FORMAT_VERSION}&addr={}",
        utf8_percent_encode(&url, VALUE_SET)
    );
    if let Some(s) = &invite.slot {
        check_name(s, MAX_SLOT_CHARS).map_err(InviteError::BadSlot)?;
        link.push_str(&format!("&slot={}", utf8_percent_encode(s, VALUE_SET)));
    }
    if let Some(g) = &invite.game {
        check_name(g, MAX_GAME_CHARS).map_err(|_| InviteError::BadGame)?;
        link.push_str(&format!("&game={}", utf8_percent_encode(g, VALUE_SET)));
    }
    if let Some(fp) = &invite.fingerprint {
        if !invite.address.secure {
            return Err(InviteError::BadFingerprint);
        }
        // 64 chiffres hexadécimaux : rien à encoder.
        link.push_str(&format!("&fp={}", fp.to_hex()));
    }
    if link.len() > MAX_LINK_BYTES {
        return Err(InviteError::TooLong);
    }
    Ok(link)
}

/// Règles communes au slot et au nom de jeu.
fn check_name(name: &str, max_chars: usize) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("vide");
    }
    if name.chars().count() > max_chars {
        return Err("trop long");
    }
    if name.starts_with('-') {
        return Err("ne doit pas commencer par « - »");
    }
    if name.starts_with(char::is_whitespace) || name.ends_with(char::is_whitespace) {
        return Err("espace au début ou à la fin");
    }
    if name.chars().any(|c| c.is_control() || is_format_char(c)) {
        return Err("caractère de contrôle ou de sens d'écriture interdit");
    }
    Ok(())
}

/// Caractères invisibles qui changent l'affichage (sens d'écriture, largeur nulle…).
fn is_format_char(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{FEFF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(secure: bool, host: &str, port: u16) -> ServerAddress {
        ServerAddress {
            secure,
            host: host.to_string(),
            port,
        }
    }

    fn inv(a: ServerAddress, slot: Option<&str>, game: Option<&str>) -> Invite {
        Invite {
            fingerprint: None,
            address: a,
            slot: slot.map(String::from),
            game: game.map(String::from),
        }
    }

    #[test]
    fn parse_table() {
        let long_slot = format!("{LINK_PREFIX}v=1&addr=ws://h&slot={}", "a".repeat(17));
        let long_game = format!("{LINK_PREFIX}v=1&addr=ws://h&game={}", "a".repeat(65));
        let huge = format!("{LINK_PREFIX}v=1&addr=ws://h&x={}", "a".repeat(2100));
        let ok = |a, s: Option<&str>, g: Option<&str>| Ok(inv(a, s, g));

        let cases: Vec<(String, Result<Invite, InviteError>)> = vec![
            // Liens valides
            (
                "raftup://join?v=1&addr=wss://exemple.fr:38281".into(),
                ok(addr(true, "exemple.fr", 38281), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://exemple.fr".into(),
                ok(addr(false, "exemple.fr", 38281), None, None),
            ),
            (
                "  raftup://join?v=1&addr=ws://h:1234&slot=Ana  \n".into(),
                ok(addr(false, "h", 1234), Some("Ana"), None),
            ),
            (
                "raftup://join?v=1&addr=ws%3A%2F%2F10.0.0.1%3A5&slot=Ana%20Maria&game=Hollow%20Knight".into(),
                ok(addr(false, "10.0.0.1", 5), Some("Ana Maria"), Some("Hollow Knight")),
            ),
            (
                "raftup://join?v=1&addr=ws://[::1]:4000".into(),
                ok(addr(false, "[::1]", 4000), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://[2001:db8::1]".into(),
                ok(addr(false, "[2001:db8::1]", 38281), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://h:65535".into(),
                ok(addr(false, "h", 65535), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://h:1".into(),
                ok(addr(false, "h", 1), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://h&inconnu=zzz&slot=Bob".into(),
                ok(addr(false, "h", 38281), Some("Bob"), None),
            ),
            (
                "raftup://join?game=G&slot=Zoé&addr=ws://h&v=1".into(),
                ok(addr(false, "h", 38281), Some("Zoé"), Some("G")),
            ),
            (
                "raftup://join?v=1&addr=ws://h&slot=a-b".into(),
                ok(addr(false, "h", 38281), Some("a-b"), None),
            ),
            (
                format!("{LINK_PREFIX}v=1&addr=ws://h&slot={}", "a".repeat(16)),
                ok(addr(false, "h", 38281), Some(&"a".repeat(16)), None),
            ),
            // Forme réécrite par Chrome et Edge (bug vu le 18 novembre 2026).
            (
                "raftup://join/?v=1&addr=ws://localhost:38281&slot=Raftup1&game=Slime%20Rancher"
                    .into(),
                ok(
                    addr(false, "localhost", 38281),
                    Some("Raftup1"),
                    Some("Slime Rancher"),
                ),
            ),
            // Pas un lien
            ("".into(), Err(InviteError::NotAnInvite)),
            ("raftup://join//?v=1&addr=ws://h".into(), Err(InviteError::NotAnInvite)),
            ("https://exemple.fr".into(), Err(InviteError::NotAnInvite)),
            ("RAFTUP://join?v=1&addr=ws://h".into(), Err(InviteError::NotAnInvite)),
            ("raftup://other?v=1&addr=ws://h".into(), Err(InviteError::NotAnInvite)),
            ("raftup://join?addr=ws://h".into(), Err(InviteError::NotAnInvite)),
            ("raftup://join?v=1&addr".into(), Err(InviteError::NotAnInvite)),
            // Version, adresse manquante
            ("raftup://join?v=2&addr=ws://h".into(), Err(InviteError::UnsupportedVersion(2))),
            ("raftup://join?v=0&addr=ws://h".into(), Err(InviteError::UnsupportedVersion(0))),
            ("raftup://join?v=1".into(), Err(InviteError::MissingAddress)),
            // Adresses piégées
            (
                "raftup://join?v=1&addr=http://h".into(),
                Err(InviteError::BadAddress("schéma ws:// ou wss:// attendu")),
            ),
            (
                "raftup://join?v=1&addr=h:38281".into(),
                Err(InviteError::BadAddress("schéma ws:// ou wss:// attendu")),
            ),
            (
                "raftup://join?v=1&addr=ws://user@h".into(),
                Err(InviteError::BadAddress("identifiant « user@ » interdit")),
            ),
            (
                "raftup://join?v=1&addr=ws://u:p%40ss@h".into(),
                Err(InviteError::BadAddress("identifiant « user@ » interdit")),
            ),
            (
                "raftup://join?v=1&addr=ws://h/chemin".into(),
                Err(InviteError::BadAddress("chemin ou requête interdits")),
            ),
            (
                "raftup://join?v=1&addr=ws://h%3Fa%3D1".into(),
                Err(InviteError::BadAddress("chemin ou requête interdits")),
            ),
            (
                "raftup://join?v=1&addr=ws://h:0".into(),
                Err(InviteError::BadAddress("port hors de 1 à 65535")),
            ),
            (
                "raftup://join?v=1&addr=ws://h:65536".into(),
                Err(InviteError::BadAddress("port hors de 1 à 65535")),
            ),
            (
                "raftup://join?v=1&addr=ws://h:".into(),
                Err(InviteError::BadAddress("port invalide")),
            ),
            (
                "raftup://join?v=1&addr=ws://h:-1".into(),
                Err(InviteError::BadAddress("port invalide")),
            ),
            (
                "raftup://join?v=1&addr=ws://h:+5".into(),
                Err(InviteError::BadAddress("port invalide")),
            ),
            (
                "raftup://join?v=1&addr=ws://[::1".into(),
                Err(InviteError::BadAddress("crochet fermant manquant")),
            ),
            (
                "raftup://join?v=1&addr=ws://[nope]:5".into(),
                Err(InviteError::BadAddress("adresse IPv6 invalide")),
            ),
            (
                "raftup://join?v=1&addr=ws://::1".into(),
                Err(InviteError::BadAddress(
                    "caractères d'hôte interdits (ASCII, chiffres, « . » et « - » seulement)",
                )),
            ),
            (
                "raftup://join?v=1&addr=ws://b%C3%BCcher.de".into(),
                Err(InviteError::BadAddress(
                    "caractères d'hôte interdits (ASCII, chiffres, « . » et « - » seulement)",
                )),
            ),
            (
                "raftup://join?v=1&addr=ws://xn--bcher-kva.de".into(),
                ok(addr(false, "xn--bcher-kva.de", 38281), None, None),
            ),
            (
                "raftup://join?v=1&addr=ws://".into(),
                Err(InviteError::BadAddress("hôte vide")),
            ),
            (
                "raftup://join?v=1&addr=ws://a..b".into(),
                Err(InviteError::BadAddress("hôte mal formé")),
            ),
            // Slot et jeu piégés
            (
                "raftup://join?v=1&addr=ws://h&slot=-Ana".into(),
                Err(InviteError::BadSlot("ne doit pas commencer par « - »")),
            ),
            (
                "raftup://join?v=1&addr=ws://h&slot=Ana%E2%80%AEfdp".into(),
                Err(InviteError::BadSlot(
                    "caractère de contrôle ou de sens d'écriture interdit",
                )),
            ),
            (
                "raftup://join?v=1&addr=ws://h&slot=A%00B".into(),
                Err(InviteError::BadSlot(
                    "caractère de contrôle ou de sens d'écriture interdit",
                )),
            ),
            ("raftup://join?v=1&addr=ws://h&slot=".into(), Err(InviteError::BadSlot("vide"))),
            (
                "raftup://join?v=1&addr=ws://h&slot=%20Ana".into(),
                Err(InviteError::BadSlot("espace au début ou à la fin")),
            ),
            (long_slot, Err(InviteError::BadSlot("trop long"))),
            (long_game, Err(InviteError::BadGame)),
            ("raftup://join?v=1&addr=ws://h&game=-x".into(), Err(InviteError::BadGame)),
            ("raftup://join?v=1&addr=ws://h&game=A%E2%80%AEB".into(), Err(InviteError::BadGame)),
            // Encodage, doublons, taille
            ("raftup://join?v=1&addr=ws://h&slot=%FF".into(), Err(InviteError::BadEncoding)),
            (
                "raftup://join?v=1&addr=ws://h&addr=ws://autre".into(),
                Err(InviteError::Duplicate("addr".into())),
            ),
            (
                "raftup://join?v=1&v=1&addr=ws://h".into(),
                Err(InviteError::Duplicate("v".into())),
            ),
            (
                "raftup://join?v=1&addr=ws://h&slot=A&slot=B".into(),
                Err(InviteError::Duplicate("slot".into())),
            ),
            (huge, Err(InviteError::TooLong)),
        ];

        assert!(cases.len() >= 30);
        for (input, expected) in cases {
            assert_eq!(parse_invite(&input), expected, "entrée : {input}");
        }
    }

    #[test]
    fn round_trip() {
        let samples = vec![
            inv(addr(true, "exemple.fr", 38281), None, None),
            inv(
                addr(false, "10.0.0.1", 1),
                Some("Ana Maria"),
                Some("Hollow Knight"),
            ),
            inv(addr(false, "[::1]", 65535), Some("Zoé"), None),
            inv(
                addr(true, "h", 5),
                Some("a&b=c?d#e+f%g"),
                Some("Ça va ? \"oui\""),
            ),
            inv(addr(true, "h", 5), Some("日本語名"), Some("ゲーム")),
        ];
        for x in samples {
            let link = make_invite(&x).expect("lien fabriqué");
            assert_eq!(parse_invite(&link).as_ref(), Ok(&x), "lien : {link}");
        }
    }

    #[test]
    fn make_orders_parameters() {
        let x = inv(addr(true, "h", 5), Some("Ana"), Some("G"));
        assert_eq!(
            make_invite(&x).as_deref(),
            Ok("raftup://join?v=1&addr=wss://h:5&slot=Ana&game=G")
        );
    }

    #[test]
    fn make_refuses_invalid() {
        let bad_host = inv(addr(false, "h@x", 5), None, None);
        assert!(matches!(
            make_invite(&bad_host),
            Err(InviteError::BadAddress(_))
        ));
        let port0 = inv(addr(false, "h", 0), None, None);
        assert!(matches!(
            make_invite(&port0),
            Err(InviteError::BadAddress(_))
        ));
        let dash = inv(addr(false, "h", 5), Some("-x"), None);
        assert!(matches!(make_invite(&dash), Err(InviteError::BadSlot(_))));
        let long = inv(addr(false, "h", 5), None, Some(&"g".repeat(65)));
        assert_eq!(make_invite(&long), Err(InviteError::BadGame));
    }

    #[test]
    fn address_to_url_and_default_port() {
        let a = ServerAddress::parse("wss://h").expect("adresse valide");
        assert_eq!(a.to_url(), "wss://h:38281");
    }

    #[test]
    fn empreinte_du_certificat_dans_le_lien() {
        let fp = Fingerprint::of_der(b"certificat de test");
        let mut invite = inv(addr(true, "exemple.fr", 38281), Some("Ioka"), None);
        invite.fingerprint = Some(fp);
        let link = make_invite(&invite).expect("lien fabriqué");
        assert!(link.ends_with(&format!("&fp={}", fp.to_hex())), "{link}");
        let back = parse_invite(&link).expect("lien relu");
        assert_eq!(back, invite);
        // Le mod du jeu reçoit l'adresse sans schéma : il essaiera wss puis ws.
        assert_eq!(back.address_for_game(), "exemple.fr:38281");
        // Sans empreinte : l'adresse telle quelle.
        let plain = inv(addr(false, "h", 5), None, None);
        assert_eq!(plain.address_for_game(), "ws://h:5");

        // En majuscules, l'empreinte est acceptée et relue en minuscules.
        let upper = link.replace(&fp.to_hex(), &fp.to_hex().to_uppercase());
        assert_eq!(parse_invite(&upper).expect("relu").fingerprint, Some(fp));
        // Empreinte trop courte, ou en clair : refusée.
        let short = link.replace(&fp.to_hex(), &fp.to_hex()[..10]);
        assert_eq!(parse_invite(&short), Err(InviteError::BadFingerprint));
        let clear = "raftup://join?v=1&addr=ws://h:5&fp=".to_string() + &fp.to_hex();
        assert_eq!(parse_invite(&clear), Err(InviteError::BadFingerprint));
        let doubled = format!("{link}&fp={}", fp.to_hex());
        assert_eq!(
            parse_invite(&doubled),
            Err(InviteError::Duplicate("fp".to_string()))
        );
        // Fabriquer un lien `ws://` avec empreinte est refusé aussi.
        let mut bad = inv(addr(false, "h", 5), None, None);
        bad.fingerprint = Some(fp);
        assert_eq!(make_invite(&bad), Err(InviteError::BadFingerprint));
    }
}
