//! Écran « Accueil » (plan de l'interface, étape 4) : ce qui se passe et quoi faire
//! ensuite, en un coup d'œil.
//!
//! - en haut, une **vitrine** : la grande image du jeu mis en avant ([`featured`]), avec
//!   par-dessus **le prochain pas**, une seule action choisie par [`next_step`]
//!   (fonctions pures, testées sans interface) ;
//! - « Ta bibliothèque » : les jeux de ce PC en une rangée qui défile ;
//! - les parties en cours (celle que ce PC héberge) ;
//! - l'invitation reçue et pas encore suivie.
//!
//! « Joués récemment » de la maquette attend un historique des parties, qui n'existe
//! pas encore (question notée dans la PR de l'étape 4).

use egui::{Color32, CornerRadius, RichText, Sense, Ui};

use super::join::JoinState;
use launcher_core::art::ArtKind;

use super::library::{cover_card, CardLook, GameSummary, LibraryState};
use super::matches;
use crate::app::Page;
use crate::art_ui;
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

/// Le jeu mis en avant dans la vitrine : le premier jeu prêt, sinon le premier jeu de
/// la Bibliothèque, sinon aucun. `in_library` dit si un jeu est sur ce PC.
// Le résultat emprunte un élément de `games` : Rust le déduit seul (une seule entrée
// empruntée), sans écrire de durée de vie.
pub fn featured(games: &[GameSummary], in_library: impl Fn(&str) -> bool) -> Option<&GameSummary> {
    games
        .iter()
        .find(|g| g.ready)
        .or_else(|| games.iter().find(|g| in_library(&g.id)))
}

/// Hauteur de la vitrine en haut de l'Accueil.
const SHOWCASE_HEIGHT: f32 = 280.0;

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

    let ctx = ui.ctx().clone();
    let hero =
        featured(&games, |id| library.in_library(id)).map(|g| (g.id.clone(), g.name.clone()));
    let hero_art = hero
        .as_ref()
        .and_then(|(id, _)| library.art_for(&ctx, id, ArtKind::Hero));

    super::page(ui, |ui| {
        // Vitrine : grande image du jeu mis en avant, le prochain pas par-dessus.
        let (heading, text, button, page) = step_texts(&step);
        if showcase(ui, hero.as_ref(), hero_art.as_ref(), &heading, text, button) {
            goto = Some(page);
        }
        ui.add_space(18.0);

        // Ta bibliothèque : les jeux de ce PC, en une rangée qui défile.
        let mine: Vec<&GameSummary> = games.iter().filter(|g| library.in_library(&g.id)).collect();
        ui.label(widgets::title("Ta bibliothèque", 20.0));
        ui.add_space(8.0);
        if mine.is_empty() {
            ui.label(widgets::dim(
                "Aucun jeu du catalogue trouvé sur ce PC pour l'instant : le Catalogue montre ceux que Raftup sait installer.",
            ));
        } else {
            // `id_salt` : la position du défilement est retenue d'une image à l'autre.
            egui::ScrollArea::horizontal()
                .id_salt("accueil_bibliotheque")
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(22.0, 16.0);
                        for game in mine {
                            let cover = library.art_for(&ctx, &game.id, ArtKind::Cover);
                            let look = CardLook {
                                cover_id: &game.id,
                                name: &game.name,
                                status: game.label.clone(),
                                status_color: theme::TEXT_DIM,
                                fraction: if game.ready { 1.0 } else { 0.0 },
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
        }
        ui.add_space(18.0);

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
    });
    goto
}

/// La vitrine : l'image large du jeu mis en avant (ou un aplat coloré), assombrie vers
/// le bas, avec le prochain pas écrit par-dessus. Renvoie vrai au clic sur le bouton.
fn showcase(
    ui: &mut Ui,
    hero: Option<&(String, String)>,
    texture: Option<&egui::TextureHandle>,
    heading: &str,
    text: &str,
    button: &str,
) -> bool {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, SHOWCASE_HEIGHT), Sense::hover());
    let radius = CornerRadius::same(theme::RADIUS);
    let painter = ui.painter_at(rect);
    match (texture, hero) {
        (Some(texture), Some((_, name))) => {
            art_ui::paint_cover(ui, texture, rect);
            let label = format!("Illustration de {name}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
        }
        (None, Some((id, _))) => {
            painter.rect_filled(rect, radius, theme::cover_color(id));
        }
        _ => {
            painter.rect_filled(rect, radius, theme::panel_fill());
        }
    }
    // Dégradé sombre vers le bas : le texte reste lisible sur n'importe quelle image.
    // Un maillage (`Mesh`) à quatre coins : transparent en haut, sombre en bas.
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_black_alpha(20);
    let bottom = Color32::from_black_alpha(215);
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, theme::BORDER),
        egui::StrokeKind::Inside,
    );

    // Le texte, en bas à gauche de l'image.
    let inner = rect.shrink2(egui::vec2(24.0, 22.0));
    let mut clicked = false;
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::bottom_up(egui::Align::Min)),
    );
    // `bottom_up` : on pose d'abord le bouton (en bas), puis les textes au-dessus.
    clicked |= widgets::primary_button(&mut content, button, true);
    content.add_space(10.0);
    content.label(RichText::new(text).color(theme::TEXT).size(15.0));
    content.label(widgets::title(heading, 30.0));
    content.label(widgets::caps("Prochain pas").color(theme::ACCENT));
    clicked
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
    fn le_jeu_mis_en_avant() {
        let games = [
            game("APQuest", false),
            game("Hollow Knight", false),
            game("Slime Rancher", true),
        ];
        // Le premier jeu prêt passe avant tout.
        let pick = featured(&games, |_| true).map(|g| g.name.as_str());
        assert_eq!(pick, Some("Slime Rancher"));
        // Aucun prêt : le premier jeu de la Bibliothèque.
        let none_ready = [game("APQuest", false), game("Hollow Knight", false)];
        let pick = featured(&none_ready, |id| id == "hollow knight").map(|g| g.name.as_str());
        assert_eq!(pick, Some("Hollow Knight"));
        // Rien sur ce PC : pas de jeu mis en avant.
        assert_eq!(featured(&none_ready, |_| false), None);
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
