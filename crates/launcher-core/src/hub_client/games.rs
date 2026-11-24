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
    /// `active` (le serveur tourne), `sleeping` (en veille : la première connexion la
    /// réveille, ADR 0052 point 6) ou `stopped` (il n'a pas pu repartir).
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
    /// Dernière fois qu'un joueur était connecté (secondes depuis 1970) ; absent chez
    /// un hub plus ancien, ou si personne n'a encore joué.
    #[serde(default)]
    pub last_activity_at: Option<i64>,
}

impl HostedGame {
    /// Vrai si le serveur de la partie tourne sur le hub.
    pub fn is_active(&self) -> bool {
        self.state == "active"
    }

    /// Vrai si la partie est en veille : elle se réveille à la première connexion.
    pub fn is_sleeping(&self) -> bool {
        self.state == "sleeping"
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

/// Nom de la seed dans l'export du hub (`GET /v1/games/{id}/export`).
pub const EXPORT_SEED_FILE: &str = "partie.archipelago";
/// Nom de la sauvegarde dans l'export : celui de la seed avec `.raftup`, comme en
/// self-host, pour que le serveur de ce PC la retrouve à côté de la seed.
pub const EXPORT_SAVE_FILE: &str = "partie.raftup";

/// Une partie téléchargée depuis le hub, pour la reprendre sur ce PC.
///
/// `Debug` à la main : seulement la taille des fichiers.
#[derive(Clone, PartialEq, Eq)]
pub struct GameExport {
    /// Le fichier `.archipelago`.
    pub seed: Vec<u8>,
    /// La sauvegarde `.raftup` ; `None` si la partie n'en avait pas encore.
    pub save: Option<Vec<u8>>,
}

impl fmt::Debug for GameExport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameExport")
            .field("seed_bytes", &self.seed.len())
            .field("save_bytes", &self.save.as_ref().map(Vec::len))
            .finish()
    }
}

impl GameExport {
    /// Relit le zip du hub. Seuls les deux fichiers attendus sont lus, chacun limité à
    /// [`MAX_SEED_BYTES`] une fois décompressé (un zip piégé ne remplit pas la mémoire).
    /// Les autres entrées sont ignorées : aucun chemin du zip n'est écrit sur le disque.
    pub fn from_zip(bytes: &[u8]) -> Result<GameExport, HubClientError> {
        let bad = |detail: String| HubClientError::BadExport(detail);
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(e.to_string()))?;
        let seed = read_entry(&mut archive, EXPORT_SEED_FILE)?
            .ok_or_else(|| bad(format!("{EXPORT_SEED_FILE} manquant")))?;
        let save = read_entry(&mut archive, EXPORT_SAVE_FILE)?;
        Ok(GameExport { seed, save })
    }
}

impl GameExport {
    /// Range la seed et la sauvegarde dans `dir` (créé s'il manque), sous les noms du
    /// self-host : `partie.archipelago` et `partie.raftup` à côté. Le serveur de ce PC
    /// retrouve donc la sauvegarde tout seul. Rend le chemin de la seed.
    ///
    /// Une ancienne sauvegarde du même dossier (reprise précédente) est remplacée ;
    /// sans sauvegarde dans l'export, elle est retirée, pour ne pas reprendre une
    /// partie plus ancienne que celle du hub.
    pub fn save_in(&self, dir: &Path) -> Result<std::path::PathBuf, HubClientError> {
        let failed = |e: std::io::Error| HubClientError::File {
            what: "partie reprise",
            detail: format!("{} : {e}", dir.display()),
        };
        std::fs::create_dir_all(dir).map_err(failed)?;
        let seed = dir.join(EXPORT_SEED_FILE);
        let save = dir.join(EXPORT_SAVE_FILE);
        std::fs::write(&seed, &self.seed).map_err(failed)?;
        match &self.save {
            Some(bytes) => std::fs::write(&save, bytes).map_err(failed)?,
            None if save.exists() => std::fs::remove_file(&save).map_err(failed)?,
            None => {}
        }
        Ok(seed)
    }
}

/// Lit une entrée du zip par son nom exact ; `None` si elle n'y est pas.
fn read_entry(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<Option<Vec<u8>>, HubClientError> {
    use std::io::Read;
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(HubClientError::BadExport(e.to_string())),
    };
    let mut bytes = Vec::new();
    // `take` coupe la lecture un octet après la limite : au-delà, c'est trop gros,
    // quelle que soit la taille annoncée dans l'en-tête du zip.
    (&mut entry)
        .take(MAX_SEED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| HubClientError::BadExport(format!("{name} : {e}")))?;
    if bytes.len() as u64 > MAX_SEED_BYTES {
        return Err(HubClientError::TooLarge {
            what: "sauvegarde ou seed",
            bytes: bytes.len() as u64,
        });
    }
    Ok(Some(bytes))
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
            last_activity_at: None,
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

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in files {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn export_range_comme_en_self_host() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("partie-1");
        let export = GameExport {
            seed: b"seed".to_vec(),
            save: Some(b"save".to_vec()),
        };
        let seed = export.save_in(&target).unwrap();
        assert_eq!(seed, target.join("partie.archipelago"));
        assert_eq!(std::fs::read(&seed).unwrap(), b"seed");
        // Même nom de base que la seed : `default_save_path` la retrouve.
        assert_eq!(
            std::fs::read(server::save::default_save_path(&seed)).unwrap(),
            b"save"
        );
        // Nouvel export sans sauvegarde : l'ancienne ne doit pas reprendre.
        let fresh = GameExport {
            seed: b"seed".to_vec(),
            save: None,
        };
        fresh.save_in(&target).unwrap();
        assert!(!target.join("partie.raftup").exists());
    }

    #[test]
    fn export_relu() {
        let bytes = zip_of(&[
            ("../piege.txt", b"x"),
            (EXPORT_SEED_FILE, b"seed"),
            (EXPORT_SAVE_FILE, b"save"),
        ]);
        let export = GameExport::from_zip(&bytes).unwrap();
        assert_eq!(export.seed, b"seed");
        assert_eq!(export.save.as_deref(), Some(&b"save"[..]));
        // Pas encore de sauvegarde : la seed seule.
        let export = GameExport::from_zip(&zip_of(&[(EXPORT_SEED_FILE, b"s")])).unwrap();
        assert_eq!(export.save, None);
        // Sans seed, ou pas un zip : refusé.
        assert!(matches!(
            GameExport::from_zip(&zip_of(&[(EXPORT_SAVE_FILE, b"s")])),
            Err(HubClientError::BadExport(_))
        ));
        assert!(GameExport::from_zip(b"pas un zip").is_err());
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
