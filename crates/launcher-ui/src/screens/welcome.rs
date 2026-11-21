//! Assistant du premier lancement (plan de l'interface, étape 5), en trois pas :
//! 1. le compte Rafthub (se connecter, ou continuer sans) ;
//! 2. les jeux trouvés sur ce PC ;
//! 3. le premier jeu prêt (créer un salon, ou finir l'installation).
//!
//! Il s'ouvre tout seul tant que `welcome_done` n'est pas vrai dans `config.toml`.
//! « Passer » ou la fin de l'assistant l'y écrivent : il ne revient plus.

use egui::RichText;
use launcher_core::config;

use super::library::{load_paths, GameSummary};
use crate::app::Page;
use crate::theme;
use crate::widgets;

/// Nombre de pas de l'assistant.
const STEPS: usize = 3;

/// Où en est l'assistant.
#[derive(Debug, Default)]
pub struct WelcomeState {
    /// L'assistant est affiché.
    open: bool,
    /// Pas affiché, de 0 à `STEPS - 1`.
    step: usize,
}

impl WelcomeState {
    /// L'assistant ouvert au premier pas (tests, et premier lancement).
    pub fn opened() -> WelcomeState {
        WelcomeState {
            open: true,
            step: 0,
        }
    }

    /// L'assistant fermé.
    pub fn closed() -> WelcomeState {
        WelcomeState::default()
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Lit `config.toml` : l'assistant s'ouvre s'il n'a jamais été terminé ni passé. En
/// debug, les variables de la macro de captures (`RAFTUP_DEV_PAGE`,
/// `RAFTUP_DEV_CAPTURE`) le gardent fermé : il cacherait l'écran à capturer.
pub fn load() -> WelcomeState {
    if cfg!(debug_assertions)
        && (std::env::var_os("RAFTUP_DEV_PAGE").is_some()
            || std::env::var_os("RAFTUP_DEV_CAPTURE").is_some())
    {
        return WelcomeState::closed();
    }
    let Ok(paths) = load_paths() else {
        return WelcomeState::closed();
    };
    match config::load(&paths.config_file()) {
        Ok(loaded) if loaded.config.welcome_done => WelcomeState::closed(),
        Ok(_) => WelcomeState::opened(),
        // Config illisible : on n'impose pas l'assistant, l'erreur va au journal.
        Err(e) => {
            tracing::warn!("assistant du premier lancement : config non lue : {e}");
            WelcomeState::closed()
        }
    }
}

/// Écrit `welcome_done = true` : l'assistant ne s'ouvrira plus. Un échec d'écriture
/// ne gêne rien (il reviendra au prochain lancement) : il va au journal.
fn remember_done() {
    let Ok(paths) = load_paths() else {
        return;
    };
    let file = paths.config_file();
    let result = config::load(&file).and_then(|loaded| {
        let mut config = loaded.config;
        if !config.welcome_done {
            config.welcome_done = true;
            config.save(&file)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!("assistant du premier lancement : fin non enregistrée : {e}");
    }
}

/// Ce que le dernier pas propose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstGame {
    /// Ce jeu est prêt : créer un salon.
    Ready { id: String, name: String },
    /// Ce jeu est trouvé mais ses mods ne sont pas installés : ouvrir sa fiche.
    ToInstall { id: String, name: String },
    /// Aucun jeu du catalogue sur ce PC.
    Nothing,
}

/// Le premier jeu prêt, sinon le premier jeu trouvé, dans l'ordre du catalogue.
pub fn first_game(games: &[GameSummary]) -> FirstGame {
    if let Some(game) = games.iter().find(|g| g.ready) {
        return FirstGame::Ready {
            id: game.id.clone(),
            name: game.name.clone(),
        };
    }
    match games.iter().find(|g| g.detected) {
        Some(game) => FirstGame::ToInstall {
            id: game.id.clone(),
            name: game.name.clone(),
        },
        None => FirstGame::Nothing,
    }
}

/// Ce que l'assistant demande à l'application à sa fermeture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// Page à ouvrir.
    pub page: Option<Page>,
    /// Jeu à sélectionner dans la Bibliothèque.
    pub select: Option<String>,
}

/// Dessine l'assistant par-dessus l'écran, s'il est ouvert.
/// `username` : le pseudo du compte Rafthub connecté, s'il y en a un.
pub fn show(
    ctx: &egui::Context,
    state: &mut WelcomeState,
    username: Option<&str>,
    games: &[GameSummary],
) -> Outcome {
    let mut outcome = Outcome::default();
    if !state.open {
        return outcome;
    }
    let mut finish = false;
    // `Modal` grise le reste de la fenêtre et bloque les clics derrière la boîte.
    egui::Modal::new(egui::Id::unique("assistant_bienvenue")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(widgets::caps(&format!("Bienvenue · {} / {STEPS}", state.step + 1)));
        match state.step {
            0 => {
                ui.label(widgets::title("Ton compte Rafthub", 22.0));
                ui.label(widgets::dim(
                    "Un compte sert à retrouver tes amis et à les inviter. Tu peux jouer sans.",
                ));
                ui.add_space(10.0);
                match username {
                    Some(name) => {
                        ui.label(
                            RichText::new(format!("Connecté en tant que {name}"))
                                .color(theme::GREEN),
                        );
                    }
                    None => {
                        if widgets::ghost_button(ui, "Se connecter") {
                            outcome.page = Some(Page::Friends);
                            finish = true;
                        }
                    }
                }
            }
            1 => {
                ui.label(widgets::title("Tes jeux", 22.0));
                let found: Vec<&GameSummary> = games.iter().filter(|g| g.detected).collect();
                if found.is_empty() {
                    ui.label(widgets::dim(
                        "Aucun jeu du catalogue trouvé sur ce PC. Dans la Bibliothèque, tu pourras indiquer le dossier d'un jeu.",
                    ));
                }
                for game in found {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&game.name).color(theme::TEXT));
                        ui.label(widgets::dim(&game.label));
                    });
                }
            }
            _ => {
                ui.label(widgets::title("Ton premier jeu", 22.0));
                ui.add_space(6.0);
                match first_game(games) {
                    FirstGame::Ready { id, name } => {
                        ui.label(format!(
                            "{name} est prêt : crée un salon pour jouer avec tes amis."
                        ));
                        ui.add_space(6.0);
                        if widgets::primary_button(ui, "Créer un salon", true) {
                            outcome.select = Some(id);
                            outcome.page = Some(Page::Create);
                            finish = true;
                        }
                    }
                    FirstGame::ToInstall { id, name } => {
                        ui.label(format!(
                            "{name} est sur ce PC : sa fiche installe ses mods en un clic."
                        ));
                        ui.add_space(6.0);
                        if widgets::primary_button(ui, &format!("Ouvrir {name}"), true) {
                            outcome.select = Some(id);
                            outcome.page = Some(Page::Library);
                            finish = true;
                        }
                    }
                    FirstGame::Nothing => {
                        ui.label("La Bibliothèque montre les jeux pris en charge et ce qu'il faut pour chacun.");
                        ui.add_space(6.0);
                        if widgets::primary_button(ui, "Ouvrir la Bibliothèque", true) {
                            outcome.page = Some(Page::Library);
                            finish = true;
                        }
                    }
                }
            }
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if widgets::ghost_button(ui, "Passer") {
                finish = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.step + 1 < STEPS {
                    let next = if state.step == 0 && username.is_none() {
                        "Continuer sans compte"
                    } else {
                        "Suivant"
                    };
                    if widgets::primary_button(ui, next, true) {
                        state.step += 1;
                    }
                } else if widgets::ghost_button(ui, "Terminer") {
                    finish = true;
                }
                if state.step > 0 && widgets::ghost_button(ui, "Retour") {
                    state.step -= 1;
                }
            });
        });
    });
    if finish {
        state.open = false;
        // Pas d'écriture sur le disque dans les tests : le vrai dossier du joueur.
        if !cfg!(test) {
            remember_done();
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::library::Tone;

    fn game(id: &str, detected: bool, ready: bool) -> GameSummary {
        GameSummary {
            id: id.to_string(),
            name: id.to_uppercase(),
            ap_game: id.to_string(),
            authors: Vec::new(),
            label: String::new(),
            tone: Tone::Dim,
            ready,
            detected,
            from_registry: false,
            trust: None,
        }
    }

    #[test]
    fn premier_jeu_pret_puis_trouve() {
        assert_eq!(first_game(&[]), FirstGame::Nothing);
        assert_eq!(first_game(&[game("a", false, false)]), FirstGame::Nothing);
        assert_eq!(
            first_game(&[game("a", true, false), game("b", true, true)]),
            FirstGame::Ready {
                id: "b".into(),
                name: "B".into()
            }
        );
        assert_eq!(
            first_game(&[game("a", false, false), game("b", true, false)]),
            FirstGame::ToInstall {
                id: "b".into(),
                name: "B".into()
            }
        );
    }
}
