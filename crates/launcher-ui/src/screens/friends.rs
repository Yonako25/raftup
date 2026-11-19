//! Écran « Amis » : compte sur le hub, liste d'amis avec leur présence, demandes d'ami
//! (ADR 0002, 0035).
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
    },
    Logout,
    /// Renvoyer la présence et relire amis et demandes.
    Sync,
    SendRequest(String),
    Accept(String),
    Decline(String),
    Cancel(String),
    Remove(String),
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
    add_name: String,
    lists: Lists,
    notice: Option<Notice>,
    /// Dernière synchronisation réussie ou tentée.
    last_sync: Option<Instant>,
    /// Ami dont on a cliqué « Retirer » une première fois (deuxième clic = confirmation).
    confirm_remove: Option<String>,
    /// En debug : se connecter tout seul avec ces identifiants (macro de captures).
    dev_login: Option<(String, String)>,
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
        let url = HubUrl::parse(&url_text).map_err(|e| e.to_string());
        FriendsState {
            client: url.as_ref().ok().map(|u| HubClient::new(u.clone())),
            url_text,
            url,
            store,
            username: None,
            started: false,
            job: None,
            mode: Mode::Login,
            email: String::new(),
            password: String::new(),
            new_username: String::new(),
            add_name: String::new(),
            lists: Lists::default(),
            notice: None,
            last_sync: None,
            confirm_remove: None,
            dev_login,
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
                self.notice = Some(Notice::Error("L'appel au hub a échoué.".to_string()));
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
                self.notice = warning.map(Notice::Error);
            }
            Ok(Done::LoggedOut { message }) => {
                self.username = None;
                self.lists = Lists::default();
                self.notice = message.map(Notice::Info);
            }
            Ok(Done::Lists { lists, message }) => {
                self.lists = lists;
                self.last_sync = Some(Instant::now());
                if let Some(message) = message {
                    self.notice = Some(Notice::Info(message));
                }
            }
            Err(message) => self.notice = Some(Notice::Error(message)),
        }
    }

    /// Change d'adresse de hub : la session de l'ancien hub est laissée de côté.
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
        save_url(&url);
        self.client = Some(HubClient::new(url.clone()));
        self.url = Ok(url);
        self.username = None;
        self.lists = Lists::default();
        self.notice = None;
        // Une session déjà rangée pour ce hub-là ? On la reprend.
        self.started = true;
        self.start(ctx, Action::Restore);
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
        } => match client.register(&email, &password, &username) {
            Ok(_) => login(client, store, &email, &password),
            Err(error) => Outcome {
                client,
                result: Err(error_text(&error)),
            },
        },
        Action::Logout => {
            let mut client = client;
            let remote = client.logout();
            let local = store.clear(&url);
            let message = match (remote, local) {
                (Ok(()), Ok(())) => "Déconnecté.".to_string(),
                (Err(_), _) => "Déconnecté sur ce PC ; le hub n'a pas pu être prévenu.".to_string(),
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
            "Le hub ne répond pas : vérifiez son adresse (en bas de l'écran) ou votre connexion."
                .to_string()
        }
        HubClientError::Api {
            code: ErrorCode::Internal,
            ..
        } => "Le hub a rencontré une erreur. Réessayez plus tard.".to_string(),
        // Les messages du hub sont déjà en français et sans secret.
        HubClientError::Api { message, .. } => capitalize(message),
        other => capitalize(&other.to_string()),
    }
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
        super::header(ui, "Communauté", "Amis", |ui| {
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
            "Un compte Raftup sert à retrouver vos amis et à voir qui joue. \
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
                ui.label("E-mail");
                ui.add(
                    egui::TextEdit::singleline(&mut state.email)
                        .hint_text("vous@exemple.org")
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
            ui.label(widgets::dim(format!("Connecté en tant que {name}")));
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
    widgets::panel(ui, "Adresse du hub", |ui| {
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
            Ok(url) => format!("En service : {url}"),
            Err(e) => format!("Adresse refusée : {e}"),
        };
        ui.label(widgets::dim(status).size(12.0));
        ui.label(
            widgets::dim(
                "Aucun hub public pour l'instant : par défaut, celui lancé sur ce PC. \
                 Une adresse distante doit être en https://.",
            )
            .size(12.0),
        );
    });
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
        assert!(error_text(&internal).starts_with("Le hub a rencontré"));
        assert!(error_text(&HubClientError::Network("refused".into())).contains("ne répond pas"));
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
}
