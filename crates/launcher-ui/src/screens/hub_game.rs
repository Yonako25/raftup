//! Parties hébergées par Rafthub (ADR 0052, étape 2) : envoi de la seed générée sur ce
//! PC, puis écran « Partie en cours » d'une partie qui tourne sur le hub.
//!
//! Tout passe par `launcher_core::hub_client` (le hub est en AGPL, ADR 0003). Chaque
//! appel part dans un **thread à part**, avec son propre client (adresse + jeton du
//! compte connecté, [`HubAccess`]) : le thread du dessin n'attend jamais le réseau.
//!
//! - [`Upload`] : l'envoi de la seed (`POST /v1/games`), lancé par « Créer une partie »
//!   quand l'hôte a choisi « Héberger sur Rafthub » ; en cas d'échec, la seed est gardée
//!   pour « Réessayer » (sur Rafthub ou sur ce PC).
//! - [`HubGames`] : les parties hébergées du compte (`GET /v1/games`, relu à la
//!   connexion) et celle qu'on regarde, dont l'état est relu toutes les 5 s
//!   (`GET /v1/games/{id}`) : joueurs connectés, avancement par slot.
//!
//! L'adresse rendue par le hub est `hôte:port`, sans schéma, avec l'empreinte du
//! certificat : les liens d'invitation portent `wss://hôte:port` et `fp=…`, comme une
//! partie chiffrée par Raftup en self-host (ADR 0043). « Rejoindre » n'a donc rien de
//! particulier à faire pour une partie hébergée.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use launcher_core::generation::GenerateOutput;
use launcher_core::hub_client::{
    CreatedGame, ErrorCode, GameStatus, GameUpload, HostedGame, HubClient, HubClientError, HubUrl,
    SessionToken,
};
use launcher_core::SlotStatus;

use super::matches::{fraction, progress};
use super::running::LaunchRequest;
use crate::app::Page;
use crate::theme;
use crate::widgets;

/// Toutes les combien on relit l'état de la partie regardée.
const POLL_EVERY: Duration = Duration::from_secs(5);

/// Longueur maximale du nom d'une partie, côté hub.
const MAX_NAME_CHARS: usize = 64;

/// Durée d'affichage de « Copié ».
const COPIED_FOR: Duration = Duration::from_secs(2);

/// De quoi parler au hub au nom du compte connecté : son adresse et le jeton de session.
///
/// `Debug` dérivé sans risque : celui de `SessionToken` masque le jeton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubAccess {
    pub url: HubUrl,
    pub token: SessionToken,
}

impl HubAccess {
    /// Un client neuf, pour un appel dans un thread.
    pub fn client(&self) -> HubClient {
        HubClient::with_token(self.url.clone(), self.token.clone())
    }

    /// Hôte du hub, pour l'afficher (« hub.raftup-games.com »).
    pub fn host(&self) -> &str {
        self.url.host()
    }
}

/// Nom de la partie sur le hub : celui que l'hôte a tapé, sinon « Partie de <pseudo> »,
/// sinon « Partie <seed> » ; coupé à 64 caractères.
pub fn game_name(typed: &str, my_name: &str, seed_name: &str) -> String {
    let typed = typed.trim();
    let my_name = my_name.trim();
    let name = if !typed.is_empty() {
        typed.to_string()
    } else if !my_name.is_empty() {
        format!("Partie de {my_name}")
    } else {
        format!("Partie {seed_name}")
    };
    // Sans caractère de contrôle (refusé par le hub).
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect()
}

/// Message en français pour une erreur du hub sur une partie hébergée.
pub fn error_text(error: &HubClientError) -> String {
    const ON_PC: &str = "Tu peux aussi l'héberger sur ton PC.";
    if error.is_unauthorized() {
        return "Ta session Rafthub a expiré : reconnecte-toi dans « Amis », puis réessaie."
            .to_string();
    }
    match error {
        HubClientError::TooLarge { .. } => {
            format!("La seed est trop grosse pour Rafthub (32 Mio au plus). {ON_PC}")
        }
        HubClientError::File { detail, .. } => {
            format!("Impossible de lire la seed sur ce PC : {detail}")
        }
        HubClientError::Network(detail) => {
            format!("Rafthub est injoignable ({detail}). Vérifie ta connexion, puis réessaie.")
        }
        HubClientError::Api { code, message, .. } => match code {
            ErrorCode::GameLimitReached => "Tu héberges déjà le nombre maximal de parties sur \
                 Rafthub. Supprime-en une (écran « En cours »), ou héberge celle-ci sur ton PC."
                .to_string(),
            ErrorCode::HubFull => format!(
                "Rafthub héberge déjà autant de parties qu'il le peut. Réessaie plus tard. {ON_PC}"
            ),
            ErrorCode::TooManyPlayers => format!(
                "Trop de joueurs pour une partie hébergée par Rafthub (16 au plus). {ON_PC}"
            ),
            ErrorCode::SeedTooLarge => {
                format!("La seed est trop grosse pour Rafthub (32 Mio au plus). {ON_PC}")
            }
            ErrorCode::HostingDisabled => {
                format!("Ce Rafthub n'héberge pas de parties pour le moment. {ON_PC}")
            }
            ErrorCode::NoFreePort => format!(
                "Rafthub n'a plus de place (aucun port libre) pour une nouvelle partie. \
                 Réessaie plus tard. {ON_PC}"
            ),
            ErrorCode::InvalidSeed => format!("Rafthub n'a pas pu lire la seed : {message}"),
            ErrorCode::InvalidSave => {
                format!("Rafthub a refusé la sauvegarde : {message}")
            }
            ErrorCode::GameNotFound => "Cette partie n'existe plus sur Rafthub.".to_string(),
            ErrorCode::NotGameHost => "Seul l'hôte de la partie peut faire ça.".to_string(),
            _ => format!("Rafthub a refusé : {message}"),
        },
        other => format!("Rafthub a refusé : {other}"),
    }
}

// ---------------------------------------------------------------------------
// Envoi de la seed (« Créer une partie »).
// ---------------------------------------------------------------------------

/// Où en est l'envoi de la seed générée.
#[derive(Default)]
pub enum Upload {
    #[default]
    Idle,
    /// Envoi en cours, dans un thread ; la réponse arrivera sur `receiver`.
    Sending {
        output: Box<GenerateOutput>,
        receiver: Receiver<Result<CreatedGame, String>>,
    },
    /// Échec ; la seed est gardée pour « Réessayer ».
    Failed {
        output: Box<GenerateOutput>,
        message: String,
    },
}

impl Upload {
    pub fn is_busy(&self) -> bool {
        matches!(self, Upload::Sending { .. })
    }

    /// La seed d'un envoi raté, à réutiliser pour « Réessayer ».
    pub fn failed_output(&self) -> Option<&GenerateOutput> {
        match self {
            Upload::Failed { output, .. } => Some(output),
            _ => None,
        }
    }
}

/// Lance l'envoi de la seed au hub, dans un thread.
pub fn start_upload(
    ctx: &egui::Context,
    access: &HubAccess,
    output: GenerateOutput,
    name: String,
    password: Option<String>,
) -> Upload {
    let (sender, receiver) = mpsc::channel();
    let client = access.client();
    let seed = output.archipelago_file.clone();
    let ctx = ctx.clone();
    // `move` : le thread possède sa copie du client, du chemin et des textes.
    std::thread::spawn(move || {
        let result = GameUpload::from_files(&name, password.as_deref(), &seed, None)
            .and_then(|upload| client.create_game(&upload))
            .map_err(|e| error_text(&e));
        let _ = sender.send(result);
        ctx.request_repaint();
    });
    Upload::Sending {
        output: Box::new(output),
        receiver,
    }
}

/// Lit la réponse de l'envoi, sans attendre. Rend la partie créée une seule fois.
pub fn poll_upload(upload: &mut Upload) -> Option<CreatedGame> {
    let Upload::Sending { output, receiver } = upload else {
        return None;
    };
    let message = match receiver.try_recv() {
        Err(TryRecvError::Empty) => return None,
        Ok(Ok(created)) => {
            *upload = Upload::Idle;
            return Some(created);
        }
        Ok(Err(message)) => message,
        Err(TryRecvError::Disconnected) => "L'envoi à Rafthub s'est arrêté net.".to_string(),
    };
    let output = output.clone();
    *upload = Upload::Failed { output, message };
    None
}

/// Dessine le panneau de l'envoi (rien si `Idle`). Rend vrai au clic sur « Réessayer ».
pub fn show_upload(ui: &mut Ui, upload: &mut Upload) -> bool {
    let mut retry = false;
    let mut close = false;
    match upload {
        Upload::Idle => return false,
        Upload::Sending { output, .. } => {
            widgets::panel(ui, "Envoi sur Rafthub", |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!(
                        "Envoi de la seed {} à Rafthub, qui démarre la partie…",
                        output.seed_name
                    ));
                });
            });
        }
        Upload::Failed { output, message } => {
            widgets::panel(ui, "Envoi sur Rafthub", |ui| {
                ui.label(RichText::new(message.as_str()).color(theme::RED_TEXT));
                ui.label(
                    widgets::dim(format!(
                        "La seed {} est gardée : réessaie, ou choisis « Héberger sur mon PC » \
                         puis « Réessayer », sans régénérer.",
                        output.seed_name
                    ))
                    .size(13.0),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    retry = widgets::primary_button(ui, "Réessayer", true);
                    close = widgets::ghost_button(ui, "Fermer");
                });
            });
        }
    }
    ui.add_space(12.0);
    if close {
        *upload = Upload::Idle;
    }
    retry
}

// ---------------------------------------------------------------------------
// Suivi des parties hébergées (« En cours », « Partie en cours »).
// ---------------------------------------------------------------------------

/// La partie regardée.
struct Tracked {
    game: HostedGame,
    /// Mot de passe d'admin : rendu à la création, puis par chaque `GET` (hôte seul).
    admin_password: Option<String>,
    /// Dernier état lu ; `None` avant la première réponse.
    status: Option<GameStatus>,
}

/// Échec de la lecture de l'état : `gone` si la partie n'existe plus sur le hub.
struct StatusError {
    gone: bool,
    message: String,
}

/// Réponse d'un thread réseau.
enum Job {
    Listed(Result<Vec<HostedGame>, String>),
    Status {
        id: String,
        /// `Box` : l'état est gros à côté des autres réponses (clippy le demande).
        result: Result<Box<GameStatus>, StatusError>,
    },
    Deleted {
        id: String,
        result: Result<(), String>,
    },
}

/// Ce que le launcher garde des parties hébergées sur le hub.
#[derive(Default)]
pub struct HubGames {
    /// Accès en service (compte connecté à un hub qui héberge) ; `None` sinon.
    access: Option<HubAccess>,
    /// Les parties hébergées par ce compte.
    mine: Vec<HostedGame>,
    current: Option<Tracked>,
    /// Appel en cours (un seul à la fois).
    job: Option<Receiver<Job>>,
    last_poll: Option<Instant>,
    /// Erreur à montrer en haut de l'écran.
    notice: Option<String>,
    /// « Supprimer la partie » cliqué une fois (deuxième clic = confirmation).
    confirm_delete: bool,
    /// Le mot de passe d'admin est affiché en clair.
    show_admin: bool,
    copied_at: Option<Instant>,
    /// Clic sur « Lancer et connecter », pas encore traité par l'application.
    launch_request: Option<LaunchRequest>,
}

impl HubGames {
    /// Nombre de parties hébergées sur le hub (compteur « En cours »).
    pub fn count(&self) -> usize {
        self.mine.len()
    }

    /// Les parties hébergées du compte, dans l'ordre du hub.
    pub fn games(&self) -> &[HostedGame] {
        &self.mine
    }

    /// Tests de l'interface : une partie déjà créée, sans réseau.
    #[cfg(test)]
    pub(crate) fn with_game(created: CreatedGame, status: Option<GameStatus>) -> HubGames {
        let mut games = HubGames::default();
        adopt(&mut games, created);
        if let Some(tracked) = &mut games.current {
            tracked.status = status;
        }
        // Pas de lecture tout de suite : les tests n'ont pas de hub.
        games.last_poll = Some(Instant::now());
        games
    }

    fn start(&mut self, ctx: &egui::Context, work: impl FnOnce(HubClient) -> Job + Send + 'static) {
        if self.job.is_some() {
            return;
        }
        let Some(access) = &self.access else {
            return;
        };
        let client = access.client();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        // `Send + 'static` : la fermeture part dans un autre thread et ne garde aucune
        // référence vers l'écran.
        std::thread::spawn(move || {
            let _ = sender.send(work(client));
            ctx.request_repaint();
        });
        self.job = Some(receiver);
    }

    fn poll(&mut self) -> Option<Page> {
        let receiver = self.job.as_ref()?;
        let job = match receiver.try_recv() {
            Ok(job) => job,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                return None;
            }
        };
        self.job = None;
        match job {
            Job::Listed(Ok(games)) => {
                self.mine = games;
                self.notice = None;
            }
            // Liste illisible : on garde ce qu'on savait, sans bandeau (la page « Amis »
            // dit déjà si le hub est injoignable).
            Job::Listed(Err(_)) => {}
            Job::Status { id, result } => self.on_status(&id, result),
            Job::Deleted { id, result: Ok(()) } => {
                self.mine.retain(|g| g.id != id);
                if self.current.as_ref().is_some_and(|t| t.game.id == id) {
                    self.current = None;
                }
                self.notice = None;
                return Some(Page::InProgress);
            }
            Job::Deleted { result: Err(e), .. } => self.notice = Some(e),
        }
        None
    }

    fn on_status(&mut self, id: &str, result: Result<Box<GameStatus>, StatusError>) {
        let Some(tracked) = self.current.as_mut().filter(|t| t.game.id == id) else {
            return;
        };
        match result {
            Ok(status) => {
                tracked.game = status.game.clone();
                if status.admin_password.is_some() {
                    tracked.admin_password = status.admin_password.clone();
                }
                if let Some(known) = self.mine.iter_mut().find(|g| g.id == id) {
                    *known = status.game.clone();
                }
                tracked.status = Some(*status);
                self.notice = None;
            }
            Err(StatusError {
                gone: true,
                message,
            }) => {
                self.mine.retain(|g| g.id != id);
                self.current = None;
                self.notice = Some(message);
            }
            Err(StatusError { message, .. }) => self.notice = Some(message),
        }
    }
}

/// Une partie tout juste créée sur le hub devient celle qu'on regarde.
pub fn adopt(state: &mut HubGames, created: CreatedGame) {
    let game = created.game.clone();
    match state.mine.iter_mut().find(|g| g.id == game.id) {
        Some(known) => *known = game.clone(),
        None => state.mine.push(game.clone()),
    }
    state.current = Some(Tracked {
        game,
        admin_password: Some(created.admin_password),
        status: None,
    });
    state.last_poll = None;
    state.confirm_delete = false;
    state.show_admin = false;
}

/// Regarde la partie `id` (clic sur « Ouvrir » dans « En cours »).
pub fn select(state: &mut HubGames, id: &str) {
    if state.current.as_ref().is_some_and(|t| t.game.id == id) {
        return;
    }
    let Some(game) = state.mine.iter().find(|g| g.id == id).cloned() else {
        return;
    };
    state.current = Some(Tracked {
        game,
        admin_password: None,
        status: None,
    });
    state.last_poll = None;
    state.confirm_delete = false;
    state.show_admin = false;
}

/// Prend la demande « Lancer et connecter » en attente, s'il y en a une.
pub fn take_launch_request(state: &mut HubGames) -> Option<LaunchRequest> {
    state.launch_request.take()
}

/// À appeler à chaque image : réponses des threads, liste relue à la connexion, état de
/// la partie regardée relu toutes les 5 s. Rend la page où aller (après une suppression).
pub fn background(
    state: &mut HubGames,
    access: Option<HubAccess>,
    ctx: &egui::Context,
) -> Option<Page> {
    let goto = state.poll();
    if access != state.access {
        // Autre compte, autre hub ou déconnexion : on oublie ce qu'on savait.
        let same_account = matches!((&access, &state.access), (Some(a), Some(b)) if a.url == b.url);
        state.access = access;
        if !same_account {
            state.mine.clear();
            state.current = None;
            state.notice = None;
        }
        // Réponse d'un appel de l'ancien accès : ignorée.
        state.job = None;
        if state.access.is_some() {
            state.start(ctx, |client| {
                Job::Listed(client.games().map_err(|e| error_text(&e)))
            });
        }
        return goto;
    }
    let due = state.last_poll.is_none_or(|at| at.elapsed() >= POLL_EVERY);
    if let Some(id) = state.current.as_ref().map(|t| t.game.id.clone()) {
        if due && state.job.is_none() && state.access.is_some() {
            state.last_poll = Some(Instant::now());
            state.start(ctx, move |client| {
                let result = client
                    .game_status(&id)
                    .map(Box::new)
                    .map_err(|e| StatusError {
                        gone: e.code() == Some(&ErrorCode::GameNotFound),
                        message: error_text(&e),
                    });
                Job::Status { id, result }
            });
        }
        ctx.request_repaint_after(POLL_EVERY);
    }
    goto
}

/// Écran « Partie en cours » d'une partie hébergée sur Rafthub.
pub fn show(ui: &mut Ui, state: &mut HubGames) -> Option<Page> {
    let mut goto = None;
    let mut copy: Option<String> = None;
    let mut delete = false;
    super::page(ui, |ui| {
        let Some(tracked) = &state.current else {
            super::header(ui, "En cours", "Partie hébergée", |_| {});
            if let Some(message) = &state.notice {
                widgets::error_banner(ui, message);
                ui.add_space(10.0);
            }
            widgets::panel(ui, "Aucune partie hébergée ouverte", |ui| {
                ui.label(widgets::dim(
                    "Ouvre une partie hébergée sur Rafthub depuis l'écran « En cours ».",
                ));
                ui.add_space(8.0);
                if widgets::primary_button(ui, "Voir les parties en cours", true) {
                    goto = Some(Page::InProgress);
                }
            });
            return;
        };
        let game = &tracked.game;
        super::header(ui, "En cours · hébergée sur Rafthub", &game.name, |ui| {
            if widgets::ghost_button(ui, "Toutes les parties") {
                goto = Some(Page::InProgress);
            }
        });
        if let Some(message) = &state.notice {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }
        address_panel(ui, game, &mut copy);
        ui.add_space(12.0);
        if let Some(password) = &tracked.admin_password {
            admin_panel(ui, password, &mut state.show_admin, &mut copy);
            ui.add_space(12.0);
        }
        let request = crew_panel(ui, game, tracked.status.as_ref(), &mut copy);
        if request.is_some() {
            state.launch_request = request;
        }
        ui.add_space(12.0);
        widgets::panel(ui, "Supprimer la partie", |ui| {
            ui.label(widgets::dim(
                "Arrête le serveur sur Rafthub et efface la seed et la sauvegarde du hub. \
                 Les joueurs ne pourront plus se connecter.",
            ));
            ui.add_space(6.0);
            let busy = state.job.is_some();
            if state.confirm_delete {
                ui.horizontal(|ui| {
                    if !busy && widgets::danger_button(ui, "Confirmer la suppression") {
                        delete = true;
                    }
                    if widgets::ghost_button(ui, "Annuler") {
                        state.confirm_delete = false;
                    }
                });
            } else if widgets::danger_button(ui, "Supprimer la partie du hub") {
                state.confirm_delete = true;
            }
        });
    });
    if delete {
        state.confirm_delete = false;
        if let Some(id) = state.current.as_ref().map(|t| t.game.id.clone()) {
            state.start(ui.ctx(), move |client| {
                let result = client.delete_game(&id).map_err(|e| error_text(&e));
                Job::Deleted { id, result }
            });
        }
    }
    if let Some(text) = copy {
        ui.ctx().copy_text(text);
        state.copied_at = Some(Instant::now());
    }
    if let Some(at) = state.copied_at {
        if at.elapsed() < COPIED_FOR {
            ui.ctx()
                .request_repaint_after(COPIED_FOR.saturating_sub(at.elapsed()));
        } else {
            state.copied_at = None;
        }
    }
    goto
}

/// Adresse à donner aux mods, avec « Copier ».
fn address_panel(ui: &mut Ui, game: &HostedGame, copy: &mut Option<String>) {
    widgets::panel(ui, "Adresse de la partie", |ui| {
        ui.horizontal(|ui| {
            widgets::pill(ui, "Sur Rafthub", theme::TEXT, theme::ACCENT_BG);
            if game.is_active() {
                widgets::pill(ui, "● En ligne", theme::GREEN, theme::ACCENT_BG);
            } else {
                widgets::pill(ui, "Arrêtée", theme::AMBER, theme::ACCENT_BG);
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(widgets::mono(&game.address, 20.0));
            if widgets::ghost_button(ui, "Copier l'adresse") {
                *copy = Some(game.address.clone());
            }
        });
        ui.label(
            widgets::dim(
                "Partie chiffrée par Rafthub : le mod du jeu reçoit cette adresse (il essaie \
                 wss puis ws) ; les liens d'invitation vérifient le certificat du hub. La \
                 partie continue quand ton PC est éteint.",
            )
            .size(13.0),
        );
        let password = if game.has_password {
            "Mot de passe de la salle : celui que tu as choisi."
        } else {
            "Pas de mot de passe de salle."
        };
        ui.label(widgets::dim(password).size(13.0));
        if !game.is_active() {
            ui.label(
                RichText::new("Le serveur de cette partie ne tourne pas sur le hub en ce moment.")
                    .color(theme::AMBER),
            );
        }
    });
}

/// Mot de passe d'admin, masqué par défaut.
fn admin_panel(ui: &mut Ui, password: &str, shown: &mut bool, copy: &mut Option<String>) {
    widgets::panel(ui, "Mot de passe d'admin", |ui| {
        ui.label(
            widgets::dim("Pour les commandes « !admin » du serveur. Il n'est montré qu'à toi.")
                .size(13.0),
        );
        ui.horizontal(|ui| {
            let text = if *shown {
                password.to_string()
            } else {
                "•".repeat(12)
            };
            ui.label(widgets::mono(text, 16.0));
            let toggle = if *shown { "Masquer" } else { "Afficher" };
            if widgets::ghost_button(ui, toggle) {
                *shown = !*shown;
            }
            if widgets::ghost_button(ui, "Copier le mot de passe") {
                *copy = Some(password.to_string());
            }
        });
    });
}

/// Les joueurs : connexion, avancement, invitation et « Lancer et connecter ».
fn crew_panel(
    ui: &mut Ui,
    game: &HostedGame,
    status: Option<&GameStatus>,
    copy: &mut Option<String>,
) -> Option<LaunchRequest> {
    let mut request = None;
    widgets::panel(ui, "Équipage", |ui| {
        let Some(status) = status else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(widgets::dim("Lecture de l'état de la partie sur Rafthub…"));
            });
            return;
        };
        let players: Vec<&SlotStatus> = status.slots.iter().filter(|s| !s.is_group).collect();
        if players.is_empty() {
            ui.label(widgets::dim("Aucun joueur à afficher (partie arrêtée ?)."));
            return;
        }
        for slot in players {
            ui.horizontal(|ui| {
                let dot = if slot.connections > 0 {
                    theme::GREEN
                } else {
                    theme::TEXT_DIM
                };
                ui.label(RichText::new("●").color(dot));
                ui.label(RichText::new(&slot.name).strong().color(theme::TEXT));
                ui.label(widgets::dim(&slot.game).size(12.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, "Lancer et connecter") {
                        request = Some(LaunchRequest {
                            ap_game: slot.game.clone(),
                            address: game.address.clone(),
                            slot: slot.name.clone(),
                            needs_password: game.has_password,
                        });
                    }
                    if widgets::ghost_button(ui, "Copier l'invitation") {
                        *copy = game.invite_link(Some(&slot.name), Some(&slot.game)).ok();
                    }
                    ui.label(widgets::dim(progress(slot.checked, slot.total)).size(12.0));
                    let color = if slot.status >= 30 {
                        theme::GREEN
                    } else {
                        theme::ACCENT
                    };
                    widgets::bar(ui, 120.0, fraction(slot.checked, slot.total), color);
                });
            });
        }
    });
    request
}

/// Ligne d'une partie hébergée dans « En cours ». Rend vrai au clic sur « Ouvrir ».
pub fn row(ui: &mut Ui, game: &HostedGame) -> bool {
    let mut open = false;
    ui.horizontal(|ui| {
        widgets::badge(ui, 52.0, "RH", false);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                RichText::new(&game.name)
                    .size(17.0)
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            let (line, color) = if game.is_active() {
                ("En ligne sur Rafthub", theme::GREEN)
            } else {
                ("Arrêtée sur Rafthub", theme::AMBER)
            };
            ui.label(RichText::new(line).color(color));
            let players = game.players as usize;
            ui.label(
                widgets::dim(format!(
                    "{} · {} · seed {}",
                    widgets::plural(players, "joueur", "joueurs"),
                    game.address,
                    game.seed_name
                ))
                .size(13.0),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            open = widgets::primary_button(ui, "   Ouvrir   ", true);
        });
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(code: ErrorCode) -> HubClientError {
        HubClientError::Api {
            status: 400,
            code,
            message: "détail".into(),
        }
    }

    #[test]
    fn erreurs_du_hub_en_francais() {
        let cases = [
            (ErrorCode::GameLimitReached, "nombre maximal de parties"),
            (ErrorCode::HubFull, "autant de parties qu'il le peut"),
            (ErrorCode::TooManyPlayers, "16 au plus"),
            (ErrorCode::SeedTooLarge, "32 Mio au plus"),
            (ErrorCode::HostingDisabled, "n'héberge pas de parties"),
            (ErrorCode::NoFreePort, "aucun port libre"),
        ];
        for (code, expected) in cases {
            let text = error_text(&api(code.clone()));
            assert!(text.contains(expected), "{code:?} : {text}");
            // Jamais le code brut du hub à l'écran.
            assert!(!text.contains('_'), "{text}");
        }
        let too_large = HubClientError::TooLarge {
            what: "seed",
            bytes: 1,
        };
        assert!(error_text(&too_large).contains("32 Mio"));
        assert!(error_text(&api(ErrorCode::Unauthorized)).contains("reconnecte-toi"));
        assert!(error_text(&HubClientError::NotLoggedIn).contains("reconnecte-toi"));
    }

    fn output() -> GenerateOutput {
        GenerateOutput {
            archipelago_file: std::path::PathBuf::from("AP_1.archipelago"),
            zip_file: std::path::PathBuf::from("AP_1.zip"),
            seed_name: "1234".into(),
            players: Vec::new(),
            summary: launcher_core::SeedSummary {
                seed_name: "1234".into(),
                generator_version: "0.6.8".into(),
                slots: Vec::new(),
                players: 1,
                has_password: false,
                release_mode: "auto".into(),
                collect_mode: "auto".into(),
                hint_cost: 10,
            },
        }
    }

    /// Un refus du hub garde la seed, pour « Réessayer » sans régénérer.
    #[test]
    fn envoi_refuse_garde_la_seed() {
        let (sender, receiver) = mpsc::channel();
        let mut upload = Upload::Sending {
            output: Box::new(output()),
            receiver,
        };
        assert!(poll_upload(&mut upload).is_none());
        assert!(upload.is_busy());
        sender
            .send(Err(error_text(&api(ErrorCode::HubFull))))
            .unwrap();
        assert!(poll_upload(&mut upload).is_none());
        assert_eq!(
            upload.failed_output().map(|o| o.seed_name.as_str()),
            Some("1234")
        );
        match &upload {
            Upload::Failed { message, .. } => assert!(message.contains("Réessaie plus tard")),
            _ => panic!("l'envoi devait avoir échoué"),
        }
    }

    /// Le thread de l'envoi disparaît sans répondre : échec, la seed reste.
    #[test]
    fn envoi_interrompu() {
        let (sender, receiver) = mpsc::channel::<Result<CreatedGame, String>>();
        drop(sender);
        let mut upload = Upload::Sending {
            output: Box::new(output()),
            receiver,
        };
        assert!(poll_upload(&mut upload).is_none());
        assert!(upload.failed_output().is_some());
    }

    #[test]
    fn nom_de_la_partie() {
        assert_eq!(game_name("  Ma partie ", "Mimi", "42"), "Ma partie");
        assert_eq!(game_name("", " Mimi ", "42"), "Partie de Mimi");
        assert_eq!(game_name(" ", "", "42"), "Partie 42");
        assert_eq!(game_name(&"é".repeat(80), "", "").chars().count(), 64);
        assert_eq!(game_name("a\nb", "", ""), "ab");
    }
}
