//! Tests des variables de modèle.

use launcher_core::template::{check, expand, TemplateError, Vars};

#[test]
fn substitution_simple() {
    let vars = Vars::new()
        .with("game_dir", "C:\\Jeux\\Slime")
        .with("port", "38281");
    let out = expand("{game_dir}\\mod:{port}", &vars).unwrap();
    assert_eq!(out, "C:\\Jeux\\Slime\\mod:38281");
}

#[test]
fn double_accolade_donne_une_accolade() {
    let out = expand("{{json}} {{", &Vars::new()).unwrap();
    assert_eq!(out, "{json} {");
}

#[test]
fn accolade_litterale_a_cote_d_une_variable() {
    let vars = Vars::new().with("a", "1");
    assert_eq!(expand("{{{a}}}", &vars).unwrap(), "{1}");
}

#[test]
fn variable_inconnue_refusee() {
    let err = expand("{inconnue}", &Vars::new()).unwrap_err();
    assert!(matches!(err, TemplateError::UnknownVariable { name } if name == "inconnue"));
}

#[test]
fn accolade_non_fermee_refusee() {
    let vars = Vars::new().with("a", "1");
    assert!(matches!(
        expand("abc {a", &vars).unwrap_err(),
        TemplateError::Unclosed
    ));
}

#[test]
fn saut_de_ligne_et_nul_refuses() {
    for bad in ["a\nb", "a\rb", "a\0b"] {
        let vars = Vars::new().with("password", bad);
        let err = expand("pw={password}", &vars).unwrap_err();
        assert!(matches!(err, TemplateError::ForbiddenChar { name } if name == "password"));
    }
}

#[test]
fn valeur_non_utilisee_n_est_pas_verifiee() {
    let vars = Vars::new().with("x", "a\nb");
    assert_eq!(expand("rien", &vars).unwrap(), "rien");
}

#[test]
fn accents_conserves() {
    let vars = Vars::new().with("slot", "Hélène");
    assert_eq!(expand("é{slot}é", &vars).unwrap(), "éHélèneé");
}

#[test]
fn check_accepte_et_refuse() {
    assert!(check("{game_dir}/x {{ok}}", &["game_dir"]).is_ok());
    assert!(matches!(
        check("{password}", &["game_dir"]).unwrap_err(),
        TemplateError::UnknownVariable { name } if name == "password"
    ));
    assert!(matches!(
        check("{game_dir", &["game_dir"]).unwrap_err(),
        TemplateError::Unclosed
    ));
}
