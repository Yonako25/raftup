//! Plafonds du sondeur de règles dans le module d'export (lot 2 de l'inventaire de
//! conversion, `docs/inventaire-conversion.md`).

use py_bridge::{exporter_script, ProbeLimits, EXPORTER_SCRIPT};

#[test]
fn plafonds_par_defaut_identiques_au_script() {
    // Les valeurs par défaut de Rust sont celles écrites dans le script Python.
    let limits = ProbeLimits::default();
    assert!(EXPORTER_SCRIPT.contains(&format!("MAX_PATHS = {}  #", limits.max_paths)));
    assert!(EXPORTER_SCRIPT.contains(&format!("MAX_QUESTIONS = {}  #", limits.max_questions)));
}

#[test]
fn plafonds_redefinis_a_la_fin() {
    let text = exporter_script(ProbeLimits {
        max_paths: 50_000,
        max_questions: 128,
    });
    assert!(text.starts_with(EXPORTER_SCRIPT));
    let tail = &text[EXPORTER_SCRIPT.len()..];
    assert!(tail.contains("\nMAX_PATHS = 50000\n"));
    assert!(tail.contains("\nMAX_QUESTIONS = 128\n"));
}

#[test]
fn plafonds_bornes() {
    let text = exporter_script(ProbeLimits {
        max_paths: u32::MAX,
        max_questions: 0,
    });
    let tail = &text[EXPORTER_SCRIPT.len()..];
    assert!(tail.contains(&format!("MAX_PATHS = {}\n", ProbeLimits::MAX.max_paths)));
    assert!(tail.contains("MAX_QUESTIONS = 1\n"));
}
