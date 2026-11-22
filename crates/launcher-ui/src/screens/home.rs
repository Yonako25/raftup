//! Écran « Accueil » (plan de l'interface, étape 4) : ce qui se passe et quoi faire
//! ensuite, en un coup d'œil.
//!
//! - en haut, **le prochain pas** : une seule action proposée, choisie par
//!   [`next_step`] (fonction pure, testée sans interface) ;
//! - les parties en cours (celle que ce PC héberge) ;
//! - l'invitation reçue et pas encore suivie ;
//! - les jeux prêts à jouer.
//!
//! « Joués récemment » de la maquette attend un historique des parties, qui n'existe
//! pas encore (question notée dans la PR de l'étape 4).

use egui::{RichText, Ui};

use super::join::JoinState;
use launcher_core::art::ArtKind;

use super::library::{cover_card, CardLook, GameSummary, LibraryState};
use super::matches;
use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::Shared;

/// L'action mise en avant en haut de l'Accueil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextStep {
    /// Une partie tourne sur ce PC : y retourner.
    OpenHosted,
    /// Une invitation attend : la suivre.
    FollowInvite,
    /// Au moins un jeu est prêt : créer un salon avec lui (son nom).
    CreateLobby(String),
    /// Rien n'est prêt : aller installer un jeu.
    InstallGame,
}

/// Choisit le prochain pas : la partie en cours d'abord, puis l'invitation reçue,
/// puis un salon avec le premier jeu prêt ; sinon, installer un jeu.
pub fn next_step(hosting: bool, invite: bool, games: &[GameSummary]) -> NextStep {
    if hosting {
        return NextStep::OpenHosted;
    }
    if invite {
        return NextStep::FollowInvite;
    }
    match games.iter().find(|game| game.ready) {
        Some(game) => NextStep::CreateLobby(game.name.clone()),
        None => NextStep::InstallGame,
    }
}

/// Textes et page du prochain pas : (titre, explication, bouton, page).
fn step_texts(step: &NextStep) -> (String, &'static str, &'static str, Page) {
    match step {
        NextStep::OpenHosted => (
            "Une partie tourne sur ce PC".to_string(),
            "Tes amis peuvent la rejoindre avec l'adresse de la partie.",
            "Ouvrir la partie",
            Page::Hosted,
        ),
        NextStep::FollowInvite => (
            "Tu as reçu une invitation".to_string(),
            "Raftup a lu le lien : il reste à vérifier ton jeu et à te connecter.",
            "Voir l'invitation",
            Page::Join,
        ),
        NextStep::CreateLobby(name) => (
            format!("{name} est prêt"),
            "Crée un salon, invite tes amis, puis lance la partie.",
            "Créer un salon",
            Page::Create,
        ),
        NextStep::InstallGame => (
            "Prépare ton premier jeu".to_string(),
            "La Bibliothèque trouve tes jeux et installe leurs mods en un clic.",
            "Ouvrir la Bibliothèque",
            Page::Library,
        ),
    }
}

pub fn show(
    ui: &mut Ui,
    shared: &Shared,
    library: &mut LibraryState,
    join: &JoinState,
) -> Option<Page> {
    let mut goto = None;
    let games = library.summaries();
    let invite = join.pending_invite();
    let step = next_step(shared.status.is_some(), invite.is_some(), &games);

    super::page(ui, |ui| {
        super::header(ui, "Raftup", "Accueil", |_| {});

        let (heading, text, button, page) = step_texts(&step);
        widgets::panel(ui, "", |ui| {
            ui.label(widgets::caps("PROCHAIN PAS"));
            ui.label(widgets::title(&heading, 22.0));
            ui.label(widgets::dim(text));
            ui.add_space(8.0);
            if widgets::primary_button(ui, button, true) {
                goto = Some(page);
            }
        });
        ui.add_space(12.0);

        // Parties en cours.
        widgets::panel(ui, "Parties en cours", |ui| match &shared.status {
            Some(status) => {
                let players: Vec<_> = status.server.slots.iter().filter(|s| !s.is_group).collect();
                let checked: usize = players.iter().map(|s| s.checked).sum();
                let total: usize = players.iter().map(|s| s.total).sum();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("seed {}", status.seed.seed_name))
                            .strong()
                            .color(theme::TEXT),
                    );
                    ui.label(widgets::dim(format!(
                        "sur ce PC · démarrée il y a {}",
                        format::short_duration(format::now() - status.started_at)
                    )));
                });
                ui.horizontal(|ui| {
                    widgets::bar(ui, 200.0, matches::fraction(checked, total), theme::ACCENT);
                    ui.label(widgets::dim(matches::progress(checked, total)).size(12.0));
                });
                if widgets::ghost_button(ui, "Voir les parties") {
                    goto = Some(Page::InProgress);
                }
            }
            None => {
                ui.label(widgets::dim("Aucune partie en cours."));
            }
        });
        ui.add_space(12.0);

        // Invitation reçue.
        if let Some(game) = invite {
            widgets::panel(ui, "Invitation reçue", |ui| {
                let text = match game {
                    Some(game) => format!("Une partie de {game} t'attend."),
                    None => "Une partie t'attend.".to_string(),
                };
                ui.label(RichText::new(text).color(theme::TEXT));
                if widgets::ghost_button(ui, "Voir l'invitation") {
                    goto = Some(Page::Join);
                }
            });
            ui.add_space(12.0);
        }

        // Jeux prêts : leurs jaquettes, comme dans la Bibliothèque (ADR 0047).
        let ctx = ui.ctx().clone();
        widgets::panel(ui, "Jeux prêts", |ui| {
            let ready: Vec<&GameSummary> = games.iter().filter(|g| g.ready).collect();
            if ready.is_empty() {
                ui.label(widgets::dim(
                    "Aucun jeu prêt pour l'instant : la Bibliothèque montre ce qu'il manque.",
                ));
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(22.0, 16.0);
                for game in ready {
                    let cover = library.art_for(&ctx, &game.id, ArtKind::Cover);
                    let look = CardLook {
                        cover_id: &game.id,
                        name: &game.name,
                        status: game.label.clone(),
                        status_color: theme::TEXT_DIM,
                        fraction: 1.0,
                        marker: None,
                        cover,
                    };
                    if cover_card(ui, &look, false) {
                        library.select(&game.id);
                        goto = Some(Page::Library);
                    }
                }
            });
        });
        if let Some((text, url)) = library.art_attribution() {
            ui.add_space(6.0);
            super::library::attribution_line(ui, &text, &url);
        }
    });
    goto
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::library::Tone;

    fn game(name: &str, ready: bool) -> GameSummary {
        GameSummary {
            id: name.to_lowercase(),
            name: name.to_string(),
            ap_game: name.to_string(),
            authors: Vec::new(),
            label: String::new(),
            tone: Tone::Dim,
            ready,
            detected: ready,
            from_registry: false,
            trust: None,
        }
    }

    #[test]
    fn le_prochain_pas_suit_l_ordre_d_importance() {
        let games = [game("APQuest", false), game("Slime Rancher", true)];
        assert_eq!(next_step(true, true, &games), NextStep::OpenHosted);
        assert_eq!(next_step(false, true, &games), NextStep::FollowInvite);
        assert_eq!(
            next_step(false, false, &games),
            NextStep::CreateLobby("Slime Rancher".to_string())
        );
        assert_eq!(
            next_step(false, false, &[game("APQuest", false)]),
            NextStep::InstallGame
        );
        assert_eq!(next_step(false, false, &[]), NextStep::InstallGame);
    }
}
