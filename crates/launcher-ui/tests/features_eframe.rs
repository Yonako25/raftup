//! Garde-fou : eframe est compilé sans ses fonctions par défaut (`default-features = false`).
//! Il faut donc demander « clipboard » à la main, sinon `copy_text` ne fait rien et les
//! boutons « Copier le lien », « Copier l'adresse »… sont muets (bug du 18 novembre 2026).

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn eframe_a_le_presse_papiers_et_les_liens() {
    // `CARGO_MANIFEST_DIR` : dossier de cette crate ; le Cargo.toml du workspace est deux crans au-dessus.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let text = std::fs::read_to_string(root).unwrap();
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("eframe ="))
        .expect("ligne eframe du workspace");
    for feature in ["\"clipboard\"", "\"links\""] {
        assert!(line.contains(feature), "eframe sans {feature} : {line}");
    }
}
