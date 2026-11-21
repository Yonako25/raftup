//! Captures de référence (rendu wgpu) du Salon et de la Bibliothèque.
//!
//! Le test dessine chaque écran en image et le compare, pixel par pixel (avec une petite
//! tolérance), au PNG de `crates/launcher-ui/tests/snapshots/`. En cas d'écart, egui_kittest
//! écrit à côté `<nom>.new.png` (l'image du jour) et `<nom>.diff.png` (les différences).
//! Mettre à jour les références : `tests/README.md`.
//!
//! Un seul test pour les deux images : deux rendus wgpu en même temps (deux tests en
//! parallèle) font planter le pilote sous Windows (`STATUS_ACCESS_VIOLATION`, vu le
//! 21 novembre 2026). Ici, les écrans sont rendus l'un après l'autre.
//!
//! Seulement sous Windows : le rendu passe par la carte graphique (ou, à défaut, par le
//! moteur logiciel WARP de Windows), et les PNG de référence ont été faits sous Windows.
//! Sous Linux (CI), il faudrait un pilote Vulkan logiciel (lavapipe) et des références à
//! part ; sous macOS, Metal arrondit autrement. Les tests sans image, eux, tournent partout.

use egui_kittest::SnapshotResults;

use super::bibliotheque::bibliotheque;
use super::salon::salon;
use super::{click, type_into};

#[test]
#[cfg_attr(
    not(windows),
    ignore = "captures de référence faites sous Windows (rendu wgpu) : voir tests/README.md"
)]
fn captures_du_salon_et_de_la_bibliotheque() {
    // `snapshot` ne panique pas tout de suite : il range son résultat dans le harnais.
    // Avec deux harnais, egui_kittest demande de réunir les résultats en un seul lot,
    // vérifié à la fin (les deux images sont ainsi comparées, même si la première échoue).
    let mut results = SnapshotResults::new();
    // Les accolades limitent la vie de chaque harnais : le premier (et son rendu wgpu)
    // est détruit avant la création du second.
    {
        let mut harness = salon();
        type_into(
            &mut harness,
            "Ton nom dans la partie (16 caractères au plus)",
            "Alice",
        );
        type_into(&mut harness, "Pseudo", "Bob");
        click(&mut harness, "Ajouter");
        harness.snapshot("salon");
        results.extend_harness(&mut harness);
    }
    {
        let mut harness = bibliotheque();
        harness.snapshot("bibliotheque");
        results.extend_harness(&mut harness);
    }
    // Panique (le test échoue) si une image ne correspond pas à sa référence.
    results.unwrap();
}
