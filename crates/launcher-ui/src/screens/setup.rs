//! Écran 1 : « Héberger une partie ».

use std::path::PathBuf;

use egui::{RichText, Ui};
use launcher_core::{Encryption, HostOptions, SeedSummary};

use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::{Command, Shared, Worker};

/// Ce que l'hôte a saisi. `Default` est écrit à la main pour mettre les valeurs par défaut.
pub struct SetupForm {
    seed_path: Option<PathBuf>,
    /// Résumé de la seed choisie, pour montrer ses slots avant le démarrage.
    seed: Option<SeedSummary>,
    port: String,
    password: String,
    public_host: String,
    encrypted: bool,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
}

impl Default for SetupForm {
    fn default() -> SetupForm {
        SetupForm {
            seed_path: None,
            seed: None,
            port: "38281".to_string(),
            password: String::new(),
            public_host: "localhost".to_string(),
            encrypted: true,
            cert: None,
            key: None,
        }
    }
}

pub fn show(
    ui: &mut Ui,
    form: &mut SetupForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) -> Option<Page> {
    let mut goto = None;
    super::page(ui, |ui| {
        super::header(
            ui,
            "Parties › Nouvelle partie",
            "Héberger une partie",
            |ui| {
                // Ces deux écrans arrivent plus tard.
                if widgets::ghost_button(ui, "Créer un salon") {
                    goto = Some(Page::Soon("Créer un salon"));
                }
                if widgets::ghost_button(ui, "Rejoindre par lien") {
                    goto = Some(Page::Soon("Rejoindre par lien"));
                }
            },
        );
        if let Some(message) = error.as_deref() {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }

        // Côte à côte si la fenêtre est assez large, sinon l'un sous l'autre.
        if ui.available_width() >= super::WIDE {
            ui.columns(2, |columns| {
                config_panel(&mut columns[0], form, error, worker, shared);
                slots_panel(&mut columns[1], form.seed.as_ref());
            });
        } else {
            config_panel(ui, form, error, worker, shared);
            ui.add_space(12.0);
            slots_panel(ui, form.seed.as_ref());
        }
    });
    goto
}

/// Un champ de texte avec son libellé au-dessus.
fn labelled(ui: &mut Ui, label: &str) {
    ui.add_space(4.0);
    ui.label(widgets::dim(label).size(13.0));
}

/// Ligne « chemin en lecture seule + bouton Parcourir… ». Renvoie vrai si on a cliqué.
fn file_row(ui: &mut Ui, path: &Option<PathBuf>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        // Le champ est en lecture seule : on lui donne une copie du chemin à afficher.
        let mut shown = path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        ui.add(
            egui::TextEdit::singleline(&mut shown)
                .hint_text("Aucun fichier choisi")
                .interactive(false)
                .desired_width((ui.available_width() - 120.0).max(80.0)),
        );
        clicked = widgets::ghost_button(ui, "Parcourir…");
    });
    clicked
}

fn config_panel(
    ui: &mut Ui,
    form: &mut SetupForm,
    error: &mut Option<String>,
    worker: &Worker,
    shared: &Shared,
) {
    widgets::panel(ui, "Configuration", |ui| {
        labelled(ui, "Fichier de partie (.archipelago)");
        if file_row(ui, &form.seed_path) {
            pick_seed(form, error);
        }
        if let Some(seed) = &form.seed {
            ui.label(
                RichText::new(format!(
                    "Seed {} · générée avec Archipelago {} · {} joueur{}",
                    seed.seed_name,
                    seed.generator_version,
                    seed.players,
                    if seed.players > 1 { "s" } else { "" }
                ))
                .color(theme::GREEN)
                .size(13.0),
            );
        }

        labelled(ui, "Port");
        ui.add(egui::TextEdit::singleline(&mut form.port).desired_width(120.0));

        labelled(ui, "Mot de passe de la salle (facultatif)");
        ui.add(
            egui::TextEdit::singleline(&mut form.password)
                .password(true)
                .desired_width(f32::INFINITY),
        );

        labelled(ui, "Adresse publique (nom de domaine ou IP)");
        ui.add(egui::TextEdit::singleline(&mut form.public_host).desired_width(f32::INFINITY));

        labelled(ui, "Chiffrement");
        let wss = widgets::choice_card(
            ui,
            form.encrypted,
            "Chiffré (wss) — recommandé",
            widgets::dim("Demande un certificat pour votre nom de domaine.").size(13.0),
            |ui| {
                ui.add_space(4.0);
                ui.label(widgets::dim("Certificat (PEM)").size(13.0));
                if file_row(ui, &form.cert) {
                    form.cert = pick_pem("Certificat PEM", &["pem", "crt"]).or(form.cert.take());
                }
                ui.label(widgets::dim("Clé privée (PEM)").size(13.0));
                if file_row(ui, &form.key) {
                    form.key = pick_pem("Clé privée PEM", &["pem", "key"]).or(form.key.take());
                }
            },
        );
        if wss {
            form.encrypted = true;
        }
        ui.add_space(4.0);
        let plain = widgets::choice_card(
            ui,
            !form.encrypted,
            "En clair (ws)",
            RichText::new("Le mot de passe de la salle passe en clair sur le réseau.")
                .color(theme::AMBER)
                .size(13.0),
            |_| {},
        );
        if plain {
            form.encrypted = false;
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, "▶ Démarrer le serveur", !shared.starting) {
                match build_options(form) {
                    Ok(options) => {
                        *error = None;
                        worker.send(Command::Start(options));
                    }
                    Err(message) => *error = Some(message),
                }
            }
            let scheme = if form.encrypted { "wss" } else { "ws" };
            ui.label(
                widgets::dim(format!(
                    "Les joueurs se connecteront à {scheme}://hôte:port"
                ))
                .size(13.0),
            );
        });
        if shared.starting {
            ui.label(widgets::dim("Démarrage…"));
        }
    });
}

/// Ouvre la boîte « choisir la seed » et la lit.
fn pick_seed(form: &mut SetupForm, error: &mut Option<String>) {
    let picked = rfd::FileDialog::new()
        .set_title("Choisir un fichier de partie")
        .add_filter("Partie Archipelago", &["archipelago"])
        .pick_file();
    let Some(path) = picked else { return };
    match SeedSummary::load(&path) {
        Ok((summary, _data)) => {
            *error = None;
            form.seed = Some(summary);
            form.seed_path = Some(path);
        }
        Err(e) => {
            form.seed = None;
            form.seed_path = Some(path);
            *error = Some(e.to_string());
        }
    }
}

fn pick_pem(name: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(name, extensions)
        .add_filter("Tous les fichiers", &["*"])
        .pick_file()
}

/// Vérifie le formulaire et en fait les réglages du serveur, ou un message clair.
fn build_options(form: &SetupForm) -> Result<HostOptions, String> {
    let seed = match (&form.seed_path, &form.seed) {
        (Some(path), Some(_)) => path.clone(),
        _ => return Err("Choisissez d'abord un fichier de partie (.archipelago) valide.".into()),
    };
    let port = format::parse_port(&form.port)?;
    let public_host = form.public_host.trim().to_string();
    if public_host.is_empty() {
        return Err(
            "Indiquez l'adresse publique (nom de domaine ou IP) donnée aux joueurs.".into(),
        );
    }
    let encryption =
        if form.encrypted {
            match (&form.cert, &form.key) {
                (Some(cert), Some(key)) => Encryption::Wss {
                    cert: cert.clone(),
                    key: key.clone(),
                },
                _ => return Err(
                    "Le chiffrement wss demande un certificat et une clé privée (fichiers PEM). \
                     Sans certificat, choisissez « En clair (ws) »."
                        .into(),
                ),
            }
        } else {
            Encryption::Plain
        };
    let password = Some(form.password.clone()).filter(|p| !p.is_empty());
    Ok(HostOptions {
        seed,
        port,
        password,
        encryption,
        public_host,
    })
}

fn slots_panel(ui: &mut Ui, seed: Option<&SeedSummary>) {
    widgets::panel(ui, "Dans cette partie", |ui| {
        let Some(seed) = seed else {
            ui.label(widgets::dim(
                "Choisissez un fichier de partie pour voir ses slots.",
            ));
            return;
        };
        for slot in &seed.slots {
            ui.horizontal(|ui| {
                widgets::badge(ui, 36.0, &format::initials(&slot.game), slot.is_group);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(&slot.name).strong());
                    let subtitle = if slot.is_group {
                        format!("Groupe d'objets · {}", format::join_names(&slot.members))
                    } else {
                        format!(
                            "{} · {}",
                            slot.game,
                            format::locations_count(slot.locations)
                        )
                    };
                    ui.label(widgets::dim(subtitle).size(13.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(widgets::dim(format!("Slot {}", slot.slot)).size(13.0));
                });
            });
            ui.add_space(4.0);
        }
        ui.add_space(6.0);
        ui.label(
            widgets::dim(format::seed_options(
                &seed.release_mode,
                &seed.collect_mode,
                seed.hint_cost,
            ))
            .size(13.0),
        );
    });
}
