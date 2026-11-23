//! Parties hébergées par Rafthub (ADR 0052, étape 2) : choix du mode dans « Créer une
//! partie », écran de la partie hébergée, ligne dans « En cours ». Sans réseau : l'accès
//! au hub vise le port 1 de ce PC, où personne n'écoute, et aucun clic ne l'appelle.

use egui_kittest::Harness;
use launcher_core::hub_client::{CreatedGame, GameStatus, HostedGame, HubUrl, SessionToken};
use launcher_core::SlotStatus;

use super::{builder, click, demo_catalog, shows, shows_part, type_into};
use crate::app::Page;
use crate::screens::create::{self, CreateForm, HostMode};
use crate::screens::friends::FriendsState;
use crate::screens::hub_game::{self, HubAccess, HubGames};
use crate::screens::matches;
use crate::worker::{Shared, Worker};

fn access() -> HubAccess {
    HubAccess {
        url: HubUrl::parse("http://127.0.0.1:1").unwrap(),
        token: SessionToken::new("jeton"),
    }
}

/// Le salon, avec ou sans accès au hub pour héberger.
struct Salon {
    form: CreateForm,
    error: Option<String>,
    worker: Worker,
    shared: Shared,
    access: Option<HubAccess>,
}

fn salon(access: Option<HubAccess>) -> Harness<'static, Salon> {
    let state = Salon {
        form: CreateForm::for_tests(&demo_catalog()),
        error: None,
        worker: Worker::detached(),
        shared: Shared::default(),
        access,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut Salon| {
            create::show(
                ui,
                &mut s.form,
                &mut s.error,
                &s.worker,
                &s.shared,
                s.access.as_ref(),
            );
        },
        state,
    );
    harness.run();
    harness
}

#[test]
fn sans_rafthub_seul_le_pc() {
    let harness = salon(None);
    assert!(shows(&harness, "Sur ce PC"));
    assert!(!shows(&harness, "Héberger sur Rafthub"));
    assert!(!shows(&harness, "Héberger sur mon PC"));
}

#[test]
fn le_choix_rafthub_remplace_les_reglages_du_serveur() {
    let mut harness = salon(Some(access()));
    // Par défaut : sur ce PC, comme avant.
    assert!(shows(&harness, "Héberger sur mon PC"));
    assert!(shows(&harness, "Héberger sur Rafthub"));
    assert!(shows(&harness, "Sur ce PC"));
    assert_eq!(harness.state().form.host_mode(), HostMode::ThisPc);

    click(&mut harness, "Héberger sur Rafthub");
    assert_eq!(harness.state().form.host_mode(), HostMode::Rafthub);
    assert!(harness.state().form.on_rafthub());
    assert!(shows(&harness, "Sur Rafthub · 127.0.0.1:1"));
    assert!(shows(&harness, "Nom de la partie (facultatif)"));
    assert!(!shows(&harness, "Sur ce PC"));
    assert!(shows_part(&harness, "puis envoyée à Rafthub"));

    // Avec un pseudo, plus rien ne bloque : ni port, ni adresse, ni certificat à fournir.
    type_into(
        &mut harness,
        "Ton nom dans la partie (16 caractères au plus)",
        "Alice",
    );
    harness.state_mut().form_public_host_empty();
    harness.run();
    assert!(create::check_form(&harness.state().form).is_ok());
    // Le même formulaire, sur ce PC, est refusé faute d'adresse.
    click(&mut harness, "Héberger sur mon PC");
    assert_eq!(harness.state().form.host_mode(), HostMode::ThisPc);
    assert!(create::check_form(&harness.state().form).is_err());
    assert!(shows(&harness, "Sur ce PC"));
}

/// Le hub n'est plus proposé (déconnexion) : la partie repart sur ce PC, même si le
/// choix « Rafthub » était resté.
#[test]
fn rafthub_choisi_puis_deconnecte() {
    let mut harness = salon(Some(access()));
    click(&mut harness, "Héberger sur Rafthub");
    harness.state_mut().access = None;
    harness.run();
    assert!(!harness.state().form.on_rafthub());
    assert!(shows(&harness, "Sur ce PC"));
}

impl Salon {
    /// Adresse publique vide : refusée sur ce PC, sans effet sur Rafthub.
    fn form_public_host_empty(&mut self) {
        self.form.set_public_host_for_tests("");
    }
}

#[test]
fn acces_seulement_connecte_a_un_hub_qui_heberge() {
    let mut friends = FriendsState::for_tests();
    assert!(friends.hosting_access().is_none());
    friends.set_logged_in_for_tests("Mimi", false);
    assert!(friends.hosting_access().is_none());
    friends.set_logged_in_for_tests("Mimi", true);
    let access = friends.hosting_access().unwrap();
    assert_eq!(access.host(), "127.0.0.1:1");
}

fn hosted() -> HostedGame {
    HostedGame {
        id: "0123456789abcdef0123456789abcdef".into(),
        name: "Partie de Mimi".into(),
        seed_name: "4242".into(),
        state: "active".into(),
        address: "hub.example.org:38281".into(),
        host: "hub.example.org".into(),
        port: 38281,
        fingerprint: Some("ab".repeat(32)),
        players: 1,
        has_password: true,
        created_at: 0,
    }
}

fn created() -> CreatedGame {
    serde_json::from_value(serde_json::json!({
        "id": "0123456789abcdef0123456789abcdef",
        "name": "Partie de Mimi",
        "seed_name": "4242",
        "state": "active",
        "address": "hub.example.org:38281",
        "host": "hub.example.org",
        "port": 38281,
        "fingerprint": "ab".repeat(32),
        "players": 1,
        "has_password": true,
        "created_at": 0,
        "admin_password": "ADMINPASS1234567",
        "resumed": false,
    }))
    .unwrap()
}

fn status() -> GameStatus {
    let slot = SlotStatus {
        team: 0,
        slot: 1,
        name: "Mimi".into(),
        game: "Slime Rancher".into(),
        is_group: false,
        connections: 1,
        tags: Vec::new(),
        checked: 3,
        total: 10,
        status: 20,
        last_activity: None,
        away: None,
        release_votes: 0,
    };
    let mut json = serde_json::to_value(hosted_view()).unwrap();
    json["is_host"] = true.into();
    json["connections"] = 1.into();
    json["slots"] = serde_json::to_value(vec![slot]).unwrap();
    serde_json::from_value(json).unwrap()
}

/// `HostedGame` en JSON, comme le hub l'écrit.
fn hosted_view() -> serde_json::Value {
    let game = hosted();
    serde_json::json!({
        "id": game.id, "name": game.name, "seed_name": game.seed_name, "state": game.state,
        "address": game.address, "host": game.host, "port": game.port,
        "fingerprint": game.fingerprint, "players": game.players,
        "has_password": game.has_password, "created_at": game.created_at,
    })
}

fn game_screen(status: Option<GameStatus>) -> Harness<'static, HubGames> {
    let mut harness = builder().build_ui_state(
        |ui, games: &mut HubGames| {
            hub_game::show(ui, games);
        },
        HubGames::with_game(created(), status),
    );
    // `run_steps` et non `run` : sans état, la roue d'attente redessine sans cesse.
    harness.run_steps(3);
    harness
}

#[test]
fn la_partie_hebergee_s_affiche() {
    let harness = game_screen(Some(status()));
    assert!(shows(&harness, "Partie de Mimi"));
    assert!(shows(&harness, "hub.example.org:38281"));
    assert!(shows(&harness, "Sur Rafthub"));
    assert!(shows(&harness, "Mimi"));
    assert!(shows(&harness, "3 / 10 lieux"));
    assert!(shows(&harness, "Copier l'invitation"));
    // Le mot de passe d'admin est masqué tant qu'on ne clique pas « Afficher ».
    assert!(shows(&harness, "Mot de passe d'admin"));
    assert!(!shows(&harness, "ADMINPASS1234567"));
}

#[test]
fn afficher_le_mot_de_passe_d_admin() {
    let mut harness = game_screen(Some(status()));
    click(&mut harness, "Afficher");
    assert!(shows(&harness, "ADMINPASS1234567"));
    click(&mut harness, "Masquer");
    assert!(!shows(&harness, "ADMINPASS1234567"));
}

#[test]
fn en_attente_du_premier_etat() {
    let harness = game_screen(None);
    assert!(shows(
        &harness,
        "Lecture de l'état de la partie sur Rafthub…"
    ));
}

#[test]
fn lancer_et_connecter_donne_l_adresse_sans_schema() {
    let mut harness = game_screen(Some(status()));
    click(&mut harness, "Lancer et connecter");
    let request = hub_game::take_launch_request(harness.state_mut()).unwrap();
    // Le mod reçoit `hôte:port` : il essaie wss puis retombe sur ws (ADR 0043).
    assert_eq!(request.address, "hub.example.org:38281");
    assert_eq!(request.slot, "Mimi");
    assert_eq!(request.ap_game, "Slime Rancher");
    assert!(request.needs_password);
}

#[test]
fn supprimer_demande_une_confirmation() {
    let mut harness = game_screen(Some(status()));
    click(&mut harness, "Supprimer la partie du hub");
    assert!(shows(&harness, "Confirmer la suppression"));
    click(&mut harness, "Annuler");
    assert!(!shows(&harness, "Confirmer la suppression"));
}

/// « En cours » liste la partie hébergée ; « Ouvrir » mène à son écran.
#[test]
fn en_cours_liste_la_partie_hebergee() {
    struct State {
        shared: Shared,
        games: HubGames,
        asked: Option<Page>,
    }
    let state = State {
        shared: Shared::default(),
        games: HubGames::with_game(created(), None),
        asked: None,
    };
    let mut harness = builder().build_ui_state(
        |ui, s: &mut State| {
            if let Some(page) = matches::show(ui, &s.shared, None, &mut s.games) {
                s.asked = Some(page);
            }
        },
        state,
    );
    harness.run();
    assert!(shows(&harness, "Partie de Mimi"));
    assert!(shows(&harness, "En ligne sur Rafthub"));
    // Une partie sur le hub : pas de « Aucune partie en cours ».
    assert!(!shows(&harness, "Aucune partie en cours"));
    click(&mut harness, "   Ouvrir   ");
    assert_eq!(harness.state().asked, Some(Page::HubGame));
}
