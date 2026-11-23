//! Parties hébergées par Rafthub (ADR 0052, étape 2, côté launcher).
//!
//! La seed est générée sur le PC de l'hôte comme pour une partie en self-host, puis
//! envoyée au hub (`POST /v1/games`) qui fait tourner le serveur de jeu. Ce module
//! redéfinit la forme du JSON des routes `/v1/games` (le hub est en AGPL, ADR 0003 :
//! aucun type n'est partagé avec sa crate), et sait en faire un lien d'invitation.
//!
//! Le hub rend l'adresse **sans schéma** (`hôte:port`) et l'empreinte de son
//! certificat (`fingerprint`) : c'est exactement un serveur à certificat généré, comme
//! en self-host (ADR 0043). Les liens portent donc `wss://hôte:port` et `fp=…`, et le
//! mod du jeu reçoit `hôte:port` (il essaie `wss` puis retombe sur `ws`).

use std::fmt;
use std::path::Path;

use base64::Engine;
use client_core::Fingerprint;
use serde::Deserialize;

use super::HubClientError;
use crate::invite::{make_invite, Invite, InviteError, ServerAddress};

/// Avancement d'un slot, dans la forme du serveur de jeu (le hub la renvoie telle quelle).
pub use server::SlotStatus;

/// Taille maximale d'une seed (et d'une sauvegarde) acceptée par le hub : 32 Mio
/// (ADR 0052, point 7). Le launcher refuse avant d'envoyer, pour ne pas faire monter
/// des dizaines de mégaoctets pour rien.
pub const MAX_SEED_BYTES: u64 = 32 << 20;

/// Ce qu'on envoie au hub pour créer une partie hébergée.
///
/// `Debug` est écrit à la main : il n'affiche ni le mot de passe ni le contenu des
/// fichiers (seulement leur taille).
#[derive(Clone)]
pub struct GameUpload {
    /// Nom affiché de la partie (64 caractères au plus, vérifié par le hub).
    pub name: String,
    /// Mot de passe de la salle ; `None` : partie ouverte.
    pub password: Option<String>,
    /// Le fichier `.archipelago`.
    pub seed: Vec<u8>,
    /// Une sauvegarde `.raftup` de cette seed, pour continuer une partie commencée.
    pub save: Option<Vec<u8>>,
}

impl fmt::Debug for GameUpload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameUpload")
            .field("name", &self.name)
            .field("password", &self.password.as_ref().map(|_| "***"))
            .field("seed_bytes", &self.seed.len())
            .field("save_bytes", &self.save.as_ref().map(Vec::len))
            .finish()
    }
}

impl GameUpload {
    /// Lit la seed (et la sauvegarde, s'il y en a une) sur le disque, après avoir vérifié
    /// leur taille. Un mot de passe vide compte comme pas de mot de passe.
    pub fn from_files(
        name: &str,
        password: Option<&str>,
        seed: &Path,
        save: Option<&Path>,
    ) -> Result<GameUpload, HubClientError> {
        let seed = read_limited(seed, "seed")?;
        let save = save
            .map(|path| read_limited(path, "sauvegarde"))
            .transpose()?;
        Ok(GameUpload {
            name: name.trim().to_string(),
            password: password.filter(|p| !p.is_empty()).map(str::to_string),
            seed,
            save,
        })
    }

    /// Corps JSON de `POST /v1/games` : fichiers en base64 standard (avec `=`).
    pub(crate) fn to_json(&self) -> serde_json::Value {
        let engine = base64::engine::general_purpose::STANDARD;
        let mut body = serde_json::json!({
            "name": self.name,
            "seed": engine.encode(&self.seed),
        });
        if let Some(password) = &self.password {
            body["password"] = serde_json::json!(password);
        }
        if let Some(save) = &self.save {
            body["save"] = serde_json::json!(engine.encode(save));
        }
        body
    }
}

/// Lit un fichier de `MAX_SEED_BYTES` au plus.
fn read_limited(path: &Path, what: &'static str) -> Result<Vec<u8>, HubClientError> {
    let unreadable = |e: std::io::Error| HubClientError::File {
        what,
        detail: format!("{} : {e}", path.display()),
    };
    let size = std::fs::metadata(path).map_err(unreadable)?.len();
    if size > MAX_SEED_BYTES {
        return Err(HubClientError::TooLarge { what, bytes: size });
    }
    std::fs::read(path).map_err(unreadable)
}

/// Une partie hébergée, comme le hub la décrit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HostedGame {
    /// Identifiant (32 chiffres hexadécimaux) : il sert de laissez-passer.
    pub id: String,
    pub name: String,
    pub seed_name: String,
    /// `active` (le serveur tourne) ou `stopped` (il n'a pas pu repartir).
    pub state: String,
    /// Adresse à donner aux mods : `hôte:port`, sans schéma.
    pub address: String,
    pub host: String,
    pub port: u16,
    /// Empreinte SHA-256 du certificat, en hexadécimal ; `None` si la partie est arrêtée.
    #[serde(default)]
    pub fingerprint: Option<String>,
    pub players: u32,
    pub has_password: bool,
    pub created_at: i64,
}

impl HostedGame {
    /// Vrai si le serveur de la partie tourne sur le hub.
    pub fn is_active(&self) -> bool {
        self.state == "active"
    }

    /// L'empreinte, si elle est bien formée.
    pub fn pinned(&self) -> Option<Fingerprint> {
        self.fingerprint.as_deref().and_then(Fingerprint::parse)
    }

    /// Adresse `wss://hôte:port` : le serveur du hub a toujours un certificat.
    pub fn server_address(&self) -> Result<ServerAddress, InviteError> {
        ServerAddress::parse(&format!("wss://{}", self.address))
    }

    /// Adresse en `wss://`, pour les écrans et les outils de Raftup (épinglage).
    pub fn wss_url(&self) -> String {
        self.server_address()
            .map(|a| a.to_url())
            .unwrap_or_else(|_| self.address.clone())
    }

    /// Lien d'invitation `raftup://join?…` vers cette partie, avec l'empreinte (`fp`).
    pub fn invite_link(
        &self,
        slot: Option<&str>,
        game: Option<&str>,
    ) -> Result<String, InviteError> {
        make_invite(&Invite {
            address: self.server_address()?,
            slot: slot.map(str::to_string),
            game: game.map(str::to_string),
            fingerprint: self.pinned(),
        })
    }
}

/// Réponse de `POST /v1/games`.
///
/// `Debug` à la main : le mot de passe d'admin n'y apparaît pas.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct CreatedGame {
    // `flatten` : les champs de la partie sont au même niveau que les autres dans le
    // JSON ; serde les range dans `game`.
    #[serde(flatten)]
    pub game: HostedGame,
    /// Mot de passe d'admin de la partie (`!admin login …`), à montrer à l'hôte seul.
    pub admin_password: String,
    /// Vrai si la sauvegarde envoyée a repris.
    #[serde(default)]
    pub resumed: bool,
}

impl fmt::Debug for CreatedGame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreatedGame")
            .field("game", &self.game)
            .field("admin_password", &"***")
            .field("resumed", &self.resumed)
            .finish()
    }
}

/// Réponse de `GET /v1/games/{id}` : état détaillé.
#[derive(Clone, PartialEq, Deserialize)]
pub struct GameStatus {
    #[serde(flatten)]
    pub game: HostedGame,
    /// Vrai si le compte qui demande est l'hôte.
    pub is_host: bool,
    /// Mot de passe d'admin, rendu à l'hôte seulement.
    #[serde(default)]
    pub admin_password: Option<String>,
    /// Connexions ouvertes (joueurs et outils de suivi).
    #[serde(default)]
    pub connections: usize,
    /// Avancement par slot ; vide si la partie est arrêtée.
    #[serde(default)]
    pub slots: Vec<SlotStatus>,
}

impl fmt::Debug for GameStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameStatus")
            .field("game", &self.game)
            .field("is_host", &self.is_host)
            .field(
                "admin_password",
                &self.admin_password.as_ref().map(|_| "***"),
            )
            .field("connections", &self.connections)
            .field("slots", &self.slots.len())
            .finish()
    }
}

/// Réponse de `GET /v1/games`.
#[derive(Deserialize)]
pub(crate) struct GameList {
    pub games: Vec<HostedGame>,
}

/// Vrai si `id` a la forme d'un identifiant de partie (32 chiffres hexadécimaux) : rien
/// d'autre ne part dans un chemin d'URL.
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game() -> HostedGame {
        HostedGame {
            id: "0123456789abcdef0123456789abcdef".into(),
            name: "Partie".into(),
            seed_name: "123".into(),
            state: "active".into(),
            address: "hub.example.org:38281".into(),
            host: "hub.example.org".into(),
            port: 38281,
            fingerprint: Some("ab".repeat(32)),
            players: 2,
            has_password: false,
            created_at: 0,
        }
    }

    #[test]
    fn lien_en_wss_avec_empreinte() {
        let link = game().invite_link(Some("Mimi"), Some("APQuest")).unwrap();
        let invite = crate::invite::parse_invite(&link).unwrap();
        assert!(invite.address.secure);
        assert_eq!(invite.address.host, "hub.example.org");
        assert_eq!(invite.address.port, 38281);
        assert_eq!(invite.fingerprint, game().pinned());
        // Le mod du jeu reçoit l'adresse sans schéma (repli wss → ws).
        assert_eq!(invite.address_for_game(), "hub.example.org:38281");
        assert_eq!(game().wss_url(), "wss://hub.example.org:38281");
    }

    #[test]
    fn identifiant_verifie() {
        assert!(valid_id("0123456789abcdef0123456789ABCDEF"));
        assert!(!valid_id("../x"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"g".repeat(32)));
    }

    #[test]
    fn secrets_masques() {
        let upload = GameUpload {
            name: "P".into(),
            password: Some("motdepasse".into()),
            seed: vec![1, 2, 3],
            save: None,
        };
        assert!(!format!("{upload:?}").contains("motdepasse"));
        let created = CreatedGame {
            game: game(),
            admin_password: "ADMIN1234".into(),
            resumed: false,
        };
        assert!(!format!("{created:?}").contains("ADMIN1234"));
    }

    #[test]
    fn corps_en_base64() {
        let upload = GameUpload {
            name: "P".into(),
            password: None,
            seed: vec![0, 1, 2],
            save: Some(vec![255]),
        };
        let body = upload.to_json();
        assert_eq!(body["seed"], "AAEC");
        assert_eq!(body["save"], "/w==");
        assert!(body.get("password").is_none());
    }
}
