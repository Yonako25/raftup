//! Tests du fichier d'épinglage `generator-pins.toml` (ADR 0017).
//!
//! Le vrai fichier doit se charger ; les cas de refus partent du vrai fichier,
//! dans lequel on remplace une ligne pour fabriquer une version fausse.

use launcher_core::generation::{parse_pins, pins, GenerationError};

/// Le texte du vrai fichier, pour fabriquer des variantes fausses.
const REAL: &str = include_str!("../generator-pins.toml");

/// Remplace `from` par `to` (une seule fois) ; échoue si `from` n'existe pas, pour que
/// le test ne devienne pas « vrai » par accident si le fichier change.
fn tweak(from: &str, to: &str) -> String {
    assert!(REAL.contains(from), "motif absent du fichier : {from}");
    REAL.replacen(from, to, 1)
}

/// Le texte d'erreur d'un refus attendu.
fn refusal(text: &str) -> String {
    match parse_pins(text) {
        Err(GenerationError::Pins(message)) => message,
        other => panic!("refus attendu, obtenu : {other:?}"),
    }
}

#[test]
fn le_vrai_fichier_se_charge_et_se_valide() {
    let pins = pins().expect("le fichier embarqué doit être valide");
    assert_eq!(pins.format_version, 1);
    assert_eq!(pins.python.version, "3.12.16+20261110");
    assert_eq!(pins.python.strip_prefix, "python");
    assert_eq!(pins.archipelago.version, "0.6.8");
    assert!(pins
        .archipelago
        .url
        .contains("54803be064fc7e80c4628777ed0b46a9390f255f"));
    assert_eq!(pins.wheels.len(), 19);
    // Dépendances des mondes qui ne s'importaient pas sans elles.
    for nom in ["requests", "setuptools", "pyevermizer"] {
        assert!(
            pins.wheels.iter().any(|w| w.name == nom),
            "wheel {nom} absente"
        );
    }
    assert_eq!(pins.source_packages.len(), 1);
    assert_eq!(pins.source_packages[0].name, "zilliandomizer");
    assert_eq!(pins.source_packages[0].install_as, "zilliandomizer");
    for nom in ["colorama", "websockets", "PyYAML", "MarkupSafe", "bsdiff4"] {
        assert!(
            pins.wheels.iter().any(|w| w.name == nom),
            "wheel {nom} absente"
        );
    }
}

#[test]
fn refuse_http() {
    let text = tweak(
        "url = \"https://github.com/astral-sh",
        "url = \"http://github.com/astral-sh",
    );
    assert!(refusal(&text).contains("HTTPS"));
}

#[test]
fn refuse_un_hote_hors_liste() {
    let text = tweak(
        "https://files.pythonhosted.org/packages/d1/",
        "https://evil.example/packages/d1/",
    );
    assert!(refusal(&text).contains("liste blanche"));
}

#[test]
fn refuse_un_hote_deguise_par_des_identifiants() {
    let text = tweak(
        "https://github.com/ArchipelagoMW",
        "https://github.com@evil.example/ArchipelagoMW",
    );
    assert!(refusal(&text).contains("hôte invalide"));
}

#[test]
fn refuse_un_sha256_invalide() {
    // Un caractère non hexadécimal à la place du premier.
    let text = tweak("sha256 = \"e23fe5b1", "sha256 = \"g23fe5b1");
    assert!(refusal(&text).contains("hexadécimal"));
    // Trop court.
    let text = tweak(
        "sha256 = \"3b9c408cd43b09ab27691d5a0b8d18686cd113ebdd06fdd9606c229010648ef8\"",
        "sha256 = \"3b9c\"",
    );
    assert!(refusal(&text).contains("64"));
}

#[test]
fn refuse_une_wheel_en_double() {
    let doublon = "\n[[wheels]]\nname = \"colorama\"\nversion = \"0.4.6\"\n\
        filename = \"colorama-0.4.6-py2.py3-none-any.whl\"\n\
        url = \"https://files.pythonhosted.org/packages/d1/d6/3965ed04c63042e047cb6a3e6ed1a63a35087b6a609aa3a15ed8ac56c221/colorama-0.4.6-py2.py3-none-any.whl\"\n\
        sha256 = \"4f1d9991f5acc0ca119f9d443620b77f9d6b33703e51011c16baf57afb285fc6\"\n\
        size = 25335\nmax_bytes = 5242880\n";
    let text = format!("{REAL}{doublon}");
    assert!(refusal(&text).contains("en double"));
}

#[test]
fn refuse_un_champ_inconnu() {
    let text = tweak("max_entries = 6000", "max_entries = 6000\nsurprise = 1");
    assert!(refusal(&text).contains("surprise"));
}

#[test]
fn refuse_une_taille_nulle_ou_superieure_au_plafond() {
    // Archive : size = 0.
    let text = tweak("size = 47438699", "size = 0");
    assert!(refusal(&text).contains("size"));
    // Archive : size plus grande que max_bytes.
    let text = tweak("size = 23193480", "size = 99999999999");
    assert!(refusal(&text).contains("max_bytes"));
    // Wheel : size = 0 (la première occurrence de 25335 est celle de colorama).
    let text = tweak("size = 25335", "size = 0");
    assert!(refusal(&text).contains("colorama"));
    // Wheel : size au-dessus du plafond.
    let text = tweak("size = 25335", "size = 5242881");
    assert!(refusal(&text).contains("colorama"));
}

#[test]
fn les_tailles_du_vrai_fichier_sont_celles_relevees() {
    let pins = pins().expect("fichier valide");
    assert_eq!(pins.python.size, 47_438_699);
    assert_eq!(pins.archipelago.size, 23_193_480);
    assert!(pins.wheels.iter().all(|w| w.size > 0));
}

#[test]
fn refuse_format_version_2() {
    let text = tweak("format_version = 1", "format_version = 2");
    assert!(refusal(&text).contains("format_version"));
}

#[test]
fn refuse_un_filename_dangereux() {
    let text = tweak(
        "filename = \"colorama-0.4.6-py2.py3-none-any.whl\"",
        "filename = \"..-colorama-0.4.6-py2.py3-none-any.whl\"",
    );
    assert!(refusal(&text).contains("dangereux"));
    let text = tweak(
        "filename = \"colorama-0.4.6-py2.py3-none-any.whl\"",
        "filename = \"sub/colorama-0.4.6-py2.py3-none-any.whl\"",
    );
    assert!(refusal(&text).contains("dangereux"));
}

#[test]
fn refuse_une_wheel_pour_une_autre_plateforme() {
    // Même fichier, étiquette Linux : incompatible avec Windows.
    let text = tweak(
        "filename = \"orjson-3.11.7-cp312-cp312-win_amd64.whl\"",
        "filename = \"orjson-3.11.7-cp312-cp312-manylinux_2_17_x86_64.whl\"",
    );
    assert!(refusal(&text).contains("incompatible"));
    // Mauvaise version de Python.
    let text = tweak(
        "filename = \"orjson-3.11.7-cp312-cp312-win_amd64.whl\"",
        "filename = \"orjson-3.11.7-cp311-cp311-win_amd64.whl\"",
    );
    assert!(refusal(&text).contains("incompatible"));
}

#[test]
fn refuse_un_strip_prefix_qui_sort_de_l_archive() {
    let text = tweak("strip_prefix = \"python\"", "strip_prefix = \"../python\"");
    assert!(refusal(&text).contains("strip_prefix"));
}

#[test]
fn refuse_une_source_avec_un_nom_de_dossier_dangereux() {
    let text = tweak(
        "install_as = \"zilliandomizer\"",
        "install_as = \"../zilliandomizer\"",
    );
    assert!(refusal(&text).contains("install_as"));
}

#[test]
fn refuse_une_source_hors_liste_blanche() {
    let text = tweak(
        "https://github.com/beauxq/zilliandomizer/",
        "https://evil.example/beauxq/zilliandomizer/",
    );
    assert!(refusal(&text).contains("liste blanche"));
}
