//! Types échangés avec l'API `/v1` du hub, **redéfinis côté launcher**.
//!
//! Le hub est en AGPL (ADR 0003) : le launcher (MIT / Apache) ne dépend jamais de sa
//! crate. Ces types recopient donc la forme du JSON de `crates/hub/src/api.rs`, sans
//! partager de code. Un champ ajouté plus tard par le hub est ignoré à la lecture
//! (serde ignore les champs inconnus par défaut) : un vieux launcher reste compatible.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Mot de passe tapé par le joueur. Il ne fait que passer : jamais écrit sur le disque,
/// jamais dans le journal (NF09).
///
/// `Debug` est écrit à la main pour afficher `Password(***)` ; il n'y a pas de `Display`.
#[derive(Clone)]
pub struct Password(String);

impl Password {
    pub fn new(text: impl Into<String>) -> Password {
        Password(text.into())
    }

    /// Le texte en clair, seulement pour le corps de la requête.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// Jeton de session délivré par le hub à la connexion. C'est un secret : qui le connaît
/// agit au nom du joueur. `Debug` le masque.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn new(text: impl Into<String>) -> SessionToken {
        SessionToken(text.into())
    }

    /// Le jeton en clair, pour l'en-tête `Authorization` ou le trousseau. Ne pas logger.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(***)")
    }
}

/// Réponse de `GET /v1`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiInfo {
    pub service: String,
    pub api_version: u32,
}

/// Réponse de `GET /v1/info` : ce que le hub propose.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct HubInfo {
    pub service: String,
    /// Nom du hub choisi par son admin (« Rafthub » par défaut).
    #[serde(default)]
    pub name: String,
    /// Version du logiciel du hub.
    #[serde(default)]
    pub version: String,
    /// Moyens de connexion : `password`, et `discord` s'il est proposé.
    #[serde(default)]
    pub login_methods: Vec<String>,
    /// `open`, `invite` ou `closed` (ADR 0036).
    #[serde(default)]
    pub registration_mode: String,
    /// Le hub accepte d'héberger des parties (`POST /v1/games`, ADR 0052). Absent chez
    /// un hub plus ancien : faux.
    #[serde(default)]
    pub hosting: bool,
}

impl HubInfo {
    /// Ce qu'on suppose d'un hub trop ancien pour avoir `GET /v1/info`.
    pub fn password_only() -> HubInfo {
        HubInfo {
            service: "raftup-hub".to_string(),
            name: String::new(),
            version: String::new(),
            login_methods: vec!["password".to_string()],
            registration_mode: "open".to_string(),
            hosting: false,
        }
    }

    /// Vrai si le hub propose d'héberger des parties (ADR 0052).
    pub fn offers_hosting(&self) -> bool {
        self.hosting
    }

    /// Vrai si le hub propose la connexion Discord.
    pub fn offers_discord(&self) -> bool {
        self.login_methods.iter().any(|m| m == "discord")
    }
}

/// Compte créé (`POST /v1/accounts`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Account {
    pub user_id: i64,
    pub username: String,
}

/// Session ouverte (`POST /v1/sessions`). `Debug` vient de `derive`, mais le jeton s'y
/// affiche masqué grâce au `Debug` de [`SessionToken`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub token: SessionToken,
    pub user_id: i64,
    pub username: String,
    /// Fin de validité, en secondes Unix.
    pub expires_at: i64,
}

/// Forme JSON de la session : le jeton y est une simple chaîne.
#[derive(Deserialize)]
pub(crate) struct SessionJson {
    pub token: String,
    pub user_id: i64,
    pub username: String,
    pub expires_at: i64,
}

/// Compte connecté (`GET /v1/me`). L'adresse n'est jamais journalisée (RGPD).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user_id: i64,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Vrai quand l'adresse a été confirmée (lien reçu par e-mail).
    #[serde(default)]
    pub email_verified: bool,
    /// Vrai quand un compte Discord est lié à ce compte. Un hub plus ancien, sans ce
    /// champ, est lu comme « pas lié ».
    #[serde(default)]
    pub discord_linked: bool,
}

/// État de présence. `snake_case` : `in_game` dans le JSON, comme côté hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Online,
    InGame,
    Offline,
}

/// Présence d'un joueur : le nom du jeu n'existe qu'en partie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

/// Longueur maximale du nom du jeu dans la présence, en caractères : la même limite
/// que le hub (`MAX_GAME_CHARS` dans `crates/hub/src/api.rs`).
pub const MAX_GAME_CHARS: usize = 64;

impl Presence {
    /// « En partie » avec le nom du jeu, **nettoyé** pour que le hub l'accepte :
    /// caractères de contrôle retirés, espaces autour enlevés, coupé à
    /// [`MAX_GAME_CHARS`] caractères (avec « … » à la fin). Un nom vide après
    /// nettoyage donne « en ligne » : le hub refuserait « en partie » sans jeu.
    pub fn playing(game: &str) -> Presence {
        let cleaned: String = game.chars().filter(|c| !c.is_control()).collect();
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            return Presence::online();
        }
        // On compte en caractères (`chars`), pas en octets : « é » compte pour un.
        let game = if cleaned.chars().count() > MAX_GAME_CHARS {
            let mut short: String = cleaned.chars().take(MAX_GAME_CHARS - 1).collect();
            // Une coupe juste après une espace laisserait « Jeu … » : on la retire.
            short.truncate(short.trim_end().len());
            short.push('…');
            short
        } else {
            cleaned.to_string()
        };
        Presence::in_game(game)
    }

    pub fn online() -> Presence {
        Presence {
            status: Status::Online,
            game: None,
        }
    }

    pub fn offline() -> Presence {
        Presence {
            status: Status::Offline,
            game: None,
        }
    }

    pub fn in_game(game: impl Into<String>) -> Presence {
        Presence {
            status: Status::InGame,
            game: Some(game.into()),
        }
    }
}

/// Un ami et sa présence (`GET /v1/friends`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Friend {
    pub username: String,
    pub presence: Presence,
}

/// Un joueur dans une liste de demandes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Player {
    pub username: String,
}

/// Demandes d'ami reçues et envoyées (`GET /v1/friends/requests`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FriendRequests {
    pub incoming: Vec<Player>,
    pub outgoing: Vec<Player>,
}

/// Résultat d'une demande d'ami.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// La demande attend la réponse de l'autre joueur.
    Pending,
    /// L'autre joueur nous avait déjà demandé : vous êtes amis tout de suite.
    Accepted,
}

#[derive(Deserialize)]
pub(crate) struct RequestOutcomeJson {
    pub status: String,
}

/// Corps d'erreur du hub : `{"error": "<code>", "message": "<texte>"}`.
#[derive(Deserialize)]
pub(crate) struct ErrorJson {
    pub error: String,
    #[serde(default)]
    pub message: String,
}

/// Code d'erreur stable renvoyé par le hub (`error` dans le JSON).
///
/// Un code que ce launcher ne connaît pas encore arrive dans `Other` : un hub plus
/// récent ne casse pas un vieux launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorCode {
    InvalidUsername,
    InvalidEmail,
    WeakPassword,
    UsernameTaken,
    /// Pseudo déjà changé il y a moins de 24 heures.
    UsernameChangeTooSoon,
    EmailTaken,
    InvalidCredentials,
    Unauthorized,
    TooManyAttempts,
    UserNotFound,
    RequestNotFound,
    FriendNotFound,
    SelfFriend,
    AlreadyFriends,
    RequestExists,
    InvalidPresence,
    /// Le hub n'accepte plus d'inscriptions (mode `closed`, ADR 0036).
    RegistrationClosed,
    /// Le hub demande un code d'invitation et aucun n'a été envoyé.
    InviteRequired,
    /// Code d'invitation inconnu, expiré ou déjà utilisé.
    InvalidInvite,
    /// Ce hub ne propose pas Discord (auto-hébergé, ou Discord pas réglé).
    DiscordDisabled,
    /// Le joueur a refusé chez Discord.
    DiscordRefused,
    /// Discord n'a pas répondu au hub.
    DiscordUnavailable,
    /// Ce compte Discord est déjà lié à un autre compte.
    DiscordTaken,
    /// Ce compte a déjà un (autre) compte Discord lié.
    AlreadyLinked,
    /// Aucun compte Discord n'est lié à ce compte (rien à délier).
    IdentityNotFound,
    /// Délier Discord laisserait le compte sans aucun moyen de se connecter.
    LastIdentity,
    /// Connexion inconnue ou expirée (`state`, `secret` ou `ticket` refusé).
    InvalidToken,
    /// Compte suspendu par un admin.
    AccountSuspended,
    /// Ce hub n'héberge pas de parties (réglage `hosting` éteint, ADR 0052).
    HostingDisabled,
    /// Partie hébergée inconnue (ou supprimée).
    GameNotFound,
    /// Seul l'hôte de la partie peut faire ceci (export, suppression).
    NotGameHost,
    /// Le fichier envoyé n'est pas une seed lisible.
    InvalidSeed,
    /// La sauvegarde envoyée ne va pas avec cette seed.
    InvalidSave,
    /// Seed (ou sauvegarde) trop grosse pour le hub.
    SeedTooLarge,
    /// Trop de joueurs dans la seed pour une partie hébergée.
    TooManyPlayers,
    /// Le compte a déjà le nombre maximal de parties hébergées.
    GameLimitReached,
    /// Le hub héberge déjà le nombre maximal de parties.
    HubFull,
    /// Plus aucun port libre dans la plage des parties.
    NoFreePort,
    BadRequest,
    Internal,
    Other(String),
}

impl ErrorCode {
    pub fn parse(code: &str) -> ErrorCode {
        match code {
            "invalid_username" => ErrorCode::InvalidUsername,
            "invalid_email" => ErrorCode::InvalidEmail,
            "weak_password" => ErrorCode::WeakPassword,
            "username_taken" => ErrorCode::UsernameTaken,
            "username_change_too_soon" => ErrorCode::UsernameChangeTooSoon,
            "email_taken" => ErrorCode::EmailTaken,
            "invalid_credentials" => ErrorCode::InvalidCredentials,
            "unauthorized" => ErrorCode::Unauthorized,
            "too_many_attempts" => ErrorCode::TooManyAttempts,
            "user_not_found" => ErrorCode::UserNotFound,
            "request_not_found" => ErrorCode::RequestNotFound,
            "friend_not_found" => ErrorCode::FriendNotFound,
            "self_friend" => ErrorCode::SelfFriend,
            "already_friends" => ErrorCode::AlreadyFriends,
            "request_exists" => ErrorCode::RequestExists,
            "invalid_presence" => ErrorCode::InvalidPresence,
            "registration_closed" => ErrorCode::RegistrationClosed,
            "invite_required" => ErrorCode::InviteRequired,
            "invalid_invite" => ErrorCode::InvalidInvite,
            "discord_disabled" => ErrorCode::DiscordDisabled,
            "discord_refused" => ErrorCode::DiscordRefused,
            "discord_unavailable" => ErrorCode::DiscordUnavailable,
            "discord_taken" => ErrorCode::DiscordTaken,
            "already_linked" => ErrorCode::AlreadyLinked,
            "identity_not_found" => ErrorCode::IdentityNotFound,
            "last_identity" => ErrorCode::LastIdentity,
            "invalid_token" => ErrorCode::InvalidToken,
            "account_suspended" => ErrorCode::AccountSuspended,
            "hosting_disabled" => ErrorCode::HostingDisabled,
            "game_not_found" => ErrorCode::GameNotFound,
            "not_game_host" => ErrorCode::NotGameHost,
            "invalid_seed" => ErrorCode::InvalidSeed,
            "invalid_save" => ErrorCode::InvalidSave,
            "seed_too_large" => ErrorCode::SeedTooLarge,
            "too_many_players" => ErrorCode::TooManyPlayers,
            "game_limit_reached" => ErrorCode::GameLimitReached,
            "hub_full" => ErrorCode::HubFull,
            "no_free_port" => ErrorCode::NoFreePort,
            "bad_request" => ErrorCode::BadRequest,
            "internal" => ErrorCode::Internal,
            other => ErrorCode::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_du_jeu_nettoye() {
        assert_eq!(
            Presence::playing("  APQuest \n"),
            Presence::in_game("APQuest")
        );
        assert_eq!(Presence::playing("A\u{7}B"), Presence::in_game("AB"));
        assert_eq!(Presence::playing(" \t "), Presence::online());
        assert_eq!(Presence::playing(""), Presence::online());
    }

    #[test]
    fn nom_du_jeu_coupe_a_64_caracteres() {
        let exact = "é".repeat(MAX_GAME_CHARS);
        assert_eq!(Presence::playing(&exact), Presence::in_game(exact.clone()));
        let long = "é".repeat(MAX_GAME_CHARS + 10);
        let game = Presence::playing(&long).game.unwrap();
        assert_eq!(game.chars().count(), MAX_GAME_CHARS);
        assert!(game.ends_with('…'));
        // Coupe juste après une espace : pas d'espace avant « … ».
        let spaced = format!("{} suite", "a".repeat(MAX_GAME_CHARS - 2));
        let game = Presence::playing(&spaced).game.unwrap();
        assert!(!game.contains(" …"), "{game}");
        assert!(game.chars().count() <= MAX_GAME_CHARS);
    }

    #[test]
    fn me_d_un_hub_ancien() {
        let me: Me = serde_json::from_str(r#"{"user_id":1,"username":"Mimi"}"#).unwrap();
        assert!(!me.discord_linked);
        let me: Me =
            serde_json::from_str(r#"{"user_id":1,"username":"Mimi","discord_linked":true}"#)
                .unwrap();
        assert!(me.discord_linked);
    }
}
