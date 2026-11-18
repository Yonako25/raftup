//! Tests du plan de lancement (pur) et du lancement réel (programme de test du système).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use launcher_core::launch::{launch, plan_launch, plan_launch_offline, Credentials, LaunchError};
use launcher_core::profile::Profile;
use launcher_core::template::TemplateError;

const HEAD: &str = r#"
format_version = 1

[game]
id = "demo"
name = "Demo"
ap_game = "Demo Game"
status = "available"

[credits]
authors = ["Quelqu'un"]
license = "MIT"

[[steps]]
id = "le-mod"
kind = "mod"
name = "Le mod"
version = "1.0.0"
source = { url = "https://github.com/x/y/releases/download/v1/m.zip" }
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
install = { copy_to = "{game_dir}/Mods/m.dll" }
"#;

/// Profil valide dont la section `[launch]` est donnée par le test.
fn profile(launch_section: &str) -> Profile {
    Profile::parse(
        "demo.toml",
        &format!("{HEAD}\n[launch]\n{launch_section}\n"),
    )
    .unwrap()
}

fn creds(password: Option<&str>) -> Credentials {
    Credentials {
        address: "ws://serveur.fr:4242".to_string(),
        slot: "Alice".to_string(),
        password: password.map(str::to_string),
    }
}

const GAME: &str = "game";
const WORLDS: &str = "worlds";

fn plan(p: &Profile, c: &Credentials) -> Result<launcher_core::launch::LaunchPlan, LaunchError> {
    plan_launch(p, Path::new(GAME), Path::new(WORLDS), c)
}

#[test]
fn cli_args() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
args = ["-nolog"]
connect = { method = "cli_args", args = ["--host", "{host}", "--port", "{port}", "--slot", "{slot}", "--pw", "{password}"] }"#,
    );
    let plan = plan(&p, &creds(Some("secret"))).unwrap();
    assert_eq!(plan.exe, PathBuf::from("game/Demo.exe"));
    assert_eq!(
        plan.args,
        [
            "-nolog",
            "--host",
            "serveur.fr",
            "--port",
            "4242",
            "--slot",
            "Alice",
            "--pw",
            "secret"
        ]
    );
    assert!(plan.env.is_empty() && plan.config_edit.is_none() && plan.uri.is_none());
}

#[test]
fn port_par_defaut() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "cli_args", args = ["{host}:{port}"] }"#,
    );
    let c = Credentials {
        address: "archipelago.gg".into(),
        slot: "A".into(),
        password: None,
    };
    assert_eq!(plan(&p, &c).unwrap().args, ["archipelago.gg:38281"]);
}

#[test]
fn env() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "env", vars = { AP_ADDRESS = "{address}", AP_PASSWORD = "{password}" } }"#,
    );
    let plan = plan(&p, &creds(Some("secret"))).unwrap();
    assert_eq!(plan.env["AP_ADDRESS"], "ws://serveur.fr:4242");
    assert_eq!(plan.env["AP_PASSWORD"], "secret");
    assert!(plan.args.is_empty());
}

#[test]
fn config_file() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "config_file", path = "{game_dir}/UserData/ap.json", format = "json", set = { "server.address" = "{address}", slot = "{slot}" } }"#,
    );
    let plan = plan(&p, &creds(None)).unwrap();
    let edit = plan.config_edit.unwrap();
    assert_eq!(edit.path, PathBuf::from("game/UserData/ap.json"));
    assert_eq!(edit.values["server.address"], "ws://serveur.fr:4242");
    assert_eq!(edit.values["slot"], "Alice");
}

#[test]
fn uri() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "uri", scheme = "demo", template = "demo://connect?host={host}&slot={slot}" }"#,
    );
    let plan = plan(&p, &creds(None)).unwrap();
    assert_eq!(
        plan.uri.as_deref(),
        Some("demo://connect?host=serveur.fr&slot=Alice")
    );
    // Lancer par URI n'est pas encore pris en charge : erreur claire, rien n'est démarré.
    assert!(matches!(
        launch(&plan),
        Err(LaunchError::Unsupported("uri"))
    ));
}

#[test]
fn manual() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "manual", instructions = "Entre l'adresse dans le menu." }"#,
    );
    let plan = plan(&p, &creds(None)).unwrap();
    assert_eq!(
        plan.manual_instructions.as_deref(),
        Some("Entre l'adresse dans le menu.")
    );
    assert!(plan.args.is_empty() && plan.env.is_empty());
}

#[test]
fn valeur_interdite_refusee() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "cli_args", args = ["{slot}"] }"#,
    );
    let mut c = creds(None);
    c.slot = "Alice\n--evil".to_string();
    assert!(matches!(
        plan(&p, &c),
        Err(LaunchError::Template(TemplateError::ForbiddenChar { .. }))
    ));
}

#[test]
fn mot_de_passe_masque_dans_debug() {
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "cli_args", args = ["--pw", "{password}"] }"#,
    );
    let c = creds(Some("tres-secret-42"));
    let plan = plan(&p, &c).unwrap();
    assert!(!format!("{c:?}").contains("tres-secret-42"));
    assert!(!format!("{plan:?}").contains("tres-secret-42"));
    assert!(format!("{plan:?}").contains("***"));
    // L'argument réel, lui, contient bien le mot de passe : c'est ce qui est lancé.
    assert_eq!(plan.args[1], "tres-secret-42");
}

#[test]
fn le_plan_ne_touche_a_rien() {
    // Fonction pure : le dossier de jeu n'existe même pas, et rien n'est créé.
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "config_file", path = "{game_dir}/ap.json", format = "json", set = { a = "{slot}" } }"#,
    );
    plan(&p, &creds(None)).unwrap();
    assert!(!Path::new(GAME).exists());
}

#[test]
fn executable_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let p = profile(
        r#"exe = "{game_dir}/Absent.exe"
connect = { method = "manual", instructions = "x" }"#,
    );
    let plan = plan_launch(&p, tmp.path(), tmp.path(), &creds(None)).unwrap();
    assert!(matches!(launch(&plan), Err(LaunchError::Io { .. })));
}

/// Un vrai programme du système : lance, attend, vérifie le code de sortie.
#[test]
fn lancement_reel() {
    let tmp = tempfile::tempdir().unwrap();
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "manual", instructions = "x" }"#,
    );
    let mut plan = plan_launch(&p, tmp.path(), tmp.path(), &creds(None)).unwrap();
    // On remplace l'exécutable du profil par un programme qui existe partout.
    if cfg!(windows) {
        plan.exe = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32\\cmd.exe");
        plan.args = vec!["/C".into(), "exit".into(), "0".into()];
    } else {
        plan.exe = PathBuf::from("/bin/true");
        plan.args = vec![];
    }
    let mut child = launch(&plan).unwrap();
    assert!(child.wait().unwrap().success());
}

#[test]
fn config_json_fusionnee() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("UserData").join("ap.json");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, r#"{"keep": 1, "server": {"other": true}}"#).unwrap();

    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "config_file", path = "{game_dir}/UserData/ap.json", format = "json", set = { "server.address" = "{address}", slot = "{slot}" } }"#,
    );
    let mut plan = plan_launch(&p, tmp.path(), tmp.path(), &creds(None)).unwrap();
    // Programme du système à la place du jeu (le jeu de test n'existe pas).
    if cfg!(windows) {
        plan.exe = PathBuf::from(std::env::var("SystemRoot").unwrap()).join("System32\\cmd.exe");
        plan.args = vec!["/C".into(), "exit".into(), "0".into()];
    } else {
        plan.exe = PathBuf::from("/bin/true");
    }
    launch(&plan).unwrap().wait().unwrap();

    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
    assert_eq!(json["keep"], 1);
    assert_eq!(json["server"]["other"], true);
    assert_eq!(json["server"]["address"], "ws://serveur.fr:4242");
    assert_eq!(json["slot"], "Alice");
    assert!(!cfg.with_extension("json.tmp").exists());
}

#[test]
fn config_json_illisible_refusee() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("ap.json");
    fs::write(&cfg, "pas du json").unwrap();
    let p = profile(
        r#"exe = "{game_dir}/Demo.exe"
connect = { method = "config_file", path = "{game_dir}/ap.json", format = "json", set = { slot = "{slot}" } }"#,
    );
    let mut plan = plan_launch(&p, tmp.path(), tmp.path(), &creds(None)).unwrap();
    // L'exécutable doit exister pour arriver jusqu'à la config.
    plan.exe = cfg.clone();
    assert!(matches!(launch(&plan), Err(LaunchError::Config { .. })));
    // Le fichier d'origine n'est pas écrasé.
    assert_eq!(fs::read_to_string(&cfg).unwrap(), "pas du json");
}

#[test]
fn lancement_hors_partie_sans_connexion() {
    // Chaque méthode de connexion est ignorée : seuls l'exe et les arguments de base restent.
    let methods = [
        r#"connect = { method = "cli_args", args = ["--connect", "{address}", "--name", "{slot}"] }"#,
        r#"connect = { method = "env", vars = { AP_SLOT = "{slot}" } }"#,
        r#"connect = { method = "config_file", path = "{game_dir}/ap.json", format = "json", set = { a = "{slot}" } }"#,
        r#"connect = { method = "manual", instructions = "Adresse {address}" }"#,
    ];
    for method in methods {
        let p = profile(&format!(
            "exe = \"{{game_dir}}/Demo.exe\"\nargs = [\"-windowed\"]\n{method}"
        ));
        let plan = plan_launch_offline(&p, Path::new(GAME), Path::new(WORLDS)).unwrap();
        assert_eq!(plan.exe, Path::new(GAME).join("Demo.exe"), "{method}");
        assert_eq!(plan.args, vec!["-windowed".to_string()], "{method}");
        assert!(plan.env.is_empty(), "{method}");
        assert!(plan.config_edit.is_none(), "{method}");
        assert!(plan.uri.is_none(), "{method}");
        assert!(plan.manual_instructions.is_none(), "{method}");
    }
}
