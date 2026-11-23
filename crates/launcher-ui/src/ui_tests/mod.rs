//! Tests de l'interface du launcher, sans ouvrir de fenêtre (egui_kittest).
//!
//! egui_kittest dessine un écran dans un `egui::Context` en mémoire, puis lit l'arbre
//! d'accessibilité (AccessKit) qu'egui produit pour les lecteurs d'écran : on y cherche
//! un bouton ou un texte par son libellé, on clique, on tape, comme un joueur.
//! Aucune fenêtre, aucune carte graphique (sauf pour les captures de `captures.rs`).
//!
//! Pourquoi ici et pas dans `tests/` : `launcher-ui` est un binaire seul, sans `lib.rs`.
//! Un test de `tests/` ne verrait pas ses modules ; un module `#[cfg(test)]` les voit.
//! Mode d'emploi : `crates/launcher-ui/tests/README.md`.

// Dans un test, un `unwrap` qui panique est exactement ce qu'on veut : le test échoue.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod bibliotheque;
mod captures;
mod captures_interface;
mod ecrans;
mod menu;
mod parcours;
mod partie;
mod rafthub;
mod salon;

use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, HarnessBuilder, Node};
use launcher_core::Catalog;

/// Taille de la zone centrale du launcher dans sa fenêtre de départ (1280 × 820) :
/// moins la barre latérale (220 px) et la barre de titre. Assez large pour que les
/// écrans se mettent sur deux colonnes (`screens::WIDE`).
pub const CENTRAL: egui::Vec2 = egui::vec2(1060.0, 790.0);

/// Un harnais réglé comme le launcher : thème sombre, polices et couleurs de Raftup.
///
/// `State` est le type de l'état que le test garde d'une image à l'autre (le
/// formulaire d'un écran, par exemple) ; le harnais le passe à chaque dessin.
pub fn builder<State>() -> HarnessBuilder<State> {
    Harness::builder()
        .with_size(CENTRAL)
        .with_theme(egui::Theme::Dark)
        // egui_kittest refuse par défaut un champ sans nom pour un lecteur d'écran. Le
        // launcher en a encore : les listes de jeux du salon, les mots de passe (salon,
        // « Amis », Bibliothèque) et le slot de la Bibliothèque. Contrôle coupé tant
        // qu'ils ne sont pas nommés (à faire à part : ce n'est pas le but de ces tests).
        .with_accessibility_check(false)
        // Avant la première image : les polices doivent être là dès le premier dessin.
        .with_context_setup(crate::theme::install)
}

/// Le catalogue des tests : Slime Rancher seul, lu dans `profiles/`. Il reste fixe
/// quand le catalogue grandit : les textes attendus et les captures de référence
/// (Windows) ne changent pas à chaque nouveau profil.
pub fn demo_catalog() -> Catalog {
    let text = include_str!("../../../../profiles/slime-rancher.toml");
    Catalog::from_texts(&[("slime-rancher.toml", text)]).unwrap()
}

/// Un champ de texte, reconnu par son texte d'aide (le texte gris du champ vide).
///
/// Les durées de vie `'h` : le nœud rendu emprunte le harnais, il ne peut pas lui
/// survivre. `move` : la fermeture garde sa propre copie de la référence `hint`.
pub fn field<'h, State>(harness: &'h Harness<'_, State>, hint: &'h str) -> Node<'h> {
    harness.get_by(move |node| node.data().placeholder() == Some(hint))
}

/// Tape `text` dans le champ dont le texte d'aide est `hint`, comme au clavier.
///
/// Deux temps : le champ prend d'abord le focus (une image), puis reçoit le texte.
/// Le nœud est recherché à chaque fois : `run` modifie le harnais (`&mut`), on ne peut
/// pas garder un nœud qui l'emprunte pendant ce temps.
pub fn type_into<State>(harness: &mut Harness<'_, State>, hint: &str, text: &str) {
    field(harness, hint).focus();
    harness.run();
    field(harness, hint).type_text(text);
    harness.run();
}

/// Clique sur le seul nœud qui porte ce libellé, puis laisse l'écran se redessiner.
pub fn click<State>(harness: &mut Harness<'_, State>, label: &str) {
    // Comme un joueur : on fait défiler jusqu'au bouton s'il est sous le bas de
    // l'écran, puis on clique (un clic hors de la fenêtre ne touche rien).
    harness.get_by_label(label).scroll_to_me();
    harness.run();
    harness.get_by_label(label).click();
    harness.run();
}

/// Nombre de nœuds de l'écran qui portent exactement ce libellé (un titre de panneau
/// et un bouton peuvent avoir le même : « Se connecter »).
pub fn count<State>(harness: &Harness<'_, State>, label: &str) -> usize {
    harness.query_all_by_label(label).count()
}

/// Vrai si au moins un nœud de l'écran porte exactement ce libellé.
pub fn shows<State>(harness: &Harness<'_, State>, label: &str) -> bool {
    count(harness, label) > 0
}

/// Vrai si au moins un nœud de l'écran contient ce morceau de texte.
pub fn shows_part<State>(harness: &Harness<'_, State>, part: &str) -> bool {
    harness.query_all_by_label_contains(part).next().is_some()
}
