//! Écran « Amis » : compte sur le hub Rafthub, liste d'amis avec leur présence, demandes
//! d'ami (ADR 0002, 0035, 0036).
//!
//! L'écran ne parle au hub que par `launcher_core::hub_client` (le hub est en AGPL, le
//! launcher n'en dépend pas, ADR 0003). Chaque appel réseau part dans un **thread à
//! part** : le thread du dessin ne fait jamais attendre la fenêtre.
//!
//! Le client du hub (`HubClient`) **part dans le thread** avec la demande, puis **revient**
//! avec la réponse : un seul appel à la fois, et pas besoin de verrou pour le partager.
//!
//! Tant que le joueur est connecté, [`background`] renvoie sa présence « en ligne » et
//! relit la liste d'amis toutes les minutes, même si l'écran n'est pas affiché (le hub
//! oublie une présence au bout de 2 minutes).
//!
//! Un lien `raftup://hub/<hôte>` propose de changer de Rafthub
//! ([`FriendsState::offer_switch`]) ; rien ne change avant le clic sur « Passer sur ce
//! hub » dans la boîte de [`switch_dialog`].

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use launcher_core::config;
use launcher_core::hub_client::{
    ErrorCode, Friend, FriendRequests, HubClient, HubClientError, HubUrl, KeyringStore,
    MemoryStore, Password, Presence, RequestOutcome, Status, TokenStore, DEFAULT_HUB_URL,
};

use super::library::load_paths;
use crate::app::Page;
use crate::theme;
use crate::widgets;

/// Toutes les combien on renvoie la présence et relit la liste (le hub l'oublie à 120 s).
const SYNC_EVERY: Duration = Duration::from_secs(60);

/// Formulaire affiché quand on n'est pas connecté.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Login,
    Register,
}

/// Message affiché en haut de l'écran.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Notice {
    Info(String),
    Error(String),
}

/// Ce qu'on demande au thread réseau.
enum Action {
    /// Reprendre la session rangée dans le trousseau (au démarrage).
    Restore,
    Login {
        email: String,
        password: Password,
    },
    Register {
        email: String,
        password: Password,
        username: String,
        /// Code d'invitation ; `None` si le champ est resté vide.
        invite_code: Option<String>,
    },
    Logout,
    /// Changer de hub : présence « hors ligne » envoyée à l'ancien (s'il y avait une
    /// session), puis reprise de la session rangée pour le nouveau, s'il y en a une.
    Switch(HubUrl),
    /// Renvoyer la présence et relire amis et demandes.
    Sync,
    SendRequest(String),
    Accept(String),
    Decline(String),
    Cancel(String),
    Remove(String),
    /// Changer son pseudo (`PATCH /v1/me`).
    Rename(String),
}

/// Amis et demandes, tels que relus sur le hub.
#[derive(Debug, Clone, Default)]
struct Lists {
    friends: Vec<Friend>,
    requests: FriendRequests,
}

/// Ce que le thread réseau a obtenu.
enum Done {
    /// Connecté (connexion, inscription ou session reprise).
    LoggedIn {
        username: String,
        lists: Lists,
        /// Le jeton n'a pas pu être rangé : la session sera perdue à la fermeture.
        warning: Option<String>,
    },
    /// Pas (ou plus) connecté ; `message` dit pourquoi, s'il y a lieu.
    LoggedOut { message: Option<String> },
    /// Listes relues ; `message` : résultat de l'action, à afficher.
    Lists {
        lists: Lists,
        message: Option<String>,
    },
    /// Pseudo changé : le nouveau, et les listes relues.
    Renamed { username: String, lists: Lists },
}

/// Réponse du thread : le client revient toujours, avec le résultat.
struct Outcome {
    client: HubClient,
    result: Result<Done, String>,
}

/// Ce que l'écran garde d'une image à l'autre.
pub struct FriendsState {
    /// Texte du champ « Adresse du hub ».
    url_text: String,
    /// Adresse en service (ou pourquoi elle est refusée).
    url: Result<HubUrl, String>,
    /// Où ranger le jeton. `Arc` : partagé avec chaque thread réseau.
    store: Arc<dyn TokenStore>,
    /// Le client, quand aucun appel n'est en cours (sinon il est dans le thread).
    client: Option<HubClient>,
    /// Pseudo du compte connecté.
    username: Option<String>,
    /// La reprise de session a déjà été tentée.
    started: bool,
    /// Appel en cours : on attend sa réponse sur ce canal.
    job: Option<Receiver<Outcome>>,
    mode: Mode,
    email: String,
    password: String,
    new_username: String,
    /// Code d'invitation tapé à l'inscription (hub en mode « sur invitation »).
    invite_code: String,
    add_name: String,
    lists: Lists,
    notice: Option<Notice>,
    /// Dernière synchronisation réussie ou tentée.
    last_sync: Option<Instant>,
    /// Le champ « Nouveau pseudo » est ouvert.
    rename_open: bool,
    /// Texte du champ « Nouveau pseudo ».
    rename_text: String,
    /// Ami dont on a cliqué « Retirer » une première fois (deuxième clic = confirmation).
    confirm_remove: Option<String>,
    /// En debug : se connecter tout seul avec ces identifiants (macro de captures).
    dev_login: Option<(String, String)>,
    /// Hub proposé par un lien `raftup://hub/…`, en attente de la confirmation du joueur.
    pending_switch: Option<HubUrl>,
    /// Écrire l'adresse choisie dans `config.toml` (faux dans les tests).
    persist: bool,
}

/// Résumé pour les cartes de la barre latérale.
pub struct Summary {
    pub username: Option<String>,
    /// Amis en ligne ou en partie.
    pub online: usize,
}

impl FriendsState {
    pub fn new() -> FriendsState {
        let saved = load_saved_url();
        let mut store: Arc<dyn TokenStore> = Arc::new(KeyringStore);
        let mut url_text = saved.unwrap_or_else(|| DEFAULT_HUB_URL.to_string());
        let mut dev_login = None;
        // En debug, `RAFTUP_DEV_HUB=<adresse>` choisit le hub et garde le jeton en mémoire
        // (le trousseau du PC n'est pas touché) ; `RAFTUP_DEV_HUB_LOGIN=<e-mail>:<mot de
        // passe>` connecte tout seul (comptes de test d'un hub lancé sur ce PC).
        if cfg!(debug_assertions) {
            if let Ok(url) = std::env::var("RAFTUP_DEV_HUB") {
                url_text = url;
                store = Arc::new(MemoryStore::new());
            }
            dev_login = std::env::var("RAFTUP_DEV_HUB_LOGIN").ok().and_then(|v| {
                v.split_once(':')
                    .map(|(e, p)| (e.to_string(), p.to_string()))
            });
        }
        // En debug, `RAFTUP_DEV_HUB_INSCRIPTION=1` ouvre l'onglet « Créer un compte »
        // (capture du formulaire d'inscription par la macro).
        let mode = if cfg!(debug_assertions) && std::env::var("RAFTUP_DEV_HUB_INSCRIPTION").is_ok()
        {
            Mode::Register
        } else {
            Mode::Login
        };
        let url = HubUrl::parse(&url_text).map_err(|e| e.to_string());
        FriendsState {
            client: url.as_ref().ok().map(|u| HubClient::new(u.clone())),
            url_text,
            url,
            store,
            username: None,
            started: false,
            job: None,
            mode,
            email: String::new(),
            password: String::new(),
            new_username: String::new(),
            invite_code: String::new(),
            add_name: String::new(),
            lists: Lists::default(),
            notice: None,
            last_sync: None,
            rename_open: false,
            rename_text: String::new(),
            confirm_remove: None,
            dev_login,
            pending_switch: None,
            persist: true,
        }
    }

    pub fn summary(&self) -> Summary {
        Summary {
            username: self.username.clone(),
            online: self
                .lists
                .friends
                .iter()
                .filter(|f| f.presence.status != Status::Offline)
                .count(),
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Lance `action` dans un thread, si aucun appel n'est en cours.
    fn start(&mut self, ctx: &egui::Context, action: Action) {
        if self.job.is_some() {
            return;
        }
        // `take` sort le client de l'état : il part dans le thread et reviendra.
        let Some(client) = self.client.take() else {
            return;
        };
        let store = Arc::clone(&self.store);
        let (sender, receiver) = mpsc::channel();
        // `Context` est un pointeur partagé : le cloner ne copie pas l'interface.
        let ctx = ctx.clone();
        // `move` : le thread possède le client, le magasin et l'action ; rien n'est
        // emprunté à l'écran, qui continue de se dessiner pendant ce temps.
        std::thread::spawn(move || {
            let outcome = run(client, store.as_ref(), action);
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        self.job = Some(receiver);
        if matches!(self.notice, Some(Notice::Info(_))) {
            self.notice = None;
        }
    }

    /// Lit la réponse du thread, sans attendre.
    fn poll(&mut self) {
        let Some(receiver) = &self.job else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                // Le thread a paniqué sans répondre : le client est perdu, on en refait un.
                self.job = None;
                self.client = self.url.as_ref().ok().map(|u| HubClient::new(u.clone()));
                self.username = None;
                self.notice = Some(Notice::Error("L'appel à Rafthub a échoué.".to_string()));
                return;
            }
        };
        self.job = None;
        self.client = Some(outcome.client);
        match outcome.result {
            Ok(Done::LoggedIn {
                username,
                lists,
                warning,
            }) => {
                self.username = Some(username);
                self.lists = lists;
                self.last_sync = Some(Instant::now());
                // Connexion réussie : une ancienne erreur (mauvais mot de passe…) n'a plus
                // lieu d'être ; une information (« vous êtes sur le Rafthub … ») reste.
                match warning {
                    Some(warning) => self.notice = Some(Notice::Error(warning)),
                    None if matches!(self.notice, Some(Notice::Error(_))) => self.notice = None,
                    None => {}
                }
            }
            Ok(Done::LoggedOut { message }) => {
                self.username = None;
                self.lists = Lists::default();
                // Sans message (aucune session rangée), le bandeau affiché est gardé :
                // un lien refusé au démarrage doit rester visible.
                if let Some(message) = message {
                    self.notice = Some(Notice::Info(message));
                }
            }
            Ok(Done::Lists { lists, message }) => {
                self.lists = lists;
                self.last_sync = Some(Instant::now());
                if let Some(message) = message {
                    self.notice = Some(Notice::Info(message));
                }
            }
            Ok(Done::Renamed { username, lists }) => {
                self.notice = Some(Notice::Info(format!(
                    "Votre pseudo est maintenant {username}."
                )));
                self.username = Some(username);
                self.lists = lists;
                self.last_sync = Some(Instant::now());
                self.rename_open = false;
                self.rename_text.clear();
            }
            Err(message) => self.notice = Some(Notice::Error(message)),
        }
    }

    /// Change d'adresse de hub depuis le champ de l'écran.
    fn apply_url(&mut self, ctx: &egui::Context) {
        let url = match HubUrl::parse(&self.url_text) {
            Ok(url) => url,
            Err(e) => {
                self.notice = Some(Notice::Error(e.to_string()));
                return;
            }
        };
        self.url_text = url.to_string();
        if self.url.as_ref().ok() == Some(&url) {
            return;
        }
        self.notice = None;
        self.switch_to(url, ctx);
    }

    /// Passe sur un autre hub : adresse enregistrée, ancienne session quittée proprement
    /// (présence « hors ligne », jeton gardé pour un retour), session du nouveau reprise.
    ///
    /// Rend faux, sans rien changer, si un appel est en cours : le client est alors dans
    /// le thread réseau et on ne peut pas encore le remplacer.
    fn switch_to(&mut self, url: HubUrl, ctx: &egui::Context) -> bool {
        if self.busy() {
            return false;
        }
        if self.persist {
            save_url(&url);
        }
        // Adresse refusée au démarrage : pas de client, on en crée un pour le nouveau hub.
        if self.client.is_none() {
            self.client = Some(HubClient::new(url.clone()));
        }
        self.url_text = url.to_string();
        self.url = Ok(url.clone());
        self.username = None;
        self.lists = Lists::default();
        self.confirm_remove = None;
        // La reprise de session du démarrage est remplacée par celle du nouveau hub.
        self.started = true;
        self.start(ctx, Action::Switch(url));
        true
    }

    /// Un lien `raftup://hub/…` propose ce hub : on attend la confirmation du joueur.
    /// Déjà sur ce hub : un simple message, pas de question.
    pub fn offer_switch(&mut self, url: HubUrl) {
        if self.url.as_ref().ok() == Some(&url) {
            self.pending_switch = None;
            self.notice = Some(Notice::Info(format!(
                "Vous êtes déjà sur le Rafthub {}.",
                url.host()
            )));
            return;
        }
        self.pending_switch = Some(url);
    }

    /// Lien `raftup://hub/…` illisible : on le dit en haut de l'écran, sans rien changer.
    pub fn link_refused(&mut self, error: &launcher_core::rafthub::HubLinkError) {
        self.pending_switch = None;
        self.notice = Some(Notice::Error(format!("Lien Rafthub refusé : {error}.")));
    }

    /// Le hub en attente de confirmation, s'il y en a un.
    pub fn pending_switch(&self) -> Option<&HubUrl> {
        self.pending_switch.as_ref()
    }

    /// « Passer sur ce hub » : rend vrai si le changement est parti.
    pub fn confirm_switch(&mut self, ctx: &egui::Context) -> bool {
        let Some(url) = self.pending_switch.clone() else {
            return false;
        };
        if !self.switch_to(url.clone(), ctx) {
            return false;
        }
        self.pending_switch = None;
        self.notice = Some(Notice::Info(format!(
            "Vous êtes maintenant sur le Rafthub {}.",
            url.host()
        )));
        true
    }

    /// « Rester » : la proposition est oubliée, rien ne change.
    pub fn cancel_switch(&mut self) {
        self.pending_switch = None;
    }

    /// L'hôte du hub en service (`hub.example.org`), s'il est valide.
    pub fn host(&self) -> Option<&str> {
        self.url.as_ref().ok().map(HubUrl::host)
    }
}

impl Default for FriendsState {
    fn default() -> Self {
        FriendsState::new()
    }
}

/// À appeler à chaque image, quel que soit l'écran affiché : reprise de la session au
/// démarrage, réponses des threads, présence et listes renouvelées.
pub fn background(state: &mut FriendsState, ctx: &egui::Context) {
    state.poll();
    if !state.started {
        state.started = true;
        let action = match state.dev_login.take() {
            Some((email, password)) => Action::Login {
                email,
                password: Password::new(password),
            },
            None => Action::Restore,
        };
        state.start(ctx, action);
        return;
    }
    let due = state.last_sync.is_none_or(|at| at.elapsed() >= SYNC_EVERY);
    if state.username.is_some() && !state.busy() && due {
        // Noté tout de suite : un hub injoignable n'est pas relancé à chaque image.
        state.last_sync = Some(Instant::now());
        state.start(ctx, Action::Sync);
    }
    if state.username.is_some() {
        // Réveille l'interface pour la prochaine synchronisation, même sans clic.
        ctx.request_repaint_after(SYNC_EVERY);
    }
}

// ---------------------------------------------------------------------------
// Travail du thread réseau (sans interface).
// ---------------------------------------------------------------------------

/// Exécute une action, dans le thread réseau. Rend toujours le client.
fn run(client: HubClient, store: &dyn TokenStore, action: Action) -> Outcome {
    let url = client.url().clone();
    match action {
        Action::Restore => match HubClient::restore(url.clone(), store) {
            Ok(Some((client, me))) => logged_in(client, me.username, None),
            Ok(None) => Outcome {
                client: HubClient::new(url),
                result: Ok(Done::LoggedOut { message: None }),
            },
            // Hub injoignable au démarrage : on ne dit rien de plus qu'un message ; le
            // joueur peut réessayer en se connectant.
            Err(error) => Outcome {
                client: HubClient::new(url),
                result: Err(error_text(&error)),
            },
        },
        Action::Login { email, password } => login(client, store, &email, &password),
        Action::Register {
            email,
            password,
            username,
            invite_code,
        } => match client.register(&email, &password, &username, invite_code.as_deref()) {
            Ok(_) => login(client, store, &email, &password),
            Err(error) => Outcome {
                client,
                result: Err(error_text(&error)),
            },
        },
        Action::Switch(to) => {
            // Ancien hub : la présence passe tout de suite à « hors ligne » pour les
            // amis (sinon il faudrait attendre 2 minutes). Un échec est sans importance :
            // le hub l'oubliera de lui-même. Le jeton reste rangé, pour un retour.
            if client.is_logged_in() {
                let _ = client.set_presence(&Presence::offline());
            }
            run(HubClient::new(to), store, Action::Restore)
        }
        Action::Logout => {
            let mut client = client;
            let remote = client.logout();
            let local = store.clear(&url);
            let message = match (remote, local) {
                (Ok(()), Ok(())) => "Déconnecté.".to_string(),
                (Err(_), _) => {
                    "Déconnecté sur ce PC ; Rafthub n'a pas pu être prévenu.".to_string()
                }
                (Ok(()), Err(e)) => format!("Déconnecté, mais le jeton reste rangé : {e}"),
            };
            Outcome {
                client,
                result: Ok(Done::LoggedOut {
                    message: Some(message),
                }),
            }
        }
        Action::Sync => after(client, store, Ok(None)),
        Action::SendRequest(name) => {
            let result = client.send_request(&name).map(|outcome| {
                Some(match outcome {
                    RequestOutcome::Pending => format!("Demande envoyée à {}.", name.trim()),
                    RequestOutcome::Accepted => {
                        format!("{} vous avait déjà demandé : vous êtes amis.", name.trim())
                    }
                })
            });
            after(client, store, result)
        }
        Action::Accept(name) => {
            let result = client
                .accept_request(&name)
                .map(|()| Some(format!("{name} est maintenant votre ami.")));
            after(client, store, result)
        }
        Action::Decline(name) => {
            let result = client
                .decline_request(&name)
                .map(|()| Some(format!("Demande de {name} refusée.")));
            after(client, store, result)
        }
        Action::Cancel(name) => {
            let result = client
                .cancel_request(&name)
                .map(|()| Some(format!("Demande à {name} annulée.")));
            after(client, store, result)
        }
        Action::Remove(name) => {
            let result = client
                .remove_friend(&name)
                .map(|()| Some(format!("{name} n'est plus dans vos amis.")));
            after(client, store, result)
        }
        Action::Rename(name) => match client.change_username(&name) {
            Ok(account) => {
                let result = sync(&client).map(|lists| Done::Renamed {
                    username: account.username,
                    lists,
                });
                Outcome {
                    client,
                    result: result.map_err(|e| error_text(&e)),
                }
            }
            // Session refusée, ou refus du hub : même traitement qu'une action sur les amis.
            Err(error) => after(client, store, Err(error)),
        },
    }
}

/// Connexion, puis jeton rangé et listes lues.
fn login(
    mut client: HubClient,
    store: &dyn TokenStore,
    email: &str,
    password: &Password,
) -> Outcome {
    match client.login(email.trim(), password) {
        Ok(session) => {
            let warning = client.remember(store).err().map(|e| {
                format!("Connecté, mais la session ne sera pas gardée à la fermeture : {e}")
            });
            logged_in(client, session.username, warning)
        }
        Err(error) => Outcome {
            client,
            result: Err(error_text(&error)),
        },
    }
}

/// Après connexion : présence et listes.
fn logged_in(client: HubClient, username: String, warning: Option<String>) -> Outcome {
    let result = sync(&client).map(|lists| Done::LoggedIn {
        username,
        lists,
        warning,
    });
    Outcome {
        client,
        result: result.map_err(|e| error_text(&e)),
    }
}

/// Après une action sur les amis : relit les listes. Une session refusée par le hub
/// (expirée, révoquée ailleurs) déconnecte et oublie le jeton.
fn after(
    client: HubClient,
    store: &dyn TokenStore,
    action: Result<Option<String>, HubClientError>,
) -> Outcome {
    let result = action.and_then(|message| sync(&client).map(|lists| (lists, message)));
    match result {
        Ok((lists, message)) => Outcome {
            client,
            result: Ok(Done::Lists { lists, message }),
        },
        Err(error) if error.is_unauthorized() => {
            let url = client.url().clone();
            let _ = store.clear(&url);
            Outcome {
                client: HubClient::new(url),
                result: Ok(Done::LoggedOut {
                    message: Some("Session expirée : reconnectez-vous.".to_string()),
                }),
            }
        }
        Err(error) => Outcome {
            client,
            result: Err(error_text(&error)),
        },
    }
}

/// Présence « en ligne », puis amis et demandes.
fn sync(client: &HubClient) -> Result<Lists, HubClientError> {
    client.set_presence(&Presence::online())?;
    let mut friends = client.friends()?;
    sort_friends(&mut friends);
    let requests = client.requests()?;
    Ok(Lists { friends, requests })
}

/// Amis en partie d'abord, puis en ligne, puis hors ligne ; par pseudo à égalité.
/// Ordre stable d'une relecture à l'autre : la liste ne saute pas.
fn sort_friends(friends: &mut [Friend]) {
    fn rank(status: Status) -> u8 {
        match status {
            Status::InGame => 0,
            Status::Online => 1,
            Status::Offline => 2,
        }
    }
    friends.sort_by(|a, b| {
        rank(a.presence.status)
            .cmp(&rank(b.presence.status))
            .then_with(|| a.username.to_lowercase().cmp(&b.username.to_lowercase()))
    });
}

/// Message en clair pour le joueur.
fn error_text(error: &HubClientError) -> String {
    match error {
        HubClientError::Network(_) => {
            "Rafthub ne répond pas : vérifiez son adresse (en bas de l'écran) ou votre connexion."
                .to_string()
        }
        HubClientError::Api {
            code: ErrorCode::Internal,
            ..
        } => "Rafthub a rencontré une erreur. Réessayez plus tard.".to_string(),
        // Inscription sur un hub fermé ou sur invitation (ADR 0036) : nos propres
        // phrases, qui disent quoi faire dans le formulaire.
        HubClientError::Api {
            code: ErrorCode::RegistrationClosed,
            ..
        } => "Ce hub n'accepte plus d'inscriptions.".to_string(),
        HubClientError::Api {
            code: ErrorCode::InviteRequired,
            ..
        } => "Ce hub demande un code d'invitation : remplissez le champ « Code d'invitation »."
            .to_string(),
        HubClientError::Api {
            code: ErrorCode::InvalidInvite,
            ..
        } => "Code d'invitation incorrect, expiré ou déjà utilisé.".to_string(),
        HubClientError::Api {
            code: ErrorCode::UsernameChangeTooSoon,
            ..
        } => "Vous avez déjà changé de pseudo il y a moins de 24 heures : réessayez demain."
            .to_string(),
        // Les messages du hub sont déjà en français et sans secret.
        HubClientError::Api { message, .. } => capitalize(message),
        other => capitalize(&other.to_string()),
    }
}

/// Champ « Code d'invitation » : vide (ou des espaces) = pas de code envoyé.
fn invite_code_or_none(text: &str) -> Option<String> {
    let code = text.trim();
    (!code.is_empty()).then(|| code.to_string())
}

/// Première lettre en majuscule (« ce pseudo… » → « Ce pseudo… »).
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Adresse du hub mémorisée dans `config.toml`.
// ---------------------------------------------------------------------------

fn load_saved_url() -> Option<String> {
    let paths = load_paths().ok()?;
    match config::load(&paths.config_file()) {
        Ok(loaded) => loaded.config.hub_url,
        Err(e) => {
            tracing::warn!("adresse du hub non lue : {e}");
            None
        }
    }
}

/// Mémorise l'adresse ; l'adresse par défaut n'est pas écrite (elle pourra changer).
fn save_url(url: &HubUrl) {
    let Ok(paths) = load_paths() else {
        return;
    };
    let file = paths.config_file();
    let value = (url.as_str() != DEFAULT_HUB_URL).then_some(url.as_str());
    let result = config::load(&file).and_then(|loaded| {
        let mut config = loaded.config;
        if config.set_hub_url(value) {
            config.save(&file)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!("adresse du hub non enregistrée : {e}");
    }
}

// ---------------------------------------------------------------------------
// Dessin.
// ---------------------------------------------------------------------------

pub fn show(ui: &mut Ui, state: &mut FriendsState) -> Option<Page> {
    let ctx = ui.ctx().clone();
    super::page(ui, |ui| {
        let logged_in = state.username.is_some();
        // Copie de l'hôte : la fermeture ci-dessous emprunte `state` en écriture.
        let host = state.host().map(str::to_string);
        super::header(ui, "Communauté", "Amis", |ui| {
            if let Some(host) = &host {
                ui.label(widgets::dim(format!("Rafthub · {host}")).size(12.0));
            }
            if logged_in {
                if widgets::ghost_button(ui, "Se déconnecter") && !state.busy() {
                    state.start(&ctx, Action::Logout);
                }
                if widgets::ghost_button(ui, "Actualiser") && !state.busy() {
                    state.start(&ctx, Action::Sync);
                }
            }
            if state.busy() {
                ui.spinner();
            }
        });

        match &state.notice {
            Some(Notice::Error(message)) => {
                widgets::error_banner(ui, message);
                ui.add_space(10.0);
            }
            Some(Notice::Info(message)) => {
                info_banner(ui, message);
                ui.add_space(10.0);
            }
            None => {}
        }

        if logged_in {
            if ui.available_width() >= super::WIDE {
                ui.columns(2, |columns| {
                    friends_panel(&mut columns[0], state, &ctx);
                    requests_panels(&mut columns[1], state, &ctx);
                });
            } else {
                friends_panel(ui, state, &ctx);
                ui.add_space(12.0);
                requests_panels(ui, state, &ctx);
            }
        } else {
            account_panel(ui, state, &ctx);
        }
        ui.add_space(12.0);
        hub_panel(ui, state, &ctx);
    });
    None
}

/// Bandeau bleu discret pour un message d'information.
fn info_banner(ui: &mut Ui, message: &str) {
    egui::Frame::new()
        .fill(theme::ACCENT_BG)
        .stroke(egui::Stroke::new(1.0, theme::ACCENT))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(message).color(theme::TEXT));
        });
}

/// Connexion ou création de compte.
fn account_panel(ui: &mut Ui, state: &mut FriendsState, ctx: &egui::Context) {
    let heading = match state.mode {
        Mode::Login => "Se connecter",
        Mode::Register => "Créer un compte",
    };
    widgets::panel(ui, heading, |ui| {
        ui.label(widgets::dim(
            "Un compte Rafthub sert à retrouver vos amis et à voir qui joue. \
             Le launcher marche aussi sans compte.",
        ));
        ui.add_space(8.0);
        let mut tab = usize::from(state.mode == Mode::Register);
        widgets::tabs(
            ui,
            &[("Connexion", None), ("Nouveau compte", None)],
            &mut tab,
        );
        state.mode = if tab == 1 {
            Mode::Register
        } else {
            Mode::Login
        };
        ui.add_space(10.0);

        let mut submit = false;
        egui::Grid::new("hub_account")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                // À la connexion, le hub accepte l'e-mail ou le pseudo (ADR 0038) ;
                // à l'inscription, il faut l'adresse.
                if state.mode == Mode::Register {
                    ui.label("E-mail");
                } else {
                    ui.label("E-mail ou pseudo");
                }
                ui.add(
                    egui::TextEdit::singleline(&mut state.email)
                        .hint_text(if state.mode == Mode::Register {
                            "vous@exemple.org"
                        } else {
                            "vous@exemple.org ou votre pseudo"
                        })
                        .desired_width(280.0),
                );
                ui.end_row();
                if state.mode == Mode::Register {
                    ui.label("Pseudo");
                    ui.add(
                        egui::TextEdit::singleline(&mut state.new_username)
                            .hint_text("3 à 24 caractères")
                            .desired_width(280.0),
                    );
                    ui.end_row();
                }
                ui.label("Mot de passe");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.password)
                        .password(true)
                        .hint_text(if state.mode == Mode::Register {
                            "8 caractères au moins"
                        } else {
                            ""
                        })
                        .desired_width(280.0),
                );
                // Entrée dans le champ du mot de passe : comme un clic sur le bouton.
                submit |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.end_row();
                if state.mode == Mode::Register {
                    ui.label("Code d'invitation");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut state.invite_code)
                            .hint_text("si le hub en demande un")
                            .desired_width(280.0),
                    );
                    submit |=
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        let ready = !state.busy()
            && !state.email.trim().is_empty()
            && !state.password.is_empty()
            && (state.mode == Mode::Login || !state.new_username.trim().is_empty());
        let label = match state.mode {
            Mode::Login => "Se connecter",
            Mode::Register => "Créer le compte",
        };
        submit |= widgets::primary_button(ui, label, ready);
        if submit && ready {
            // Le mot de passe quitte le champ : il ne reste pas affiché (masqué) à l'écran.
            let password = Password::new(std::mem::take(&mut state.password));
            let email = state.email.trim().to_string();
            let action = match state.mode {
                Mode::Login => Action::Login { email, password },
                Mode::Register => Action::Register {
                    email,
                    password,
                    username: state.new_username.trim().to_string(),
                    invite_code: invite_code_or_none(&state.invite_code),
                },
            };
            state.start(ctx, action);
        }
    });
}

/// Liste d'amis avec leur présence.
fn friends_panel(ui: &mut Ui, state: &mut FriendsState, ctx: &egui::Context) {
    let count = state.lists.friends.len();
    let heading = format!("Amis · {count}");
    let mut action = None;
    widgets::panel(ui, &heading, |ui| {
        if let Some(name) = &state.username {
            let text = match state.host() {
                Some(host) => format!("Connecté en tant que {name} sur le Rafthub {host}"),
                None => format!("Connecté en tant que {name}"),
            };
            ui.label(widgets::dim(text));
            rename_row(ui, state, &mut action);
            ui.add_space(6.0);
        }
        if state.lists.friends.is_empty() {
            ui.label(widgets::dim(
                "Aucun ami pour l'instant. Ajoutez-en un par son pseudo.",
            ));
        }
        for friend in &state.lists.friends {
            ui.separator();
            ui.horizontal(|ui| {
                presence_dot(ui, friend.presence.status);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(&friend.username)
                            .strong()
                            .family(theme::bold_family())
                            .color(theme::TEXT),
                    );
                    let (text, color) = presence_text(&friend.presence);
                    ui.label(RichText::new(text).size(13.0).color(color));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let confirming = state.confirm_remove.as_deref() == Some(&friend.username);
                    if confirming {
                        if widgets::danger_button(ui, "Confirmer") {
                            action = Some(Action::Remove(friend.username.clone()));
                        }
                        if widgets::ghost_button(ui, "Non") {
                            state.confirm_remove = None;
                        }
                    } else if widgets::ghost_button(ui, "Retirer") {
                        state.confirm_remove = Some(friend.username.clone());
                    }
                });
            });
        }
    });
    if let Some(action) = action {
        state.confirm_remove = None;
        state.start(ctx, action);
    }
}

/// « Changer de pseudo » : un bouton, puis un champ avec « Valider » et « Annuler ».
/// Le hub n'accepte qu'un changement par 24 heures.
fn rename_row(ui: &mut Ui, state: &mut FriendsState, action: &mut Option<Action>) {
    if !state.rename_open {
        if widgets::ghost_button(ui, "Changer de pseudo") {
            state.rename_open = true;
            state.rename_text = state.username.clone().unwrap_or_default();
        }
        return;
    }
    ui.label(widgets::dim(
        "3 à 24 caractères : lettres, chiffres, _, - et . ; vos amis verront le nouveau \
         pseudo. Un seul changement par jour.",
    ));
    let mut submit = false;
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.rename_text)
                .hint_text("Nouveau pseudo")
                .desired_width(220.0),
        );
        submit |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let ready = can_rename(state);
        submit |= widgets::primary_button(ui, "Valider", ready);
        submit &= ready;
        if widgets::ghost_button(ui, "Annuler") {
            state.rename_open = false;
        }
    });
    if submit {
        *action = Some(Action::Rename(state.rename_text.trim().to_string()));
    }
}

/// Le bouton « Valider » est actif : rien en cours, et un pseudo différent de l'actuel.
fn can_rename(state: &FriendsState) -> bool {
    let wanted = state.rename_text.trim();
    !state.busy() && !wanted.is_empty() && state.username.as_deref() != Some(wanted)
}

/// Ajout par pseudo, demandes reçues, demandes envoyées.
fn requests_panels(ui: &mut Ui, state: &mut FriendsState, ctx: &egui::Context) {
    let mut action = None;
    widgets::panel(ui, "Ajouter un ami", |ui| {
        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.add_name)
                    .hint_text("Pseudo exact")
                    .desired_width(220.0),
            );
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ready = !state.busy() && !state.add_name.trim().is_empty();
            if (widgets::primary_button(ui, "Envoyer", ready) || enter) && ready {
                action = Some(Action::SendRequest(std::mem::take(&mut state.add_name)));
            }
        });
    });
    ui.add_space(12.0);

    let incoming = &state.lists.requests.incoming;
    widgets::panel(
        ui,
        &format!("Demandes reçues · {}", incoming.len()),
        |ui| {
            if incoming.is_empty() {
                ui.label(widgets::dim("Aucune demande en attente."));
            }
            for player in incoming {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&player.username).color(theme::TEXT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, "Refuser") {
                            action = Some(Action::Decline(player.username.clone()));
                        }
                        if widgets::primary_button(ui, "Accepter", true) {
                            action = Some(Action::Accept(player.username.clone()));
                        }
                    });
                });
            }
        },
    );
    ui.add_space(12.0);

    let outgoing = &state.lists.requests.outgoing;
    widgets::panel(
        ui,
        &format!("Demandes envoyées · {}", outgoing.len()),
        |ui| {
            if outgoing.is_empty() {
                ui.label(widgets::dim("Aucune demande envoyée."));
            }
            for player in outgoing {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&player.username).color(theme::TEXT));
                    ui.label(widgets::dim("en attente").size(12.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::ghost_button(ui, "Annuler") {
                            action = Some(Action::Cancel(player.username.clone()));
                        }
                    });
                });
            }
        },
    );
    if let Some(action) = action {
        if !state.busy() {
            state.start(ctx, action);
        }
    }
}

/// Adresse du hub, modifiable.
fn hub_panel(ui: &mut Ui, state: &mut FriendsState, ctx: &egui::Context) {
    widgets::panel(ui, "Rafthub", |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.url_text)
                    .hint_text(DEFAULT_HUB_URL)
                    .desired_width(320.0),
            );
            if widgets::primary_button(ui, "Enregistrer", !state.busy()) {
                state.apply_url(ctx);
            }
            if widgets::ghost_button(ui, "Par défaut") && !state.busy() {
                state.url_text = DEFAULT_HUB_URL.to_string();
                state.apply_url(ctx);
            }
        });
        let status = match &state.url {
            Ok(url) => format!("En service : {} ({url})", url.host()),
            Err(e) => format!("Adresse refusée : {e}"),
        };
        ui.label(widgets::dim(status).size(12.0));
        ui.label(
            widgets::dim(
                "Aucun Rafthub public pour l'instant : par défaut, celui lancé sur ce PC. \
                 Une adresse distante doit être en https://. Un lien raftup://hub/<hôte> \
                 change aussi de Rafthub, après confirmation.",
            )
            .size(12.0),
        );
    });
}

/// Boîte « Passer sur le hub <hôte> ? », dessinée par-dessus n'importe quel écran tant
/// qu'un lien `raftup://hub/…` attend la réponse du joueur.
pub fn switch_dialog(ctx: &egui::Context, state: &mut FriendsState) {
    let Some(url) = state.pending_switch().cloned() else {
        return;
    };
    let current = state.host().unwrap_or("aucun").to_string();
    let busy = state.busy();
    let mut confirm = false;
    let mut cancel = false;
    // `Modal` grise le reste de la fenêtre et bloque les clics derrière la boîte.
    let modal = egui::Modal::new(egui::Id::unique("rafthub_switch")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(widgets::dim("RAFTHUB").size(12.0));
        ui.label(
            RichText::new(format!("Passer sur le hub {} ?", url.host()))
                .size(20.0)
                .family(theme::bold_family())
                .color(theme::TEXT),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Adresse : {url}")).color(theme::TEXT));
        ui.label(widgets::dim(format!("Hub actuel : {current}")));
        ui.add_space(6.0);
        ui.label(widgets::dim(
            "Comptes et amis sont propres à chaque hub. La session du hub actuel \
             reste rangée sur ce PC : vous pourrez y revenir.",
        ));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            confirm = widgets::primary_button(ui, "Passer sur ce hub", !busy);
            cancel = widgets::ghost_button(ui, "Rester");
            if busy {
                ui.spinner();
            }
        });
    });
    // Échap ou clic à côté de la boîte : comme « Rester ».
    if cancel || modal.should_close() {
        state.cancel_switch();
    } else if confirm {
        state.confirm_switch(ctx);
    }
}

/// Pastille de couleur de la présence.
fn presence_dot(ui: &mut Ui, status: Status) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let color = presence_color(status);
    if status == Status::Offline {
        ui.painter()
            .circle_stroke(rect.center(), 4.0, egui::Stroke::new(1.0, color));
    } else {
        ui.painter().circle_filled(rect.center(), 4.0, color);
    }
}

fn presence_color(status: Status) -> egui::Color32 {
    match status {
        Status::Online => theme::GREEN,
        Status::InGame => theme::ACCENT,
        Status::Offline => theme::TEXT_DIM,
    }
}

/// Texte de la présence (« En partie · APQuest »).
fn presence_text(presence: &Presence) -> (String, egui::Color32) {
    let text = match (presence.status, &presence.game) {
        (Status::Online, _) => "En ligne".to_string(),
        (Status::InGame, Some(game)) => format!("En partie · {game}"),
        (Status::InGame, None) => "En partie".to_string(),
        (Status::Offline, _) => "Hors ligne".to_string(),
    };
    (text, presence_color(presence.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn friend(name: &str, presence: Presence) -> Friend {
        Friend {
            username: name.to_string(),
            presence,
        }
    }

    #[test]
    fn amis_ranges_par_presence_puis_pseudo() {
        let mut friends = vec![
            friend("zoé", Presence::offline()),
            friend("Bob", Presence::online()),
            friend("alice", Presence::online()),
            friend("Max", Presence::in_game("APQuest")),
        ];
        sort_friends(&mut friends);
        let names: Vec<&str> = friends.iter().map(|f| f.username.as_str()).collect();
        assert_eq!(names, ["Max", "alice", "Bob", "zoé"]);
    }

    #[test]
    fn textes_de_presence() {
        assert_eq!(presence_text(&Presence::online()).0, "En ligne");
        assert_eq!(
            presence_text(&Presence::in_game("Slime Rancher")).0,
            "En partie · Slime Rancher"
        );
        assert_eq!(presence_text(&Presence::offline()).0, "Hors ligne");
    }

    #[test]
    fn messages_d_erreur_en_clair() {
        let api = HubClientError::Api {
            status: 409,
            code: ErrorCode::UsernameTaken,
            message: "ce pseudo est déjà pris".to_string(),
        };
        assert_eq!(error_text(&api), "Ce pseudo est déjà pris");
        let internal = HubClientError::Api {
            status: 500,
            code: ErrorCode::Internal,
            message: "erreur interne".to_string(),
        };
        assert!(error_text(&internal).starts_with("Rafthub a rencontré"));
        assert!(error_text(&HubClientError::Network("refused".into())).contains("ne répond pas"));
    }

    #[test]
    fn messages_d_inscription_sur_invitation() {
        let api = |code| HubClientError::Api {
            status: 403,
            code,
            message: "texte du hub".to_string(),
        };
        assert_eq!(
            error_text(&api(ErrorCode::RegistrationClosed)),
            "Ce hub n'accepte plus d'inscriptions."
        );
        assert!(error_text(&api(ErrorCode::InviteRequired))
            .starts_with("Ce hub demande un code d'invitation"));
        assert_eq!(
            error_text(&api(ErrorCode::InvalidInvite)),
            "Code d'invitation incorrect, expiré ou déjà utilisé."
        );
    }

    #[test]
    fn code_d_invitation_vide_pas_envoye() {
        assert_eq!(invite_code_or_none(""), None);
        assert_eq!(invite_code_or_none("   "), None);
        assert_eq!(
            invite_code_or_none(" k7qx-2m9p "),
            Some("k7qx-2m9p".to_string())
        );
    }

    /// État de test : hub local, jeton en mémoire, rien d'écrit dans `config.toml`.
    fn test_state() -> FriendsState {
        let mut state = FriendsState::new();
        // Port 1 : personne n'écoute, l'appel lancé en arrière-plan échoue tout de suite.
        let url = HubUrl::parse("http://127.0.0.1:1").unwrap();
        state.client = Some(HubClient::new(url.clone()));
        state.url_text = url.to_string();
        state.url = Ok(url);
        state.store = Arc::new(MemoryStore::new());
        state.persist = false;
        state.started = true;
        state
    }

    #[test]
    fn lien_hub_demande_confirmation() {
        let mut state = test_state();
        let url = HubUrl::parse("https://hub.exemple.org").unwrap();
        state.offer_switch(url.clone());
        // Rien n'a changé tant que le joueur n'a pas répondu.
        assert_eq!(state.pending_switch(), Some(&url));
        assert_eq!(state.host(), Some("127.0.0.1:1"));
        // « Rester » : la proposition disparaît, le hub reste le même.
        state.cancel_switch();
        assert_eq!(state.pending_switch(), None);
        assert_eq!(state.host(), Some("127.0.0.1:1"));
    }

    #[test]
    fn confirmation_change_de_hub() {
        let mut state = test_state();
        state.username = Some("Mimi".into());
        state.lists.friends = vec![friend("a", Presence::online())];
        let url = HubUrl::parse("https://127.0.0.1:2").unwrap();
        state.offer_switch(url.clone());
        let ctx = egui::Context::default();
        assert!(state.confirm_switch(&ctx));
        // Nouveau hub en service, session et listes de l'ancien effacées de l'écran,
        // reprise de session lancée en arrière-plan.
        assert_eq!(state.pending_switch(), None);
        assert_eq!(state.url.as_ref().ok(), Some(&url));
        assert_eq!(state.url_text, "https://127.0.0.1:2");
        assert_eq!(state.username, None);
        assert!(state.lists.friends.is_empty());
        assert!(state.busy());
        // Un autre lien pendant l'appel : la confirmation attend la fin de l'appel.
        state.offer_switch(HubUrl::parse("https://autre.exemple.org").unwrap());
        assert!(!state.confirm_switch(&ctx));
        assert!(state.pending_switch().is_some());
    }

    #[test]
    fn lien_refuse_reste_affiche_apres_la_reprise_de_session() {
        // Bug vu en capture : la reprise de session du démarrage (aucun jeton rangé)
        // effaçait le bandeau « Lien Rafthub refusé ».
        let mut state = test_state();
        state.link_refused(&launcher_core::rafthub::HubLinkError::MissingHost);
        let (sender, receiver) = mpsc::channel();
        let client = state.client.take().unwrap();
        sender
            .send(Outcome {
                client,
                result: Ok(Done::LoggedOut { message: None }),
            })
            .unwrap();
        state.job = Some(receiver);
        state.poll();
        assert!(matches!(&state.notice, Some(Notice::Error(m)) if m.contains("refusé")));
    }

    #[test]
    fn lien_vers_le_hub_actuel_sans_question() {
        let mut state = test_state();
        state.offer_switch(HubUrl::parse("http://127.0.0.1:1").unwrap());
        assert_eq!(state.pending_switch(), None);
        assert!(matches!(&state.notice, Some(Notice::Info(m)) if m.contains("déjà")));
    }

    #[test]
    fn resume_compte_les_amis_presents() {
        let mut state = FriendsState::new();
        state.username = Some("Mimi".into());
        state.lists.friends = vec![
            friend("a", Presence::online()),
            friend("b", Presence::in_game("APQuest")),
            friend("c", Presence::offline()),
        ];
        let summary = state.summary();
        assert_eq!(summary.online, 2);
        assert_eq!(summary.username.as_deref(), Some("Mimi"));
    }

    #[test]
    fn message_de_pseudo_change_trop_tot() {
        let error = HubClientError::Api {
            status: 429,
            code: ErrorCode::UsernameChangeTooSoon,
            message: "texte du hub".to_string(),
        };
        assert!(error_text(&error).contains("moins de 24 heures"));
    }

    #[test]
    fn valider_seulement_un_pseudo_different() {
        let mut state = test_state();
        state.username = Some("Mimi".into());
        state.rename_text = " Mimi ".into();
        assert!(!can_rename(&state));
        state.rename_text = "   ".into();
        assert!(!can_rename(&state));
        state.rename_text = "Mireille".into();
        assert!(can_rename(&state));
    }

    #[test]
    fn pseudo_change_met_a_jour_l_ecran() {
        let mut state = test_state();
        state.username = Some("Mimi".into());
        state.rename_open = true;
        state.rename_text = "Mireille".into();
        let (sender, receiver) = mpsc::channel();
        let client = state.client.take().unwrap();
        sender
            .send(Outcome {
                client,
                result: Ok(Done::Renamed {
                    username: "Mireille".into(),
                    lists: Lists::default(),
                }),
            })
            .unwrap();
        state.job = Some(receiver);
        state.poll();
        assert_eq!(state.username.as_deref(), Some("Mireille"));
        assert!(!state.rename_open);
        assert!(matches!(&state.notice, Some(Notice::Info(m)) if m.contains("Mireille")));
    }
}
