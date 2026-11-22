//! Tests du profil : un profil valide se charge, chaque règle a son test de refus.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use launcher_core::profile::{
    ConnectMethod, InstallAction, Profile, ProfileError, Source, StepKind,
};

const VALID: &str = include_str!("data/profile/jeu-test.toml");
const FILE: &str = "jeu-test.toml";
const MANUAL: &str =
    "connect = { method = \"manual\", instructions = \"Entrez {address} et {slot} dans le menu.\" }";
const COPY: &str = "copy_to = \"{game_dir}/Mods/LeMod.dll\"";
const EXE: &str = "exe = \"{game_dir}/JeuTest.exe\"";

/// Remplace `from` par `to` dans le profil valide (et vérifie que `from` existe).
fn edit(from: &str, to: &str) -> String {
    assert!(
        VALID.contains(from),
        "motif absent du profil de test : {from}"
    );
    VALID.replacen(from, to, 1)
}

fn parse(text: &str) -> Result<Profile, ProfileError> {
    Profile::parse(FILE, text)
}

#[track_caller]
fn assert_invalid(text: &str, fragment: &str) {
    match parse(text) {
        Err(ProfileError::Invalid { reason, .. }) => {
            assert!(reason.contains(fragment), "raison inattendue : {reason}")
        }
        other => panic!("attendu Invalid({fragment}), obtenu {other:?}"),
    }
}

#[track_caller]
fn assert_toml_error(text: &str) {
    let err = parse(text).unwrap_err();
    assert!(matches!(err, ProfileError::Toml { .. }), "{err:?}");
}

#[track_caller]
fn assert_template_error(text: &str) {
    let err = parse(text).unwrap_err();
    assert!(matches!(err, ProfileError::Template { .. }), "{err:?}");
}

#[test]
fn profil_valide_complet() {
    let p = parse(VALID).unwrap();
    assert_eq!(p.game.id, "jeu-test");
    assert_eq!(p.game.detect.steam_appid, Some(433340));
    assert_eq!(p.steps.len(), 2);
    assert_eq!(p.steps[0].kind, StepKind::ModLoader);
    assert_eq!(p.steps[0].max_bytes_or_default(), 50_000_000);
    assert!(matches!(
        &p.steps[0].install,
        InstallAction::Extract { into, strip_prefix, inner: None } if into == "{game_dir}" && strip_prefix.is_empty()
    ));
    assert!(matches!(&p.steps[1].install, InstallAction::CopyTo(c) if c.ends_with("LeMod.dll")));
    assert_eq!(p.preserve.paths.len(), 1);
    assert_eq!(p.cleanup.paths.len(), 1);
    assert!(matches!(p.launch.connect, ConnectMethod::Manual { .. }));
}

#[test]
fn preserve_et_cleanup_facultatifs() {
    let sans = VALID
        .replace(
            "[preserve]\npaths = [\"{game_dir}/UserData/LeMod.cfg\"]\n",
            "",
        )
        .replace(
            "[cleanup]\npaths = [\"{game_dir}/UserData/cache.bin\"]\n",
            "",
        );
    let p = parse(&sans).unwrap();
    assert!(p.preserve.paths.is_empty() && p.cleanup.paths.is_empty());
}

#[test]
fn thunderstore_donne_la_bonne_url() {
    let p = parse(VALID).unwrap();
    assert_eq!(
        p.steps[1].source.resolve(&p.steps[1].version),
        "https://thunderstore.io/package/download/Auteur/Le_Mod/1.2.0/"
    );
    assert!(matches!(p.steps[0].source, Source::Url(_)));
}

#[test]
fn autres_methodes_de_connexion() {
    for connect in [
        "connect = { method = \"cli_args\", args = [\"--server={address}\", \"--slot={slot}\"] }",
        "connect = { method = \"env\", vars = { AP_PASSWORD = \"{password}\" } }",
        "connect = { method = \"uri\", scheme = \"jeutest\", template = \"jeutest://{host}:{port}/{slot}\" }",
        "connect = { method = \"config_file\", path = \"{game_dir}/UserData/ap.json\", format = \"json\", set = { server = \"{address}\" } }",
    ] {
        parse(&edit(MANUAL, connect)).unwrap_or_else(|e| panic!("{connect} : {e}"));
    }
}

#[test]
fn refuse_format_version_future() {
    let futur = edit("format_version = 1", "format_version = 2");
    let err = parse(&futur).unwrap_err();
    assert!(matches!(
        err,
        ProfileError::UnsupportedVersion {
            found: 2,
            max: 1,
            ..
        }
    ));
    // Même avec un champ inconnu d'un futur format : on répond « mettez à jour ».
    let futur = format!("{futur}\nchamp_futur = 1\n");
    assert!(matches!(
        parse(&futur),
        Err(ProfileError::UnsupportedVersion { .. })
    ));
}

#[test]
fn temps_par_lieu_facultatif_et_borne() {
    // Absent : le profil reste valide (format_version inchangé, ADR 0030).
    assert_eq!(parse(VALID).unwrap().game.seconds_per_location, None);
    let with = |value: &str| {
        edit(
            "status = \"available\"",
            &format!("status = \"available\"\nseconds_per_location = {value}"),
        )
    };
    assert_eq!(
        parse(&with("90")).unwrap().game.seconds_per_location,
        Some(90)
    );
    assert_invalid(&with("0"), "seconds_per_location");
    assert_invalid(&with("3601"), "seconds_per_location");
}

#[test]
fn refuse_champ_inconnu() {
    assert_toml_error(&format!("{VALID}\n[extra]\nx = 1\n"));
    assert_toml_error(&edit("name = \"Le mod\"", "name = \"Le mod\"\ncolour = 3"));
}

#[test]
fn refuse_sha256_mauvais_ou_absent() {
    assert_toml_error(&edit(&"b".repeat(64), "pas-un-hash"));
    assert_toml_error(&edit(&format!("sha256 = \"{}\"\n", "b".repeat(64)), ""));
}

#[test]
fn refuse_rom_et_asset() {
    for kind in ["rom", "asset"] {
        assert_toml_error(&edit("kind = \"mod\"", &format!("kind = \"{kind}\"")));
    }
}

#[test]
fn refuse_source_http() {
    assert_invalid(
        &edit("https://github.com/LavaGang", "http://github.com/LavaGang"),
        "HTTPS",
    );
}

#[test]
fn refuse_domaine_hors_liste_blanche() {
    assert_invalid(
        &edit(
            "https://github.com/LavaGang",
            "https://evil.example.com/LavaGang",
        ),
        "liste blanche",
    );
    // Ruses sur l'hôte : identifiants, port, sous-domaine collé.
    let original =
        "https://github.com/LavaGang/MelonLoader/releases/download/v0.7.3/MelonLoader.x64.zip";
    for url in [
        "https://github.com@evil.com/x.zip",
        "https://github.com:8443/x.zip",
        "https://github.com.evil.com/x.zip",
        "https://notgithub.com/x.zip",
    ] {
        assert_invalid(&edit(original, url), "source");
    }
}

#[test]
fn refuse_thunderstore_mal_forme() {
    assert_invalid(&edit("Auteur-Le_Mod", "AuteurSansTiret"), "thunderstore");
    assert_invalid(&edit("Auteur-Le_Mod", "Auteur-../x"), "thunderstore");
}

#[test]
fn refuse_cibles_hors_des_racines() {
    for bad in [
        "C:/Windows/x.dll",
        "{game_dir}/../x.dll",
        "{game_dir}x/y.dll",
        "/etc/x",
        "{game_dir}/C:/x.dll",
        "{game_dir}/{worlds_dir}/x.dll",
        "{game_dir}",
    ] {
        assert_invalid(&edit(COPY, &format!("copy_to = \"{bad}\"")), "chemin");
    }
    assert_invalid(
        &edit("into = \"{game_dir}\"", "into = \"{game_dir}/../..\""),
        "chemin",
    );
    assert_invalid(
        &edit("into = \"{game_dir}\"", "into = \"D:/autre\""),
        "chemin",
    );
    assert_invalid(
        &edit("strip_prefix = \"\"", "strip_prefix = \"../x\""),
        "strip_prefix",
    );
    assert_invalid(
        &edit("{game_dir}/UserData/LeMod.cfg", "{game_dir}/../LeMod.cfg"),
        "chemin",
    );
    // worlds_dir est accepté comme racine d'étape.
    parse(&edit(COPY, "copy_to = \"{worlds_dir}/x.apworld\"")).unwrap();
}

#[test]
fn refuse_exe_hors_du_dossier_du_jeu() {
    for bad in [
        "C:/Windows/cmd.exe",
        "{worlds_dir}/a.exe",
        "{game_dir}/../a.exe",
    ] {
        assert_invalid(&edit(EXE, &format!("exe = \"{bad}\"")), "chemin");
    }
}

#[test]
fn refuse_id_d_etape_en_double() {
    assert_invalid(
        &edit("id = \"le-mod\"", "id = \"melonloader\""),
        "en double",
    );
}

#[test]
fn refuse_nom_de_fichier_different_de_l_id() {
    let err = Profile::parse("autre.toml", VALID).unwrap_err();
    assert!(matches!(err, ProfileError::Invalid { .. }), "{err:?}");
    assert_invalid(&edit("id = \"jeu-test\"", "id = \"Jeu_Test\""), "game.id");
}

#[test]
fn refuse_licence_permission_sans_url() {
    let url_line = "permission_url = \"https://exemple.test/autorisation\"\n";
    assert_invalid(&edit(url_line, ""), "permission_url");
    // Une licence SPDX n'a pas besoin d'URL.
    let spdx = edit("license = \"permission\"", "license = \"MIT\"").replace(url_line, "");
    parse(&spdx).unwrap();
}

#[test]
fn refuse_variable_inconnue() {
    assert_template_error(&edit(
        "{game_dir}/Mods/LeMod.dll",
        "{inconnue}/Mods/LeMod.dll",
    ));
    // `{address}` est permis au lancement mais pas dans une étape.
    assert_template_error(&edit("{game_dir}/Mods/LeMod.dll", "{address}/x.dll"));
    assert_template_error(&edit("--melonloader.hideconsole", "{nope}"));
    // Les instructions manuelles sont du texte libre.
    parse(&edit("{slot} dans le menu", "{nope} dans le menu")).unwrap();
}

#[test]
fn refuse_config_file_hors_json() {
    let toml = edit(
        MANUAL,
        "connect = { method = \"config_file\", path = \"{game_dir}/c.yaml\", format = \"yaml\" }",
    );
    assert_invalid(&toml, "json");
}

#[test]
fn refuse_methode_de_connexion_inconnue() {
    assert_toml_error(&edit(MANUAL, "connect = { method = \"magie\" }"));
}

#[test]
fn refuse_install_ambigu() {
    assert_toml_error(&edit(
        "install = { copy_to = \"{game_dir}/Mods/LeMod.dll\" }",
        "install = { copy_to = \"{game_dir}/x\", into = \"{game_dir}\" }",
    ));
    assert_toml_error(&edit("extract = \"zip\"", "extract = \"rar\""));
}

/// `inner` (ADR 0050) : chemin d'une archive dans l'archive ; un chemin dangereux est refusé.
#[test]
fn archive_interieure_et_chemin_refuse() {
    let with_inner = VALID.replacen(
        "install = { extract = \"zip\", into = \"{game_dir}\", strip_prefix = \"\" }",
        "install = { extract = \"zip\", inner = \"x/install.dat\", into = \"{game_dir}\", strip_prefix = \"\" }",
        1,
    );
    assert_ne!(
        with_inner, VALID,
        "le profil de test a changé : adapter ce test"
    );
    let p = parse(&with_inner).unwrap();
    assert!(matches!(
        &p.steps[0].install,
        InstallAction::Extract { inner: Some(i), .. } if i == "x/install.dat"
    ));
    for bad in ["../install.dat", "/abs.dat", "C:/x.dat", ""] {
        let text = with_inner.replace("x/install.dat", bad);
        assert!(
            parse(&text).is_err(),
            "inner « {bad} » aurait dû être refusé"
        );
    }
}
