//! Entrées invalides : chaque erreur est typée, porte une position quand c'est possible,
//! et rien ne panique, même sur un texte abîmé au hasard.

#![allow(clippy::unwrap_used)]

use logic::MultiWorld;
use rand_chacha::ChaCha20Rng;
use rand_core::{Rng, SeedableRng};
use worlds_native::{parse_world, BuildError, LoadError, Position, MAX_FILE_BYTES};

const MINIMAL: &str = include_str!("../../../spec/monde-natif/exemples/minimal/world.toml");
const APQUEST: &str = include_str!("../../../spec/monde-natif/exemples/apquest/world.toml");

fn err(text: &str) -> LoadError {
    parse_world(text).unwrap_err()
}

/// Remplace `from` par `to` dans l'exemple minimal (une seule fois, et `from` doit y être).
fn minimal_with(from: &str, to: &str) -> String {
    assert!(MINIMAL.contains(from), "« {from} » absent de l'exemple");
    MINIMAL.replacen(from, to, 1)
}

/// Position (ligne, colonne) de la première occurrence de `needle` dans `text`.
fn position_of(text: &str, needle: &str) -> Position {
    let offset = text.find(needle).unwrap();
    Position::of(text, offset)
}

/// L'erreur est « invalide », à cette position, et son message contient `part`.
fn assert_invalid_at(text: &str, needle: &str, part: &str) {
    let e = err(text);
    assert!(matches!(e, LoadError::Invalid { .. }), "{e:?}");
    assert_eq!(e.position(), Some(position_of(text, needle)), "{e}");
    assert!(e.to_string().contains(part), "« {part} » absent de : {e}");
}

fn assert_invalid(text: &str, part: &str) {
    let e = err(text);
    assert!(matches!(e, LoadError::Invalid { .. }), "{e:?}");
    assert!(e.to_string().contains(part), "« {part} » absent de : {e}");
}

// ---- Fichier ----

#[test]
fn toml_illisible_avec_position() {
    let text = minimal_with("id = 2\nclassification", "id = = 2\nclassification");
    let e = err(&text);
    assert!(matches!(e, LoadError::Toml { pos: Some(_), .. }), "{e:?}");
    assert_eq!(
        e.position().unwrap().line,
        position_of(&text, "id = = 2").line
    );
}

#[test]
fn version_du_format() {
    assert_eq!(
        err(&minimal_with("format_version = 1", "format_version = 4")),
        LoadError::Format { found: 4 }
    );
    assert_invalid(
        &minimal_with("format_version = 1", ""),
        "format_version manquant",
    );
    assert!(matches!(
        err(&minimal_with(
            "format_version = 1",
            "format_version = \"1\""
        )),
        LoadError::Toml { .. }
    ));
}

#[test]
fn champ_inconnu_refuse() {
    let e = err(&minimal_with(
        "id = 1\nregion = \"Start\"",
        "id = 1\nregoin = \"Start\"",
    ));
    assert!(matches!(e, LoadError::Toml { .. }));
    assert!(e.to_string().contains("regoin"), "{e}");
}

#[test]
fn fichier_trop_gros() {
    let text = format!("{MINIMAL}\n#{}", "x".repeat(MAX_FILE_BYTES));
    assert!(matches!(err(&text), LoadError::TooLarge { .. }));
}

#[test]
fn trop_d_elements() {
    let mut text = String::from(MINIMAL);
    for i in 0..=worlds_native::MAX_ELEMENTS {
        text.push_str(&format!("[[regions]]\nname = \"R{i}\"\n"));
        if text.len() > MAX_FILE_BYTES {
            // Le plafond de taille arrive avant celui des éléments : les deux protègent.
            assert!(matches!(err(&text), LoadError::TooLarge { .. }));
            return;
        }
    }
    assert_invalid(&text, "trop de régions");
}

// ---- Noms et numéros ----

#[test]
fn noms_en_double() {
    assert_invalid_at(
        &minimal_with("name = \"Coin\"", "name = \"Key\""),
        "\"Key\"\nid = 2",
        "objet « Key » en double",
    );
    assert_invalid(
        &minimal_with("name = \"Vault\"", "name = \"Start\""),
        "région « Start » en double",
    );
    assert_invalid(
        &minimal_with("name = \"Treasure\"", "name = \"Chest\""),
        "lieu « Chest » en double",
    );
    let event =
        format!("{MINIMAL}\n[[events]]\nname = \"Chest\"\nregion = \"Start\"\nitem = \"Done\"\n");
    assert_invalid(&event, "un événement est aussi un lieu");
    let event =
        format!("{MINIMAL}\n[[events]]\nname = \"Win\"\nregion = \"Start\"\nitem = \"Coin\"\n");
    assert_invalid(&event, "existe déjà");
}

#[test]
fn numeros_invalides() {
    assert_invalid(
        &minimal_with("id = 2\nclassification", "id = 1\nclassification"),
        "numéro 1 déjà pris",
    );
    assert_invalid(
        &minimal_with("id = 2\nclassification", "id = 0\nclassification"),
        "doit être positif",
    );
    assert_invalid(
        &minimal_with("id = 2\nregion", "id = 1\nregion"),
        "numéro 1 déjà pris",
    );
    assert_invalid(
        &minimal_with("id = 2\nregion", "id = -4\nregion"),
        "doit être positif",
    );
}

#[test]
fn noms_vides_ou_trop_longs() {
    assert_invalid(&minimal_with("name = \"Coin\"", "name = \"\""), "nom vide");
    let long = "x".repeat(worlds_native::MAX_NAME_CHARS + 1);
    assert_invalid(
        &minimal_with("name = \"Coin\"", &format!("name = \"{long}\"")),
        "caractères",
    );
    assert_invalid(
        &minimal_with("name = \"Coin\"", "name = \"Co\\u0007in\""),
        "contrôle",
    );
}

// ---- Références ----

#[test]
fn references_inconnues() {
    assert_invalid(
        &minimal_with("to = \"Vault\"", "to = \"Vaul\""),
        "région « Vaul » inconnue",
    );
    assert_invalid(
        &minimal_with("region = \"Vault\"", "region = \"Cellar\""),
        "région « Cellar » inconnue",
    );
    assert_invalid(
        &minimal_with("[[pool]]\nitem = \"Key\"", "[[pool]]\nitem = \"Sword\""),
        "objet « Sword » inconnu",
    );
    assert_invalid(
        &minimal_with("[filler]\nitem = \"Coin\"", "[filler]\nitem = \"Gold\""),
        "objet « Gold » inconnu",
    );
    let start = minimal_with(
        "[game]\nname = \"Minimal\"",
        "[game]\nname = \"Minimal\"\nstart_region = \"Nowhere\"",
    );
    assert_invalid(&start, "start_region");
    let group = format!("{MINIMAL}\n[[item_groups]]\nname = \"G\"\nitems = [\"Key\", \"Ghost\"]\n");
    assert_invalid(&group, "objet « Ghost » inconnu");
    let group = format!("{MINIMAL}\n[[location_groups]]\nname = \"G\"\nlocations = [\"Nope\"]\n");
    assert_invalid(&group, "lieu « Nope » inconnu");
    let slot = format!("{MINIMAL}\n[[slot_data]]\nkey = \"k\"\noption = \"nope\"\n");
    assert_invalid(&slot, "option « nope » inconnue");
    let slot = format!("{MINIMAL}\n[[slot_data]]\nkey = \"k\"\n");
    assert_invalid(&slot, "un seul des deux");
}

#[test]
fn objet_d_evenement_hors_du_pool() {
    let text = format!(
        "{MINIMAL}\n[[events]]\nname = \"Win\"\nregion = \"Start\"\nitem = \"Done\"\n[[start]]\nitem = \"Done\"\n"
    );
    assert_invalid(&text, "objet d'événement");
}

#[test]
fn region_inaccessible() {
    let text = format!("{MINIMAL}\n[[regions]]\nname = \"Island\"\n");
    assert_invalid_at(
        &text,
        "\"Island\"",
        "région « Island » inaccessible depuis « Start »",
    );
}

#[test]
fn aucune_region() {
    let text = "format_version = 1\n[game]\nname = \"X\"\ngoal = \"true\"\n";
    assert_invalid(text, "aucune région");
}

// ---- Règles ----

#[test]
fn regle_objet_inconnu_ligne_et_colonne() {
    let text = minimal_with("rule = \"has(Key)\"", "rule = \"has(Kye)\"");
    assert_invalid_at(&text, "Kye", "objet « Kye » inconnu");
    // Chaîne littérale entre apostrophes : même chose.
    let text = minimal_with(
        "rule = \"has(Key)\"",
        "rule = 'has(Key) and has(\"Kye 2\")'",
    );
    assert_invalid_at(&text, "\"Kye 2\"", "objet « Kye 2 » inconnu");
    // Chaîne sur plusieurs lignes : la ligne suivante compte.
    let text = minimal_with(
        "rule = \"has(Key)\"",
        "rule = '''\nhas(Key) and\n  has(Kye)'''",
    );
    assert_invalid_at(&text, "Kye)", "objet « Kye » inconnu");
}

#[test]
fn erreurs_de_syntaxe() {
    let cases = [
        ("has(Key", "attendu"),
        ("has(Key))", "texte en trop"),
        ("has(\"Key)", "guillemet ouvert jamais fermé"),
        ("has(Key) && has(Coin)", "caractère « & » inattendu"),
        ("has(Key) = 1", "« = » seul"),
        ("has(Key) or", "règle incomplète"),
        ("hass(Key)", "fonction « hass » inconnue"),
        ("has()", "has attend"),
        ("has(Key, 1, 2)", "has attend"),
        ("has(Key, Coin)", "nombre attendu"),
        ("has(Key, -1)", "hors bornes"),
        ("has_count(Key)", "has_count attend 2"),
        ("has_all()", "au moins un objet"),
        ("has_group(Nope)", "groupe d'objets « Nope » inconnu"),
        ("can_reach(Nowhere)", "région ou lieu « Nowhere » inconnu"),
        ("can_reach_region(Chest)", "région « Chest » inconnue"),
        ("can_reach_location(Start)", "lieu « Start » inconnu"),
        ("can_reach_entrance(Door)", "entrée « Door » inconnue"),
        ("has(\"a\\nb\")", "échappement inconnu"),
        ("has(Key) == 1", "seule une option se compare"),
        ("(has(Key)", "« ) » attendu"),
        ("and", "fonction, « ( », true ou false attendu"),
        ("has(and)", "argument attendu"),
        ("option(nope)", "option « nope » inconnue"),
        ("99999999999999999999", "invalide"),
    ];
    for (rule, part) in cases {
        let text = minimal_with("rule = \"has(Key)\"", &format!("rule = '{rule}'"));
        assert_invalid(&text, part);
    }
}

#[test]
fn regles_mal_typees() {
    let options = "\n[[options]]\nname = \"hard\"\ntype = \"toggle\"\n\
                   [[options]]\nname = \"n\"\ntype = \"range\"\nmin = 0\nmax = 9\n\
                   [[options]]\nname = \"mode\"\ntype = \"choice\"\nchoices = [\"a\", \"b\"]\n";
    let with = |rule: &str| {
        format!(
            "{}{options}",
            minimal_with("rule = \"has(Key)\"", &format!("rule = '{rule}'"))
        )
    };
    assert_invalid(
        &with("not has(Key)"),
        "« not » ne s'applique qu'à une condition sur les options",
    );
    assert_invalid(
        &with("option(n)"),
        "une option numérique ou à choix se compare",
    );
    assert_invalid(&with("option(mode) == c"), "choix « c » inconnu");
    assert_invalid(&with("option(mode) == 5"), "choix n° 5 inexistant");
    assert_invalid(
        &with("has(Key, option(hard))"),
        "seule une option numérique",
    );
    assert_invalid(&with("option(n) == fast"), "valeur attendue");
    // Corrects : not sur une option, comparaisons, nombre tiré d'une option.
    for rule in [
        "has(Key) and not option(hard)",
        "option(n) >= 3 or has(Key)",
        "option(mode) == b and option(hard) == true",
        "has(Key, option(n))",
        "not (option(hard) and option(mode) != a) or has(Key)",
    ] {
        parse_world(&with(rule)).unwrap();
    }
    // Un `when` ne dépend que des options.
    let when = format!(
        "{}{options}",
        minimal_with(
            "region = \"Vault\"",
            "region = \"Vault\"\nwhen = \"has(Key)\""
        )
    );
    assert_invalid(&when, "une condition « when » ne dépend que des options");
}

#[test]
fn can_reach_ambigu() {
    let text = format!(
        "{}\n[[regions]]\nname = \"Chest\"\n[[entrances]]\nname = \"to chest\"\nfrom = \"Start\"\nto = \"Chest\"\n",
        minimal_with("rule = \"has(Key)\"", "rule = 'can_reach(Chest)'")
    );
    assert_invalid(&text, "est une région et un lieu");
}

#[test]
fn regle_trop_imbriquee_ou_trop_longue() {
    let deep = format!("{}has(Key){}", "(".repeat(200), ")".repeat(200));
    let text = minimal_with("rule = \"has(Key)\"", &format!("rule = '{deep}'"));
    assert_invalid(&text, "trop imbriquée");
    let nots = format!("{}true", "not ".repeat(200));
    let text = minimal_with("rule = \"has(Key)\"", &format!("rule = '{nots}'"));
    assert_invalid(&text, "trop imbriquée");
    let long = vec!["has(Key)"; 3000].join(" or ");
    let text = minimal_with("rule = \"has(Key)\"", &format!("rule = '{long}'"));
    assert_invalid(&text, "octets");
}

// ---- Options ----

#[test]
fn options_mal_definies() {
    let with = |option: &str| format!("{MINIMAL}\n[[options]]\n{option}\n");
    assert_invalid(
        &with("name = \"x\"\ntype = \"range\"\nmin = 5\nmax = 1"),
        "plus grand que max",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"range\"\nmin = 0\nmax = 1\ndefault = 7"),
        "hors de",
    );
    assert_invalid(&with("name = \"x\"\ntype = \"range\""), "obligatoires");
    assert_invalid(&with("name = \"x\"\ntype = \"choice\""), "vide");
    assert_invalid(
        &with("name = \"x\"\ntype = \"choice\"\nchoices = [\"a\", \"A\"]"),
        "en double",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"choice\"\nchoices = [\"a\"]\naliases = { b = \"z\" }"),
        "choix inconnu",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"choice\"\nchoices = [\"a\"]\naliases = { a = \"a\" }"),
        "déjà pris",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"choice\"\nchoices = [\"a\"]\ndefault = \"b\""),
        "n'est pas un choix",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"toggle\"\nmin = 1"),
        "ne va pas avec",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"toggle\"\ndefault = 3"),
        "true ou false",
    );
    assert_invalid(
        &with("name = \"x\"\ntype = \"slider\""),
        "type « slider » inconnu",
    );
    assert_invalid(
        &with("name = \"x y\"\ntype = \"toggle\""),
        "lettres, chiffres",
    );
    assert_invalid(
        &with("name = \"accessibility\"\ntype = \"toggle\""),
        "nom réservé",
    );
    let twice = format!(
        "{}\n[[options]]\nname = \"x\"\ntype = \"toggle\"\n",
        with("name = \"x\"\ntype = \"toggle\"")
    );
    assert_invalid(&twice, "en double");
}

#[test]
fn classifications_et_cas() {
    let with = |c: &str| {
        minimal_with(
            "classification = \"filler\"",
            &format!("classification = {c}"),
        )
    };
    assert_invalid(&with("\"rare\""), "classification « rare » inconnue");
    assert_invalid(&with("\"filler+useful\""), "ne se combine avec rien");
    assert_invalid(&with("\"skip_balancing\""), "demande « progression »");
    assert_invalid(&with("[]"), "liste de cas vide");
    assert_invalid(
        &with("[{ when = \"true\", value = \"filler\" }]"),
        "le dernier cas",
    );
    assert_invalid(
        &with("[{ value = \"useful\" }, { value = \"filler\" }]"),
        "doit avoir un « when »",
    );
    assert_invalid(
        &with("[{ when = \"has(Key)\", value = \"useful\" }, { value = \"filler\" }]"),
        "ne dépend que des options",
    );
    assert_invalid(
        &with("[{ when = \"oops(\", value = \"useful\" }, { value = \"filler\" }]"),
        "objet « Coin »",
    );
}

#[test]
fn filler_incomplet() {
    let text = minimal_with(
        "[filler]\nitem = \"Coin\"",
        "[filler]\nitem = \"Coin\"\ntrap = \"Key\"",
    );
    assert_invalid(&text, "vont ensemble");
    let text = minimal_with(
        "[filler]\nitem = \"Coin\"",
        "[filler]\nitem = \"Coin\"\ntrap = \"Key\"\ntrap_percent = 101",
    );
    assert_invalid(&text, "entre 0 et 100");
    let text = minimal_with(
        "[filler]\nitem = \"Coin\"",
        "[filler]\nitem = \"Coin\"\ntrap = \"Key\"\ntrap_percent = \"option(nope)\"",
    );
    assert_invalid(&text, "option « nope » inconnue");
}

// ---- Compilation ----

fn build_err(text: &str, options: &[(&str, i64)]) -> BuildError {
    let world = parse_world(text).unwrap();
    let mut values = world.default_values();
    for (name, value) in options {
        world.set_option(&mut values, name, *value).unwrap();
    }
    let mut w = MultiWorld::new();
    let slot = w.add_slot("Alice", "X").unwrap();
    let mut rng = ChaCha20Rng::seed_from_u64(0);
    world.build(&values, &mut w, slot, &mut rng).unwrap_err()
}

const TOGGLE: &str = "\n[[options]]\nname = \"on\"\ntype = \"toggle\"\n";

#[test]
fn compilation_element_absent() {
    // La région disparaît sans l'option, mais l'entrée et le lieu y mènent toujours.
    let text = format!(
        "{}{TOGGLE}",
        minimal_with(
            "name = \"Vault\"",
            "name = \"Vault\"\nwhen = \"option(on)\""
        )
    );
    let e = build_err(&text, &[]);
    assert!(e.to_string().contains("absente avec ces options"), "{e}");
}

#[test]
fn compilation_region_inaccessible_avec_les_options() {
    let text = format!(
        "{}{TOGGLE}",
        minimal_with("to = \"Vault\"", "to = \"Vault\"\nwhen = \"option(on)\"")
    );
    let e = build_err(&text, &[]);
    assert!(
        e.to_string().contains("inaccessible avec ces options"),
        "{e}"
    );
}

#[test]
fn compilation_pool_et_lieux() {
    let text = minimal_with(
        "[[pool]]\nitem = \"Key\"",
        "[[pool]]\nitem = \"Key\"\ncount = 3",
    );
    assert!(build_err(&text, &[])
        .to_string()
        .contains("plus d'objets dans le pool"));
    let huge = minimal_with(
        "[[pool]]\nitem = \"Key\"",
        "[[pool]]\nitem = \"Key\"\ncount = 4000000000",
    );
    assert!(build_err(&huge, &[])
        .to_string()
        .contains("plus d'objets dans le pool"));
    let text = minimal_with("[filler]\nitem = \"Coin\"", "");
    assert!(build_err(&text, &[])
        .to_string()
        .contains("pas de [filler]"));
}

#[test]
fn valeurs_d_options_refusees() {
    let world = parse_world(APQUEST).unwrap();
    let mut values = world.default_values();
    assert!(matches!(
        world.set_option(&mut values, "trap_chance", 101),
        Err(BuildError::BadOption { .. })
    ));
    assert!(matches!(
        world.set_option(&mut values, "hammer", 2),
        Err(BuildError::BadOption { .. })
    ));
    assert!(matches!(
        world.set_option(&mut values, "player_sprite", 4),
        Err(BuildError::BadOption { .. })
    ));
    assert!(matches!(
        world.set_option(&mut values, "nope", 0),
        Err(BuildError::BadOption { .. })
    ));
    // Valeurs d'un autre monde.
    let other = parse_world(MINIMAL).unwrap().default_values();
    assert!(world.check_values(&other).is_err());
}

// ---- Robustesse ----

/// Abîme le texte au hasard (suppressions, remplacements, insertions) : la lecture doit
/// toujours rendre un résultat, jamais paniquer.
#[test]
fn texte_abime_ne_fait_jamais_paniquer() {
    let mut rng = ChaCha20Rng::seed_from_u64(41);
    let alphabet: Vec<char> = "()\"',=[]{}.+-_ \nabcz019#\\éxyonr".chars().collect();
    let source: Vec<char> = APQUEST.chars().collect();
    for _ in 0..400 {
        let mut text = source.clone();
        for _ in 0..(1 + rng.next_u32() % 8) {
            let at = (rng.next_u32() as usize) % text.len().max(1);
            let c = alphabet[(rng.next_u32() as usize) % alphabet.len()];
            match rng.next_u32() % 3 {
                0 => {
                    text.remove(at);
                }
                1 => text[at] = c,
                _ => text.insert(at, c),
            }
        }
        let text: String = text.into_iter().collect();
        // Si le monde est encore valide, il doit aussi se compiler sans panique.
        if let Ok(world) = parse_world(&text) {
            let mut w = MultiWorld::new();
            let slot = w.add_slot("Alice", "X").unwrap();
            let _ = world.build(&world.default_values(), &mut w, slot, &mut rng);
        }
    }
}

/// Des règles faites de jetons au hasard : erreur ou règle, jamais de panique.
#[test]
fn regles_au_hasard_ne_font_jamais_paniquer() {
    let mut rng = ChaCha20Rng::seed_from_u64(7);
    let words = [
        "has",
        "has_all",
        "has_group",
        "can_reach",
        "option",
        "not",
        "and",
        "or",
        "true",
        "(",
        ")",
        ",",
        "Key",
        "\"Coin\"",
        "2",
        "==",
        "<",
        "Start",
        "Chest",
        "\"",
        "-",
    ];
    for _ in 0..2_000 {
        let n = 1 + rng.next_u32() % 12;
        let rule: Vec<&str> = (0..n)
            .map(|_| words[(rng.next_u32() as usize) % words.len()])
            .collect();
        let text = minimal_with(
            "rule = \"has(Key)\"",
            &format!("rule = '{}'", rule.join(" ")),
        );
        let _ = parse_world(&text);
    }
}
