//! Écran « Partie en cours », vu par l'hôte (maquette « Partie en cours »).
//!
//! En haut, un bandeau (nom, résumé, boutons) et l'adresse à donner aux joueurs.
//! Au centre, des onglets : l'équipage, le chat, le journal du serveur.
//! À droite, la progression de toute la partie et le fil d'activité.

use egui::{Color32, CornerRadius, Frame, Margin, RichText, ScrollArea, Stroke, Ui};
use launcher_core::{
    slots_due, GameEvent, HostStatus, SlotStatus, SlotTiming, AUTO_REFRESH_INTERVAL,
};

use std::collections::BTreeMap;

use egui::TextureHandle;
use launcher_core::art::ArtKind;

use super::library::LibraryState;
use crate::app::Page;
use crate::art_ui;
use crate::format::{self, Piece, StatusKind};
use crate::theme;
use crate::widgets;
use crate::worker::{Command, ReadyCheck, ReadyState, Shared, Worker};

/// Largeur gardée à droite du bandeau pour « En ligne », « Copier l'invitation » et
/// « Arrêter » (mesurée sur les captures : environ 350 px).
const HERO_BUTTONS_WIDTH: f32 = 370.0;

/// Largeur de la colonne de droite (progression, fil d'activité).
const SIDE_WIDTH: f32 = 340.0;

/// Couleurs des barres de progression, une par joueur, à tour de rôle.
const PLAYER_COLORS: [Color32; 5] = [
    theme::ACCENT,
    Color32::from_rgb(0xA7, 0x8B, 0xFA), // Lueur
    theme::GREEN,
    Color32::from_rgb(0xF2, 0x7F, 0xA6),
    theme::AMBER,
];

/// Les onglets du centre de l'écran (0 : « Équipage »).
const TAB_CHAT: usize = 1;
const TAB_JOURNAL: usize = 2;
const TAB_SALON: usize = 3;

/// Ce que l'hôte a saisi ou coché sur cet écran.
pub struct RunningState {
    /// Message à tous les joueurs, en cours de frappe.
    announce: String,
    /// Case « Détails » du journal.
    details: bool,
    /// La fenêtre « Arrêter le serveur ? » est ouverte.
    confirm_stop: bool,
    /// Onglet ouvert (0 pour l'équipage, `TAB_CHAT` ou `TAB_JOURNAL`).
    tab: usize,
    /// Mot de passe de la salle, retapé pour les tests « prêt ? » (jamais enregistré).
    salon_password: String,
    /// Case « Rafraîchir automatiquement » du salon (cochée par défaut).
    salon_auto_refresh: bool,
    /// Moment de la dernière copie : la bulle « Copié » s'affiche pendant `COPIED_FOR`.
    copied_at: Option<std::time::Instant>,
    /// Le slot que l'hôte joue lui-même (choisi dans « Ta place »), par son nom.
    my_slot: Option<String>,
    /// Clic sur « Lancer et connecter », pas encore traité par l'application.
    launch_request: Option<LaunchRequest>,
}

/// « Lancer et connecter » : le jeu à lancer et où le connecter. L'application la
/// transmet à la Bibliothèque, qui connaît les jeux installés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// Nom du jeu dans Archipelago (`game` du slot).
    pub ap_game: String,
    pub address: String,
    pub slot: String,
    pub needs_password: bool,
}

/// Prend la demande « Lancer et connecter » en attente, s'il y en a une.
pub fn take_launch_request(state: &mut RunningState) -> Option<LaunchRequest> {
    state.launch_request.take()
}

/// Durée d'affichage de la bulle « Copié dans le presse-papiers ».
const COPIED_FOR: std::time::Duration = std::time::Duration::from_secs(2);

// `Default` écrit à la main : `#[derive(Default)]` mettrait la case à `false`.
impl Default for RunningState {
    fn default() -> Self {
        RunningState {
            announce: String::new(),
            details: false,
            confirm_stop: false,
            tab: 0,
            salon_password: String::new(),
            salon_auto_refresh: true,
            copied_at: None,
            my_slot: None,
            launch_request: None,
        }
    }
}

/// Macro de développement : ouvre l'onglet « Salon ».
#[cfg(debug_assertions)]
pub fn dev_open_salon(state: &mut RunningState) {
    state.tab = TAB_SALON;
}

/// Jaquettes des jeux de la partie, par nom de jeu chez Archipelago (ADR 0051).
type Covers = BTreeMap<String, TextureHandle>;

/// `library` : la Bibliothèque, qui connaît les jeux du catalogue et leurs
/// illustrations venues du hub (jaquettes de l'équipage, image du bandeau).
pub fn show(
    ui: &mut Ui,
    state: &mut RunningState,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
    library: &mut LibraryState,
) -> Option<Page> {
    // `show` n'est appelé que si la partie tourne, mais on reste prudent : pas de `unwrap`.
    let status = shared.status.as_ref()?;
    let now = format::now();
    let mut copy: Option<String> = None;
    let mut goto = None;

    // Illustrations : une jaquette par jeu de la partie, et l'image large du jeu de
    // l'hôte (sinon du premier joueur) pour le bandeau, comme sur la maquette.
    let ctx = ui.ctx().clone();
    let mut covers = Covers::new();
    for slot in playable(status) {
        if !covers.contains_key(&slot.game) {
            if let Some(texture) = library.art_for_ap_game(&ctx, &slot.game, ArtKind::Cover) {
                covers.insert(slot.game.clone(), texture);
            }
        }
    }
    let banner_game = chosen_slot(status, state.my_slot.as_deref())
        .or_else(|| playable(status).next())
        .map(|slot| slot.game.clone());
    let banner = banner_game.and_then(|game| library.art_for_ap_game(&ctx, &game, ArtKind::Hero));

    super::page(ui, |ui| {
        if hero(
            ui,
            status,
            now,
            &mut copy,
            &mut state.confirm_stop,
            banner.as_ref(),
        ) {
            goto = Some(Page::InProgress);
        }
        ui.add_space(12.0);
        address_bar(ui, status, &mut copy);
        ui.add_space(12.0);
        if let Some(request) = my_place(ui, &mut state.my_slot, status) {
            state.launch_request = Some(request);
        }
        ui.add_space(14.0);
        if let Some(message) = error.as_deref() {
            widgets::error_banner(ui, message);
            ui.add_space(12.0);
        }

        if ui.available_width() >= super::WIDE {
            // Deux colonnes : le centre prend ce qui reste à côté de la colonne de droite.
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0;
                let main_width = ui.available_width() - SIDE_WIDTH - 20.0;
                column(ui, main_width, |ui| {
                    main_area(ui, state, worker, shared, now, &mut copy, &covers)
                });
                column(ui, SIDE_WIDTH, |ui| side_area(ui, shared, status, now));
            });
        } else {
            main_area(ui, state, worker, shared, now, &mut copy, &covers);
            ui.add_space(14.0);
            side_area(ui, shared, status, now);
        }
    });

    if let Some(text) = copy {
        ui.ctx().copy_text(text);
        state.copied_at = Some(std::time::Instant::now());
    }
    // Sans retour visible, on croit que « Copier » n'a rien fait (essai du 18 novembre).
    if let Some(at) = state.copied_at {
        if at.elapsed() < COPIED_FOR {
            copied_bubble(ui.ctx());
            // Redessiner à la fin du délai pour faire disparaître la bulle.
            ui.ctx()
                .request_repaint_after(COPIED_FOR.saturating_sub(at.elapsed()));
        } else {
            state.copied_at = None;
        }
    }
    if state.confirm_stop {
        confirm_window(ui.ctx(), state, worker);
    }
    goto
}

/// Une colonne de largeur fixe, remplie de haut en bas.
fn column(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui)) {
    // Toute la hauteur disponible : avec 0, les zones défilantes de la colonne
    // se retrouveraient écrasées.
    let height = ui.available_height();
    ui.allocate_ui_with_layout(
        egui::vec2(width, height),
        egui::Layout::top_down(egui::Align::LEFT),
        |ui| {
            ui.set_width(width);
            add_contents(ui);
        },
    );
}

/// Bandeau du haut : retour, nom de la partie, résumé, état et boutons.
/// Renvoie vrai si l'hôte demande à revenir à la liste.
fn hero(
    ui: &mut Ui,
    status: &HostStatus,
    now: f64,
    copy: &mut Option<String>,
    confirm_stop: &mut bool,
    banner: Option<&TextureHandle>,
) -> bool {
    // Image large du jeu en fond du bandeau (ADR 0051). Sa place est réservée AVANT le
    // cadre : elle sera dessinée sous lui, une fois la taille du cadre connue.
    let background = ui.painter().add(egui::Shape::Noop);
    // Avec une image, le cadre devient un voile sombre : le texte reste lisible.
    let fill = if banner.is_some() {
        Color32::from_rgba_unmultiplied(0x11, 0x13, 0x16, 170)
    } else {
        theme::panel_fill()
    };
    let mut back = false;
    let players = status.server.slots.iter().filter(|s| !s.is_group).count();
    let plural = if players > 1 { "s" } else { "" };
    let mode = match (status.encrypted, status.fingerprint.is_some()) {
        (true, true) => "chiffrée par Raftup",
        (true, false) => "chiffrée (wss)",
        (false, _) => "en clair (ws)",
    };
    let resumed = if status.resumed {
        " · reprise de la sauvegarde"
    } else {
        ""
    };
    let frame_rect = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(18, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Avec une image, le bandeau est plus haut pour la laisser voir.
            if banner.is_some() {
                ui.set_min_height(120.0);
            }
            ui.horizontal(|ui| {
                back = widgets::ghost_button(ui, "‹");
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    // Place gardée à droite pour la pastille et les deux boutons : un
                    // sous-titre long passe à la ligne au lieu de glisser sous eux.
                    ui.set_max_width((ui.available_width() - HERO_BUTTONS_WIDTH).max(200.0));
                    // Le numéro de seed reste au survol, pour qui en a besoin.
                    ui.label(widgets::title(
                        &format::party_title(&status.server.slots),
                        28.0,
                    ))
                    .on_hover_text(format!("Seed {}", status.seed.seed_name));
                    ui.label(widgets::dim(format!(
                        "{players} joueur{plural} · hébergée sur ce PC · {mode} · démarrée il y a {}{resumed}",
                        format::short_duration(now - status.started_at)
                    )));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::danger_button(ui, "■ Arrêter") {
                        *confirm_stop = true;
                    }
                    // Toutes les invitations d'un coup (une ligne par joueur), à coller
                    // dans le salon Discord de l'équipe.
                    if widgets::primary_button(ui, "Copier les invitations", true) {
                        *copy = Some(format::invitation(
                            &status.address,
                            status.fingerprint.as_deref(),
                            status.has_password,
                            &status.server.slots,
                        ));
                    }
                    widgets::pill(ui, "● En ligne", theme::GREEN, theme::GREEN_BG);
                });
            });
        })
        .response
        .rect;
    if let Some(texture) = banner {
        // L'image remplit le bandeau sans être déformée (coupée au centre).
        let uv = art_ui::cover_uv(texture.size_vec2(), frame_rect.size());
        let image = egui::Shape::image(texture.id(), frame_rect, uv, Color32::WHITE);
        ui.painter().set(background, image);
    }
    back
}

/// Barre fine avec l'adresse à donner aux joueurs.
fn address_bar(ui: &mut Ui, status: &HostStatus, copy: &mut Option<String>) {
    Frame::new()
        .fill(theme::panel_fill())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(18, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Certificat généré par Raftup (ADR 0043) : l'adresse sans schéma, pour qu'un
            // mod de jeu essaie wss puis retombe sur ws ; les liens portent l'empreinte.
            let shown = format::game_address(&status.address, status.fingerprint.as_deref());
            ui.horizontal_wrapped(|ui| {
                ui.label("Adresse à donner aux joueurs :");
                ui.label(widgets::mono(&shown, 15.0).color(theme::ITEM));
                let password = if status.has_password {
                    "avec mot de passe"
                } else {
                    "sans mot de passe"
                };
                ui.label(widgets::dim(format!("· {password}")));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::ghost_button(ui, "Copier l'adresse") {
                        *copy = Some(shown.clone());
                    }
                });
            });
            if format::is_local_host(&status.address) {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Cette adresse ne marche que sur ce PC : indique ton adresse publique pour inviter des amis.",
                    )
                    .size(13.0)
                    .color(theme::AMBER),
                );
            }
        });
}

/// Petite jaquette (40 px) du jeu d'un slot : l'illustration du hub si elle est
/// arrivée (ADR 0051), sinon l'aplat avec les initiales du jeu.
fn game_cover(ui: &mut Ui, slot: &SlotStatus, covers: &Covers) {
    super::library::small_cover_with(ui, 40.0, &slot.game, &slot.game, covers.get(&slot.game));
}

/// Les slots que l'hôte peut jouer lui-même (pas les groupes), dans l'ordre de la seed.
fn playable(status: &HostStatus) -> impl Iterator<Item = &SlotStatus> {
    status.server.slots.iter().filter(|slot| !slot.is_group)
}

/// Le slot choisi s'il existe toujours ; sinon le seul slot de la partie s'il n'y en
/// a qu'un (partie solo) ; sinon aucun.
fn chosen_slot<'a>(status: &'a HostStatus, my_slot: Option<&str>) -> Option<&'a SlotStatus> {
    if let Some(name) = my_slot {
        if let Some(slot) = playable(status).find(|slot| slot.name == name) {
            return Some(slot);
        }
    }
    let mut slots = playable(status);
    match (slots.next(), slots.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

/// Progression d'un slot en texte : « 12 / 40 lieux · objectif atteint ».
fn progress_text(slot: &SlotStatus) -> String {
    let mut text = format!("{} / {} lieux", slot.checked, slot.total);
    if slot.status >= 30 {
        text.push_str(" · objectif atteint");
    }
    text
}

/// Panneau « Ta place dans la partie » : l'hôte dit quel slot il joue, puis un seul
/// bouton lance son jeu connecté à la partie (plan de l'interface, étape 3).
/// Renvoie la demande de lancement au clic.
fn my_place(
    ui: &mut Ui,
    my_slot: &mut Option<String>,
    status: &HostStatus,
) -> Option<LaunchRequest> {
    let mut request = None;
    // `chosen` emprunte `status`, pas `my_slot` : on peut changer `my_slot` plus bas
    // (le nouveau choix s'affiche à l'image suivante).
    let chosen = chosen_slot(status, my_slot.as_deref());
    Frame::new()
        .fill(theme::panel_fill())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(18, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label("Je joue :");
                let shown = chosen.map_or("choisis ton joueur".to_string(), |slot| {
                    format!("{} · {}", slot.name, slot.game)
                });
                egui::ComboBox::from_id_salt("mon_slot")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for slot in playable(status) {
                            let selected = chosen.map(|c| &c.name) == Some(&slot.name);
                            let text = format!("{} · {}", slot.name, slot.game);
                            if ui.selectable_label(selected, text).clicked() {
                                *my_slot = Some(slot.name.clone());
                            }
                        }
                    });
                if let Some(slot) = chosen {
                    ui.label(widgets::dim(progress_text(slot)));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::primary_button(ui, "Lancer et connecter", chosen.is_some()) {
                        request = chosen.map(|slot| LaunchRequest {
                            ap_game: slot.game.clone(),
                            address: status.address.clone(),
                            slot: slot.name.clone(),
                            needs_password: status.has_password,
                        });
                    }
                });
            });
            if chosen.is_none() {
                ui.label(
                    widgets::dim(
                        "Choisis ton joueur : Raftup lance ton jeu et le connecte à la partie.",
                    )
                    .size(13.0),
                );
            }
        });
    request
}

/// Fenêtre de confirmation avant d'arrêter le serveur.
fn confirm_window(ctx: &egui::Context, state: &mut RunningState, worker: &Worker) {
    egui::Window::new("Arrêter le serveur ?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Tous les joueurs seront déconnectés. La partie reste sauvegardée.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::danger_button(ui, "■ Arrêter") {
                    worker.send(Command::Stop);
                    state.confirm_stop = false;
                }
                if widgets::ghost_button(ui, "Annuler") {
                    state.confirm_stop = false;
                }
            });
        });
}

/// Le centre : les onglets et leur contenu.
fn main_area(
    ui: &mut Ui,
    state: &mut RunningState,
    worker: &Worker,
    shared: &Shared,
    now: f64,
    copy: &mut Option<String>,
    covers: &Covers,
) {
    let Some(status) = &shared.status else {
        return;
    };
    let players = status.server.slots.iter().filter(|s| !s.is_group).count();
    let chats = shared.events.iter().filter(|e| is_chat(&e.event)).count();
    widgets::tabs(
        ui,
        &[
            ("Équipage", Some(players)),
            ("Chat", Some(chats)),
            ("Journal", None),
            ("Vérifier les joueurs", None),
        ],
        &mut state.tab,
    );
    ui.add_space(14.0);
    match state.tab {
        TAB_CHAT => chat_tab(ui, state, worker, shared),
        TAB_JOURNAL => journal_tab(ui, state, shared),
        TAB_SALON => salon_tab(ui, state, status, worker, shared, covers),
        // L'onglet 0 (« Équipage ») et toute valeur inattendue montrent l'équipage.
        _ => crew_tab(ui, status, now, copy, covers),
    }
}

/// Onglet « Équipage » : une ligne par joueur, avec sa progression et son statut.
fn crew_tab(
    ui: &mut Ui,
    status: &HostStatus,
    now: f64,
    copy: &mut Option<String>,
    covers: &Covers,
) {
    list_frame(ui, |ui| {
        // Les groupes (« item links ») ne jouent pas eux-mêmes : on ne les montre pas.
        let players = status.server.slots.iter().filter(|s| !s.is_group);
        for (index, slot) in players.enumerate() {
            if index > 0 {
                ui.separator();
            }
            crew_row(
                ui,
                slot,
                status,
                now,
                PLAYER_COLORS[index % PLAYER_COLORS.len()],
                copy,
                covers,
            );
        }
    });
}

fn crew_row(
    ui: &mut Ui,
    slot: &SlotStatus,
    status: &HostStatus,
    now: f64,
    color: Color32,
    copy: &mut Option<String>,
    covers: &Covers,
) {
    ui.horizontal(|ui| {
        game_cover(ui, slot, covers);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(&slot.name)
                    .size(15.0)
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            ui.label(
                widgets::dim(format!(
                    "{} · {}",
                    format::game_with_tags(slot),
                    format::presence(slot, now)
                ))
                .size(12.0),
            );
            // Suivi asynchrone (ADR 0031) : absence et votes de release.
            let note = format::follow_note(slot);
            if !note.is_empty() {
                ui.label(RichText::new(note).size(12.0).color(theme::AMBER));
            }
            // Lien personnel du joueur (absent si l'adresse ne permet pas d'en fabriquer un).
            if let Some(link) =
                format::player_link(&status.address, status.fingerprint.as_deref(), slot)
            {
                let button = egui::Button::new(widgets::dim("Copier l'invitation").size(12.0))
                    .frame(false);
                if ui.add(button).clicked() {
                    *copy = Some(link);
                }
            }
        });
        // De droite à gauche : le statut tout à droite, puis la progression.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Largeur fixe pour l'étiquette : les barres restent alignées d'une ligne à l'autre.
            ui.allocate_ui_with_layout(
                egui::vec2(150.0, 28.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    // Sans largeur minimale, la zone réservée se réduit à celle de l'étiquette.
                    ui.set_min_width(150.0);
                    status_tag(ui, slot);
                },
            );
            ui.add_space(8.0);
            let width = (ui.available_width() - 40.0).clamp(80.0, 320.0);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 36.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    widgets::bar(ui, width, format::progress(slot.checked, slot.total), color);
                    ui.label(
                        widgets::dim(format!("{} / {} lieux", slot.checked, slot.total))
                            .size(12.0),
                    );
                },
            );
        });
    });
}

/// Étiquette de statut en capitales (« EN JEU », « OBJECTIF ATTEINT »…).
fn status_tag(ui: &mut Ui, slot: &SlotStatus) {
    let (text, kind) = format::slot_status(slot);
    let (fg, bg) = match kind {
        StatusKind::Goal => (theme::GREEN, theme::GREEN_BG),
        StatusKind::Online => (theme::TEXT, Color32::from_white_alpha(14)),
        StatusKind::Offline => (theme::TEXT_DIM, Color32::from_white_alpha(6)),
    };
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text.to_uppercase())
                    .size(12.0)
                    .strong()
                    .family(theme::bold_family())
                    .extra_letter_spacing(1.0)
                    .color(fg),
            );
        });
}

/// Cadre commun aux listes et aux cartes de cet écran.
fn list_frame<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(theme::panel_fill())
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
        .inner
}

fn is_chat(event: &GameEvent) -> bool {
    matches!(event, GameEvent::Chat { .. } | GameEvent::ServerChat { .. })
}

/// Un morceau de phrase : nom en gras, objet en bleu clair, le reste normal.
fn piece_label(ui: &mut Ui, piece: &Piece) {
    match piece {
        Piece::Name(s) => ui.label(
            RichText::new(s)
                .strong()
                .family(theme::bold_family())
                .color(theme::TEXT),
        ),
        Piece::Item(s) => ui.label(RichText::new(s).color(theme::ITEM)),
        Piece::Text(s) => ui.label(RichText::new(s).color(theme::TEXT)),
    };
}

/// Onglet « Chat » : les messages des joueurs et de l'hôte, et le champ d'annonce.
fn chat_tab(ui: &mut Ui, state: &mut RunningState, worker: &Worker, shared: &Shared) {
    list_frame(ui, |ui| {
        ScrollArea::vertical()
            .id_salt("chat")
            .max_height(360.0)
            .auto_shrink([false, true])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                compact_rows(ui);
                let mut empty = true;
                for numbered in shared.events.iter().filter(|e| is_chat(&e.event)) {
                    empty = false;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let time = format::local_time(format::event_time(&numbered.event), false);
                        ui.label(widgets::mono(format!("{time}  "), 12.0).color(theme::TEXT_DIM));
                        for piece in format::activity(&numbered.event) {
                            piece_label(ui, &piece);
                        }
                    });
                }
                if empty {
                    ui.label(widgets::dim("Aucun message pour l'instant."));
                }
            });

        ui.add_space(10.0);
        ui.label(widgets::caps("Message à tous les joueurs"));
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut state.announce)
                    .desired_width((ui.available_width() - 100.0).max(80.0)),
            );
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (widgets::primary_button(ui, "Envoyer", true) || enter)
                && !state.announce.trim().is_empty()
            {
                worker.send(Command::Announce(std::mem::take(&mut state.announce)));
            }
        });
    });
}

/// Onglet « Journal » : le journal technique du serveur.
fn journal_tab(ui: &mut Ui, state: &mut RunningState, shared: &Shared) {
    list_frame(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(widgets::caps("Journal du serveur"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut state.details, "Détails");
            });
        });
        Frame::new()
            .fill(theme::LOG_BG)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ScrollArea::vertical()
                    .id_salt("journal")
                    .max_height(420.0)
                    .auto_shrink([false, true])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        compact_rows(ui);
                        for line in &shared.logs {
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let color = match line.level.as_str() {
                                    "ERROR" => theme::RED_TEXT,
                                    "WARN" => theme::AMBER,
                                    _ => theme::TEXT,
                                };
                                let time = format::local_time(line.time, true);
                                ui.label(widgets::mono(time, 12.0).color(theme::TEXT_DIM));
                                ui.label(
                                    widgets::mono(format!("{:<5}", line.level), 12.0).color(color),
                                );
                                ui.label(widgets::mono(&line.text, 12.0).color(color));
                                if state.details && !line.fields.is_empty() {
                                    ui.label(
                                        widgets::mono(&line.fields, 12.0).color(theme::TEXT_DIM),
                                    );
                                }
                            });
                        }
                    });
            });
    });
}

/// La colonne de droite : progression de la partie, puis fil d'activité.
fn side_area(ui: &mut Ui, shared: &Shared, status: &HostStatus, now: f64) {
    let t = format::totals(&status.server.slots);
    list_frame(ui, |ui| {
        ui.label(widgets::caps("Progression de la partie"));
        ui.add_space(4.0);
        ui.label(widgets::title(
            &format!("{} / {}", t.checked, t.locations),
            30.0,
        ));
        ui.label(widgets::dim("lieux trouvés").size(13.0));
        ui.add_space(4.0);
        widgets::bar(
            ui,
            ui.available_width(),
            format::progress(t.checked, t.locations),
            theme::ACCENT,
        );
        ui.add_space(8.0);
        let goal_plural = if t.goals > 1 { "s" } else { "" };
        ui.label(
            widgets::dim(format!(
                "{} / {} joueurs connectés · {} objectif{goal_plural} atteint{goal_plural}",
                t.connected, t.players, t.goals
            ))
            .size(13.0),
        );
        // Suivi asynchrone (ADR 0031) : absents et votes de release en cours.
        let players = status.server.slots.iter().filter(|s| !s.is_group);
        let away = players.clone().filter(|s| s.away.is_some()).count();
        let votes = players.filter(|s| s.release_votes > 0).count();
        if away > 0 || votes > 0 {
            let away_plural = if away > 1 { "s" } else { "" };
            let vote_plural = if votes > 1 { "s" } else { "" };
            ui.label(
                RichText::new(format!(
                    "{away} absent{away_plural} · {votes} vote{vote_plural} de release en cours"
                ))
                .size(13.0)
                .color(theme::AMBER),
            );
        }
    });
    ui.add_space(14.0);

    // Durée estimée calculée à la création de cette seed (ADR 0030) ; rien sinon.
    if let Some(estimate) = super::estimate::for_seed(ui, &status.seed.seed_name) {
        list_frame(ui, |ui| super::estimate::running_panel(ui, &estimate));
        ui.add_space(14.0);
    }

    list_frame(ui, |ui| {
        ui.label(widgets::caps("Fil d'activité"));
        ui.add_space(4.0);
        ScrollArea::vertical()
            .id_salt("activity")
            .max_height(520.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                compact_rows(ui);
                if shared.events.is_empty() {
                    ui.label(widgets::dim("Rien pour l'instant."));
                }
                // Le plus récent en haut : on parcourt la liste à l'envers.
                for numbered in shared.events.iter().rev().take(200) {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for piece in format::activity(&numbered.event) {
                            piece_label(ui, &piece);
                        }
                    });
                    let time = format::event_time(&numbered.event);
                    ui.label(widgets::dim(format::ago(now, Some(time))).size(12.0));
                    ui.add_space(4.0);
                }
            });
    });
}

/// Rangées serrées pour les listes de texte : sans cela, egui donne à chaque rangée
/// la hauteur d'un bouton (32 px).
fn compact_rows(ui: &mut Ui) {
    ui.spacing_mut().interact_size.y = 16.0;
    ui.spacing_mut().item_spacing.y = 4.0;
}

/// Onglet « Salon » (F08) : par joueur, le jeu, le lien d'invitation et le test « prêt ? ».
///
/// Le test ouvre une vraie connexion vers l'adresse publique de la partie, comme le ferait
/// le joueur, puis la referme. Le travail se fait dans le worker : l'écran ne bloque jamais.
fn salon_tab(
    ui: &mut Ui,
    state: &mut RunningState,
    status: &HostStatus,
    worker: &Worker,
    shared: &Shared,
    covers: &Covers,
) {
    let check = |slot: &SlotStatus, password: &str| {
        worker.send(Command::CheckReady(ReadyCheck {
            address: status.address.clone(),
            slot: slot.name.clone(),
            game: slot.game.clone(),
            password: Some(password.to_string()).filter(|p| !p.is_empty()),
            fingerprint: status.fingerprint.clone(),
        }));
    };
    let players: Vec<&SlotStatus> = status.server.slots.iter().filter(|s| !s.is_group).collect();

    // Re-tests automatiques : cette fonction n'est dessinée que si l'onglet est visible et la
    // partie hébergée. Le worker ignore un test demandé pour un slot déjà en cours.
    let timings: Vec<SlotTiming> = players
        .iter()
        .map(|slot| SlotTiming {
            name: &slot.name,
            checking: matches!(shared.readiness.get(&slot.name), Some(ReadyState::Checking)),
            last_tested: shared.readiness_at.get(&slot.name).copied(),
        })
        .collect();
    let due = slots_due(
        state.salon_auto_refresh,
        format::now(),
        AUTO_REFRESH_INTERVAL,
        &timings,
    );
    for slot in players.iter().filter(|s| due.contains(&s.name.as_str())) {
        check(slot, &state.salon_password);
    }
    let last_test = shared
        .readiness_at
        .values()
        .copied()
        .fold(None, |acc, t| Some(acc.map_or(t, |a: f64| a.max(t))));

    list_frame(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if widgets::primary_button(ui, "Tout tester", true) {
                for slot in &players {
                    check(slot, &state.salon_password);
                }
            }
            if status.has_password {
                ui.label(widgets::dim("Mot de passe de la salle :"));
                // Le mot de passe n'est pas gardé dans l'état de la partie : on le retape ici.
                ui.add(
                    egui::TextEdit::singleline(&mut state.salon_password)
                        .password(true)
                        .desired_width(160.0),
                );
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut state.salon_auto_refresh, "Rafraîchir automatiquement");
            let every = format::short_duration(AUTO_REFRESH_INTERVAL);
            let last = match last_test {
                Some(t) => format!("dernier test à {}", format::local_time(t, true)),
                None => "pas encore testé".to_string(),
            };
            ui.label(widgets::dim(format!("(toutes les {every}) · {last}")).size(12.0));
        });
        ui.add_space(4.0);
        ui.label(
            widgets::dim(
                "« Tester » ouvre une vraie connexion pour ce joueur, puis la referme : elle \
                 apparaît brièvement dans le journal du serveur.",
            )
            .size(12.0),
        );
        for slot in &players {
            ui.separator();
            ui.horizontal(|ui| {
                game_cover(ui, slot, covers);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        RichText::new(&slot.name)
                            .size(15.0)
                            .strong()
                            .family(theme::bold_family())
                            .color(theme::TEXT),
                    );
                    ui.label(widgets::dim(format::game_with_tags(slot)).size(12.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let state_of = shared.readiness.get(&slot.name);
                    let checking = matches!(state_of, Some(ReadyState::Checking));
                    if widgets::ghost_button(ui, if checking { "Test…" } else { "Tester" })
                        && !checking
                    {
                        check(slot, &state.salon_password);
                    }
                    match state_of {
                        None => widgets::pill(
                            ui,
                            "Pas testé",
                            theme::TEXT_DIM,
                            Color32::from_white_alpha(6),
                        ),
                        Some(ReadyState::Checking) => widgets::pill(
                            ui,
                            "Test en cours…",
                            theme::AMBER,
                            Color32::from_white_alpha(6),
                        ),
                        Some(ReadyState::Done(result)) => {
                            let (fg, bg) = if result.is_ready() {
                                (theme::GREEN, theme::GREEN_BG)
                            } else {
                                (theme::RED_TEXT, Color32::from_white_alpha(10))
                            };
                            widgets::pill(ui, result.label(), fg, bg);
                        }
                    }
                });
            });
            // Le message complet (quoi faire) sous la ligne, seulement après un échec.
            if let Some(ReadyState::Done(result)) = shared.readiness.get(&slot.name) {
                if !result.is_ready() {
                    ui.label(
                        RichText::new(result.message())
                            .size(12.0)
                            .color(theme::RED_TEXT),
                    );
                }
            }
        }
    });
}

/// Bulle « Copié dans le presse-papiers », en bas au centre de la fenêtre.
fn copied_bubble(ctx: &egui::Context) {
    egui::Area::new(egui::Id::unique("running_copied"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::GREEN_BG)
                .stroke(egui::Stroke::new(1.0, theme::GREEN))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS))
                .inner_margin(egui::Margin::symmetric(12, 6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("Copié dans le presse-papiers")
                            .color(theme::GREEN)
                            .size(13.0),
                    );
                });
        });
}
