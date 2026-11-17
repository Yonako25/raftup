//! Les textes de l'interface : durées, statuts, initiales, phrases d'activité…
//!
//! Tout est écrit en fonctions simples, sans egui, pour pouvoir les tester.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use launcher_core::{GameEvent, SlotStatus};

/// Heure actuelle, en secondes depuis 1970 (même échelle que le serveur).
pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Durée courte : `8 s`, `3 min`, `2 h 05`, `1 j 03 h`.
pub fn short_duration(seconds: f64) -> String {
    // `as u64` : les secondes négatives (horloges décalées) deviennent 0.
    let s = seconds.max(0.0) as u64;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{} h {:02}", s / 3600, (s % 3600) / 60)
    } else {
        format!("{} j {:02} h", s / 86_400, (s % 86_400) / 3600)
    }
}

/// `il y a 8 s`, ou `jamais` s'il n'y a pas d'heure.
pub fn ago(now: f64, then: Option<f64>) -> String {
    match then {
        Some(t) => format!("il y a {}", short_duration(now - t)),
        None => "jamais".to_string(),
    }
}

/// Heure locale `HH:MM` (ou `HH:MM:SS`) d'un instant en secondes depuis 1970.
pub fn local_time(seconds: f64, with_seconds: bool) -> String {
    let format = if with_seconds { "%H:%M:%S" } else { "%H:%M" };
    match Local.timestamp_opt(seconds as i64, 0).single() {
        Some(t) => t.format(format).to_string(),
        None => "--:--".to_string(),
    }
}

/// Initiales d'un nom de jeu pour la pastille : `ChecksFinder` donne `CF`.
pub fn initials(game: &str) -> String {
    let words: Vec<&str> = game
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let letters: Vec<char> = if words.len() >= 2 {
        // Plusieurs mots : la première lettre des deux premiers.
        words
            .iter()
            .take(2)
            .filter_map(|w| w.chars().next())
            .collect()
    } else {
        // Un seul mot : ses majuscules (ChecksFinder), sinon ses deux premières lettres.
        let capitals: Vec<char> = game.chars().filter(|c| c.is_uppercase()).take(2).collect();
        if capitals.len() == 2 {
            capitals
        } else {
            game.chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect()
        }
    };
    if letters.is_empty() {
        "?".to_string()
    } else {
        letters.iter().flat_map(|c| c.to_uppercase()).collect()
    }
}

/// « Alice, Bob et Lien ».
pub fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} et {last}", rest.join(", ")),
    }
}

/// « 1 lieu », « 25 lieux ».
pub fn locations_count(n: usize) -> String {
    if n > 1 {
        format!("{n} lieux")
    } else {
        format!("{n} lieu")
    }
}

/// Valide le champ « Port » : un entier de 1 à 65535.
pub fn parse_port(text: &str) -> Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(port) if port >= 1 => Ok(port),
        _ => Err("Le port doit être un nombre entier entre 1 et 65535.".to_string()),
    }
}

/// Mode de `release` / `collect` de la seed, en une expression française.
fn mode_text(mode: &str) -> &'static str {
    if mode.contains("auto") {
        "automatiques à l'objectif"
    } else if mode.contains("goal") {
        "permis après l'objectif"
    } else if mode.contains("enabled") {
        "permis à tout moment"
    } else {
        "désactivés"
    }
}

/// « Options de la seed : release et collect automatiques à l'objectif, coût d'un indice 10 %. »
pub fn seed_options(release_mode: &str, collect_mode: &str, hint_cost: i64) -> String {
    let release = mode_text(release_mode);
    let collect = mode_text(collect_mode);
    let modes = if release == collect {
        format!("release et collect {release}")
    } else {
        format!("release {release}, collect {collect}")
    };
    format!("Options de la seed : {modes}, coût d'un indice {hint_cost} %.")
}

/// Famille de couleur d'un statut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Goal,
    Offline,
    Online,
}

/// Texte et couleur du statut d'un slot dans le tableau « Joueurs ».
pub fn slot_status(slot: &SlotStatus) -> (&'static str, StatusKind) {
    if slot.status == 30 {
        ("Objectif atteint", StatusKind::Goal)
    } else if slot.connections == 0 {
        ("Hors ligne", StatusKind::Offline)
    } else if slot.status == 20 {
        ("En jeu", StatusKind::Online)
    } else if slot.status == 10 {
        ("Prêt", StatusKind::Online)
    } else {
        ("Connecté", StatusKind::Online)
    }
}

/// Part de la progression, entre 0 et 1 ; 0 s'il n'y a aucun emplacement.
pub fn progress(checked: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        (checked as f32 / total as f32).min(1.0)
    }
}

/// Présence d'un joueur : `en ligne`, `vu il y a 3 min` ou `pas encore venu`.
pub fn presence(slot: &SlotStatus, now: f64) -> String {
    if slot.connections > 0 {
        "en ligne".to_string()
    } else {
        match slot.last_activity {
            Some(t) => format!("vu il y a {}", short_duration(now - t)),
            None => "pas encore venu".to_string(),
        }
    }
}

/// Jeu et tags d'un slot, sans le tag `AP` (inutile à montrer) : `ChecksFinder · DeathLink`.
pub fn game_with_tags(slot: &SlotStatus) -> String {
    let mut text = slot.game.clone();
    for tag in slot.tags.iter().filter(|t| t.as_str() != "AP") {
        text.push_str(" · ");
        text.push_str(tag);
    }
    text
}

/// Compteurs des trois cartes (groupes exclus).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    pub players: usize,
    pub connected: usize,
    pub checked: usize,
    pub locations: usize,
    pub goals: usize,
}

pub fn totals(slots: &[SlotStatus]) -> Totals {
    let mut t = Totals::default();
    for slot in slots.iter().filter(|s| !s.is_group) {
        t.players += 1;
        if slot.connections > 0 {
            t.connected += 1;
        }
        t.checked += slot.checked;
        t.locations += slot.total;
        if slot.status == 30 {
            t.goals += 1;
        }
    }
    t
}

/// Texte d'invitation copié dans le presse-papiers.
pub fn invitation(address: &str, has_password: bool, slots: &[SlotStatus]) -> String {
    let password = if has_password {
        "Mot de passe : demandez-le à l'hôte"
    } else {
        "Pas de mot de passe"
    };
    let players: Vec<String> = slots
        .iter()
        .filter(|s| !s.is_group)
        .map(|s| format!("{} ({})", s.name, s.game))
        .collect();
    format!(
        "Partie Raftup (compatible avec Archipelago)\nAdresse : {address}\n{password}\nJoueurs : {}",
        players.join(", ")
    )
}

/// Un morceau d'une phrase d'activité : la couleur dépend de sa nature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Nom de joueur (en gras).
    Name(String),
    /// Nom d'objet (en bleu clair).
    Item(String),
    /// Texte ordinaire.
    Text(String),
}

fn name(s: &str) -> Piece {
    Piece::Name(s.to_string())
}

fn text(s: impl Into<String>) -> Piece {
    Piece::Text(s.into())
}

/// Instant d'un événement, en secondes depuis 1970.
pub fn event_time(event: &GameEvent) -> f64 {
    match event {
        GameEvent::Joined { time, .. }
        | GameEvent::Left { time, .. }
        | GameEvent::ItemSent { time, .. }
        | GameEvent::Chat { time, .. }
        | GameEvent::ServerChat { time, .. }
        | GameEvent::Goal { time, .. } => *time,
    }
}

/// La phrase française d'un événement, en morceaux colorables.
pub fn activity(event: &GameEvent) -> Vec<Piece> {
    match event {
        GameEvent::Joined {
            name: who,
            game,
            tags,
            ..
        } => {
            let mut details = vec![game.as_str()];
            details.extend(tags.iter().map(String::as_str).filter(|t| *t != "AP"));
            vec![
                name(who),
                text(format!(" a rejoint la partie ({})", details.join(", "))),
            ]
        }
        GameEvent::Left { name: who, .. } => vec![name(who), text(" a quitté la partie")],
        GameEvent::ItemSent {
            finder,
            receiver,
            item,
            ..
        } => {
            if finder == receiver {
                vec![
                    name(finder),
                    text(" a trouvé son "),
                    Piece::Item(item.clone()),
                ]
            } else {
                vec![
                    name(finder),
                    text(" a envoyé "),
                    Piece::Item(item.clone()),
                    text(" à "),
                    name(receiver),
                ]
            }
        }
        GameEvent::Chat {
            name: who,
            text: said,
            ..
        } => vec![name(who), text(format!(" : « {said} »"))],
        GameEvent::ServerChat { text: said, .. } => {
            vec![name("Hôte"), text(format!(" : « {said} »"))]
        }
        GameEvent::Goal { name: who, .. } => vec![name(who), text(" a atteint son objectif")],
    }
}

/// La phrase sans couleurs, pour les tests.
#[cfg(test)]
fn plain(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .map(|p| match p {
            Piece::Name(s) | Piece::Item(s) | Piece::Text(s) => s.as_str(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, status: u32, connections: usize) -> SlotStatus {
        SlotStatus {
            team: 0,
            slot: 1,
            name: name.to_string(),
            game: "ChecksFinder".to_string(),
            is_group: false,
            connections,
            tags: vec!["AP".to_string(), "DeathLink".to_string()],
            checked: 12,
            total: 25,
            status,
            last_activity: None,
        }
    }

    #[test]
    fn progression() {
        assert_eq!(progress(0, 0), 0.0);
        assert_eq!(progress(5, 20), 0.25);
        // Plus d'emplacements visités que prévu (ne devrait pas arriver) : la barre reste pleine.
        assert_eq!(progress(30, 20), 1.0);
    }

    #[test]
    fn presences() {
        let mut s = slot("Alice", 20, 1);
        assert_eq!(presence(&s, 1000.0), "en ligne");
        s.connections = 0;
        assert_eq!(presence(&s, 1000.0), "pas encore venu");
        s.last_activity = Some(820.0);
        assert_eq!(presence(&s, 1000.0), "vu il y a 3 min");
    }

    #[test]
    fn durees() {
        assert_eq!(short_duration(8.4), "8 s");
        assert_eq!(short_duration(180.0), "3 min");
        assert_eq!(short_duration(2.0 * 3600.0 + 5.0 * 60.0), "2 h 05");
        assert_eq!(short_duration(26.0 * 3600.0), "1 j 02 h");
        assert_eq!(short_duration(-5.0), "0 s");
        assert_eq!(ago(100.0, Some(92.0)), "il y a 8 s");
        assert_eq!(ago(100.0, None), "jamais");
    }

    #[test]
    fn initiales() {
        assert_eq!(initials("ChecksFinder"), "CF");
        assert_eq!(initials("Super Mario 64"), "SM");
        assert_eq!(initials("Factorio"), "FA");
        assert_eq!(initials("A Link to the Past"), "AL");
        assert_eq!(initials(""), "?");
    }

    #[test]
    fn noms_joints() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(join_names(&names(&["Alice"])), "Alice");
        assert_eq!(join_names(&names(&["Alice", "Bob"])), "Alice et Bob");
        assert_eq!(join_names(&names(&["A", "B", "C"])), "A, B et C");
    }

    #[test]
    fn port() {
        assert_eq!(parse_port(" 38281 "), Ok(38281));
        assert!(parse_port("0").is_err());
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn options_de_la_seed() {
        assert_eq!(
            seed_options("auto", "auto-enabled", 10),
            "Options de la seed : release et collect automatiques à l'objectif, coût d'un indice 10 %."
        );
        assert_eq!(
            seed_options("goal", "disabled", 5),
            "Options de la seed : release permis après l'objectif, collect désactivés, coût d'un indice 5 %."
        );
        assert_eq!(mode_text("enabled"), "permis à tout moment");
    }

    #[test]
    fn statuts() {
        assert_eq!(
            slot_status(&slot("A", 30, 1)),
            ("Objectif atteint", StatusKind::Goal)
        );
        assert_eq!(
            slot_status(&slot("A", 20, 0)),
            ("Hors ligne", StatusKind::Offline)
        );
        assert_eq!(slot_status(&slot("A", 20, 1)).0, "En jeu");
        assert_eq!(slot_status(&slot("A", 10, 1)).0, "Prêt");
        assert_eq!(slot_status(&slot("A", 5, 1)).0, "Connecté");
        assert_eq!(game_with_tags(&slot("A", 5, 1)), "ChecksFinder · DeathLink");
    }

    #[test]
    fn compteurs_sans_groupes() {
        let mut group = slot("Groupe", 0, 0);
        group.is_group = true;
        let slots = vec![slot("A", 30, 1), slot("B", 0, 0), group];
        let t = totals(&slots);
        assert_eq!(
            t,
            Totals {
                players: 2,
                connected: 1,
                checked: 24,
                locations: 50,
                goals: 1
            }
        );
    }

    #[test]
    fn invitation_complete() {
        let slots = vec![slot("Alice", 0, 0)];
        let text = invitation("wss://exemple.fr:38281", false, &slots);
        assert!(text.contains("Adresse : wss://exemple.fr:38281"));
        assert!(text.contains("Pas de mot de passe"));
        assert!(text.ends_with("Joueurs : Alice (ChecksFinder)"));
    }

    #[test]
    fn phrases_d_activite() {
        let joined = GameEvent::Joined {
            time: 0.0,
            slot: 1,
            name: "Alice".into(),
            game: "ChecksFinder".into(),
            tags: vec!["AP".into(), "DeathLink".into()],
        };
        assert_eq!(
            plain(&activity(&joined)),
            "Alice a rejoint la partie (ChecksFinder, DeathLink)"
        );
        let left = GameEvent::Left {
            time: 0.0,
            slot: 1,
            name: "Alice".into(),
        };
        assert_eq!(plain(&activity(&left)), "Alice a quitté la partie");

        let own = GameEvent::ItemSent {
            time: 0.0,
            finder: "Alice".into(),
            receiver: "Alice".into(),
            item: "Map Width".into(),
            location: "x".into(),
            flags: 1,
        };
        assert_eq!(plain(&activity(&own)), "Alice a trouvé son Map Width");
        let sent = GameEvent::ItemSent {
            time: 0.0,
            finder: "Bob".into(),
            receiver: "Lien".into(),
            item: "Map Bombs".into(),
            location: "x".into(),
            flags: 0,
        };
        assert_eq!(plain(&activity(&sent)), "Bob a envoyé Map Bombs à Lien");
        assert!(activity(&sent).contains(&Piece::Item("Map Bombs".into())));

        let chat = GameEvent::Chat {
            time: 0.0,
            slot: 1,
            name: "Alice".into(),
            text: "salut".into(),
        };
        assert_eq!(plain(&activity(&chat)), "Alice : « salut »");
        let host = GameEvent::ServerChat {
            time: 0.0,
            text: "bonne partie".into(),
        };
        assert_eq!(plain(&activity(&host)), "Hôte : « bonne partie »");
        let goal = GameEvent::Goal {
            time: 0.0,
            slot: 1,
            name: "Alice".into(),
        };
        assert_eq!(plain(&activity(&goal)), "Alice a atteint son objectif");
    }
}
