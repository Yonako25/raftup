//! Section « À reprendre sur ce PC » de l'écran « En cours » (revue d'ergonomie du
//! launcher, point 21) : les parties générées ici et arrêtées, avec « Reprendre ».
//!
//! « Reprendre » relance la partie tout de suite, avec les réglages du serveur de
//! « Héberger un fichier » (port, adresse, chiffrement) et SANS mot de passe de salle :
//! Raftup ne l'enregistre nulle part (choix de l'utilisatrice, 24 novembre 2026). La
//! sauvegarde `.raftup` posée à côté de la seed est reprise d'office.
//!
//! La liste vient de `launcher_core::past_games`, lue dans un fil à part (elle ouvre
//! chaque seed) puis gardée 30 s.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use egui::{RichText, Ui};
use launcher_core::past_games::{self, PastGame};
use launcher_core::Paths;

use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::Shared;

/// Durée pendant laquelle la liste lue est gardée avant d'être relue.
const STALE_AFTER: Duration = Duration::from_secs(30);

/// État de la section : la dernière liste lue, la lecture en cours, la reprise demandée.
#[derive(Default)]
pub struct PastGames {
    /// Dossiers de Raftup ; `None` : rien à lire (dossiers introuvables).
    paths: Option<Paths>,
    /// Résultat de la dernière lecture. `Arc<Mutex<…>>` : le fil de lecture l'écrit, l'écran
    /// le lit ; le verrou empêche les deux de le faire en même temps.
    games: Arc<Mutex<Vec<PastGame>>>,
    /// Vrai pendant une lecture (`AtomicBool` : partagé entre fils sans verrou).
    loading: Arc<AtomicBool>,
    /// Début de la dernière lecture lancée.
    loaded_at: Option<Instant>,
    /// Seed dont « Reprendre » a été cliqué, reprise par `App`.
    resume: Option<PathBuf>,
}

impl PastGames {
    pub fn new(paths: Option<Paths>) -> PastGames {
        PastGames {
            paths,
            ..PastGames::default()
        }
    }

    /// Une section déjà remplie, sans lecture du disque (tests).
    #[cfg(test)]
    pub fn with_games(games: Vec<PastGame>) -> PastGames {
        PastGames {
            games: Arc::new(Mutex::new(games)),
            loaded_at: Some(Instant::now()),
            ..PastGames::default()
        }
    }

    /// Relit la liste dans un fil à part si elle a plus de 30 s (ou n'a jamais été lue).
    fn refresh_if_stale(&mut self, ctx: &egui::Context) {
        let fresh = self.loaded_at.is_some_and(|at| at.elapsed() < STALE_AFTER);
        if fresh || self.loading.load(Ordering::Relaxed) {
            return;
        }
        let Some(paths) = self.paths.clone() else {
            return;
        };
        self.loaded_at = Some(Instant::now());
        self.loading.store(true, Ordering::Relaxed);
        // Clones des `Arc` : le fil en garde une copie, l'écran l'autre.
        let games = Arc::clone(&self.games);
        let loading = Arc::clone(&self.loading);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let list = past_games::list(&paths);
            if let Ok(mut slot) = games.lock() {
                *slot = list;
            }
            loading.store(false, Ordering::Relaxed);
            ctx.request_repaint();
        });
    }

    /// Les parties à montrer : toutes sauf celle qui tourne déjà sur ce PC.
    fn visible(&self, shared: &Shared) -> Vec<PastGame> {
        let running = shared
            .status
            .as_ref()
            .map(|status| status.seed.seed_name.as_str());
        match self.games.lock() {
            Ok(games) => games
                .iter()
                .filter(|game| Some(game.summary.seed_name.as_str()) != running)
                .cloned()
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Nombre de parties à montrer (pour savoir si « En cours » est vide).
    pub fn count(&self, shared: &Shared) -> usize {
        self.visible(shared).len()
    }

    /// La seed à reprendre, une seule fois.
    pub fn take_resume(&mut self) -> Option<PathBuf> {
        self.resume.take()
    }
}

/// Secondes depuis 1970, comme `format::ago` les attend.
fn seconds(time: SystemTime) -> f64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// La section, sous les parties en ligne. Ne dessine rien s'il n'y a aucune partie.
pub fn show(ui: &mut Ui, state: &mut PastGames, shared: &Shared) {
    state.refresh_if_stale(ui.ctx());
    let games = state.visible(shared);
    if games.is_empty() {
        return;
    }
    // Un seul serveur à la fois sur ce PC : pas de reprise pendant qu'une partie tourne.
    let busy = shared.status.is_some() || shared.starting;
    ui.add_space(18.0);
    ui.label(widgets::caps("À reprendre sur ce PC"));
    ui.add_space(6.0);
    super::matches::list_frame(ui, |ui| {
        for (index, game) in games.iter().enumerate() {
            if index > 0 {
                ui.separator();
            }
            // `push_id` : chaque ligne a son bouton « Reprendre ».
            ui.push_id(&game.seed, |ui| {
                if row(ui, game, busy) {
                    state.resume = Some(game.seed.clone());
                }
            });
        }
    });
}

/// Une partie arrêtée. Renvoie vrai au clic sur « Reprendre ».
fn row(ui: &mut Ui, game: &PastGame, busy: bool) -> bool {
    let players: Vec<_> = game.summary.slots.iter().filter(|s| !s.is_group).collect();
    let first_game = players.first().map(|s| s.game.as_str()).unwrap_or("");
    let now = format::now();
    let when = match (game.last_saved, game.generated_at) {
        (Some(saved), _) => format!("dernière partie {}", format::ago(now, Some(seconds(saved)))),
        (None, Some(generated)) => format!(
            "jamais lancée · générée {}",
            format::ago(now, Some(seconds(generated)))
        ),
        (None, None) => "jamais lancée".to_string(),
    };
    let mut clicked = false;
    ui.horizontal(|ui| {
        widgets::badge(ui, 52.0, &format::initials(first_game), false);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                RichText::new(format::games_title(players.iter().map(|s| s.game.as_str())))
                    .size(17.0)
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            ui.label(widgets::dim(format!(
                "{} · {when}",
                widgets::plural(players.len(), "joueur", "joueurs")
            )));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui.add_enabled(
                !busy,
                egui::Button::new(
                    RichText::new("Reprendre")
                        .color(egui::Color32::from_rgb(0x08, 0x12, 0x22))
                        .strong()
                        .family(theme::bold_family()),
                )
                .fill(theme::ACCENT)
                .min_size(egui::vec2(90.0, 0.0)),
            );
            clicked = response.clicked();
            response
                .on_disabled_hover_text("Une partie tourne déjà sur ce PC : arrête-la d'abord.")
                .on_hover_text("Relance la partie sur ce PC, sans mot de passe de salle.");
        });
    });
    clicked
}
