//! Écran « Créer une partie », étape 1 : la préparation.
//!
//! L'hôte choisit les fichiers YAML des joueurs (un par joueur, ADR 0019 : par la
//! boîte de fichiers seulement), d'éventuels apworlds, puis les réglages du serveur.
//! `build_request` vérifie le tout et en fait une demande de génération et des
//! réglages d'hébergement (`docs/contrat-ui-generation.md`).
//!
//! La génération elle-même (runtime, progression, annulation) arrive à l'étape 2 :
//! pour l'instant, le bouton « Générer » vérifie seulement le formulaire.

use std::path::PathBuf;

use egui::{RichText, Ui};
use launcher_core::generation::GenerateRequest;
use launcher_core::{Encryption, PartyHostOptions};

use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;

/// Avertissement obligatoire avant d'ajouter un apworld (contrat, « Avertissements »).
pub const APWORLD_WARNING: &str = "Un apworld est du code qui s'exécute sur votre PC, sans bac \
     à sable. N'ajoutez que ceux que vous jugez sûrs.";

/// Boîte de dialogue à ouvrir après le dessin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Yaml,
    Apworld,
    Cert,
    Key,
}

/// Ce que l'hôte a saisi.
pub struct CreateForm {
    /// Fichiers YAML, dans l'ordre d'ajout (sans doublon).
    yamls: Vec<PathBuf>,
    /// Apworlds supplémentaires, dans l'ordre d'ajout (sans doublon).
    apworlds: Vec<PathBuf>,
    /// L'hôte a coché « J'ai compris » sous l'avertissement des apworlds.
    apworld_ack: bool,
    pending_pick: Option<Pick>,
    port: String,
    password: String,
    public_host: String,
    encrypted: bool,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    /// Message de réussite de la vérification (étape 1 : rien n'est encore lancé).
    ready: Option<String>,
}

impl Default for CreateForm {
    /// Mêmes valeurs par défaut que l'écran « Héberger une partie ».
    fn default() -> CreateForm {
        CreateForm {
            yamls: Vec::new(),
            apworlds: Vec::new(),
            apworld_ack: false,
            pending_pick: None,
            port: "38281".to_string(),
            password: String::new(),
            public_host: "localhost".to_string(),
            encrypted: true,
            cert: None,
            key: None,
            ready: None,
        }
    }
}

pub fn show(ui: &mut Ui, form: &mut CreateForm, error: &mut Option<String>) -> Option<Page> {
    let mut goto = None;
    super::page(ui, |ui| {
        super::header(
            ui,
            "Parties › Nouvelle partie",
            "Créer une partie",
            |ui| {
                if widgets::ghost_button(ui, "Héberger un fichier .archipelago") {
                    goto = Some(Page::Hosted);
                }
            },
        );
        if let Some(message) = error.as_deref() {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }

        if ui.available_width() >= super::WIDE {
            ui.columns(2, |columns| {
                players_panel(&mut columns[0], form);
                server_panel(&mut columns[1], form, error);
            });
        } else {
            players_panel(ui, form);
            ui.add_space(12.0);
            server_panel(ui, form, error);
        }
    });
    goto
}

/// Libellé au-dessus d'un champ.
fn labelled(ui: &mut Ui, label: &str) {
    ui.add_space(4.0);
    ui.label(widgets::dim(label).size(13.0));
}

/// Liste de fichiers avec un bouton « Retirer » par ligne.
fn file_list(ui: &mut Ui, files: &mut Vec<PathBuf>, empty: &str) {
    if files.is_empty() {
        ui.label(widgets::dim(empty).size(13.0));
        return;
    }
    // On note l'index à retirer, puis on retire APRÈS la boucle : on ne peut pas
    // modifier un `Vec` pendant qu'on le parcourt (le compilateur le refuse).
    let mut remove = None;
    for (index, path) in files.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(file_name(path)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, "Retirer") {
                    remove = Some(index);
                }
            });
        })
        .response
        .on_hover_text(path.display().to_string());
    }
    if let Some(index) = remove {
        files.remove(index);
    }
}

/// Nom du fichier seul, pour l'affichage.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn players_panel(ui: &mut Ui, form: &mut CreateForm) {
    widgets::panel(ui, "Joueurs", |ui| {
        ui.label(widgets::dim("Un fichier YAML par joueur.").size(13.0));
        ui.add_space(6.0);
        file_list(ui, &mut form.yamls, "Aucun fichier YAML pour l'instant.");
        ui.add_space(6.0);
        if widgets::ghost_button(ui, "Ajouter des YAML…") {
            form.pending_pick = Some(Pick::Yaml);
        }
        let count = form.yamls.len();
        if count > 0 {
            let plural = if count > 1 { "s" } else { "" };
            ui.label(widgets::dim(format!("{count} joueur{plural}")).size(13.0));
        }
    });
    ui.add_space(12.0);
    widgets::panel(ui, "Apworlds (facultatif)", |ui| {
        ui.label(
            RichText::new(APWORLD_WARNING)
                .color(theme::AMBER)
                .size(13.0),
        );
        ui.checkbox(&mut form.apworld_ack, "J'ai compris");
        ui.add_space(6.0);
        file_list(ui, &mut form.apworlds, "Aucun apworld en plus.");
        ui.add_space(6.0);
        // `add_enabled` grise le bouton tant que l'avertissement n'est pas coché.
        let add = ui.add_enabled(
            form.apworld_ack,
            egui::Button::new(RichText::new("Ajouter des apworlds…").color(theme::TEXT)),
        );
        if add.clicked() {
            form.pending_pick = Some(Pick::Apworld);
        }
    });
}

/// Ligne « chemin en lecture seule + Parcourir… ». Renvoie vrai si on a cliqué.
fn file_row(ui: &mut Ui, path: &Option<PathBuf>) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
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

fn server_panel(ui: &mut Ui, form: &mut CreateForm, error: &mut Option<String>) {
    widgets::panel(ui, "Serveur", |ui| {
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
                    form.pending_pick = Some(Pick::Cert);
                }
                ui.label(widgets::dim("Clé privée (PEM)").size(13.0));
                if file_row(ui, &form.key) {
                    form.pending_pick = Some(Pick::Key);
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
        if widgets::primary_button(ui, "▶ Générer la partie", true) {
            match build_request(form) {
                Ok((request, _host)) => {
                    *error = None;
                    let count = request.player_files.len();
                    let plural = if count > 1 { "s" } else { "" };
                    form.ready = Some(format!(
                        "Formulaire valide : {count} joueur{plural}. La génération arrive à \
                         l'étape suivante."
                    ));
                }
                Err(message) => {
                    form.ready = None;
                    *error = Some(message);
                }
            }
        }
        if let Some(message) = &form.ready {
            ui.label(RichText::new(message).color(theme::GREEN).size(13.0));
        }
    });
}

/// Ouvre la boîte de dialogue demandée pendant le dessin.
///
/// Appelée par `App` après avoir relâché le verrou de l'état partagé : la boîte
/// bloque le fil de l'interface tant qu'elle est ouverte.
pub fn open_pending_dialog(form: &mut CreateForm) {
    let Some(pick) = form.pending_pick.take() else {
        return;
    };
    // Le formulaire change : l'ancien « Formulaire valide » ne vaut plus.
    form.ready = None;
    match pick {
        Pick::Yaml => {
            let picked = rfd::FileDialog::new()
                .set_title("Choisir les fichiers YAML des joueurs")
                .add_filter("Réglages de joueur", &["yaml", "yml"])
                .pick_files();
            add_files(&mut form.yamls, picked.unwrap_or_default());
        }
        Pick::Apworld => {
            let picked = rfd::FileDialog::new()
                .set_title("Choisir des apworlds")
                .add_filter("Monde Archipelago", &["apworld"])
                .pick_files();
            add_files(&mut form.apworlds, picked.unwrap_or_default());
        }
        Pick::Cert => {
            form.cert = pick_pem("Certificat PEM", &["pem", "crt"]).or(form.cert.take());
        }
        Pick::Key => {
            form.key = pick_pem("Clé privée PEM", &["pem", "key"]).or(form.key.take());
        }
    }
}

fn pick_pem(name: &str, extensions: &[&str]) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(name, extensions)
        .add_filter("Tous les fichiers", &["*"])
        .pick_file()
}

/// Ajoute des fichiers à la fin de la liste, sans doublon, dans l'ordre reçu.
fn add_files(list: &mut Vec<PathBuf>, picked: Vec<PathBuf>) {
    for path in picked {
        if !list.contains(&path) {
            list.push(path);
        }
    }
}

/// Vérifie le formulaire et en fait la demande de génération et les réglages
/// d'hébergement, ou un message en français (contrat, « Messages d'erreur »).
///
/// Tout est vérifié AVANT de générer : une génération dure des minutes, on ne veut
/// pas la perdre sur un port mal tapé.
fn build_request(form: &CreateForm) -> Result<(GenerateRequest, PartyHostOptions), String> {
    if form.yamls.is_empty() {
        return Err("Ajoutez au moins un fichier YAML.".into());
    }
    if !form.apworlds.is_empty() && !form.apworld_ack {
        return Err(format!("Cochez « J'ai compris » : {APWORLD_WARNING}"));
    }
    let port = format::parse_port(&form.port)?;
    let public_host = form.public_host.trim().to_string();
    if public_host.is_empty() {
        return Err("Indiquez l'adresse que les autres joueurs utiliseront.".into());
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

    // `..GenerateRequest::default()` : les autres champs (seed, spoiler, délai)
    // gardent leur valeur par défaut, comme un constructeur avec valeurs par défaut.
    let request = GenerateRequest {
        player_files: form.yamls.clone(),
        apworlds: form.apworlds.clone(),
        ..GenerateRequest::default()
    };
    let host = PartyHostOptions {
        port,
        password,
        encryption,
        public_host,
        disable_save: false,
    };
    Ok((request, host))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un formulaire déjà rempli et valide (en clair, un joueur).
    fn valid() -> CreateForm {
        CreateForm {
            yamls: vec![PathBuf::from("alice.yaml")],
            encrypted: false,
            ..CreateForm::default()
        }
    }

    #[test]
    fn formulaire_valide() {
        let mut form = valid();
        form.password = "secret".into();
        form.public_host = "  exemple.fr ".into();
        let (request, host) = build_request(&form).unwrap();
        assert_eq!(request.player_files, vec![PathBuf::from("alice.yaml")]);
        assert!(request.apworlds.is_empty());
        assert_eq!(host.port, 38281);
        assert_eq!(host.password.as_deref(), Some("secret"));
        assert_eq!(host.public_host, "exemple.fr");
        assert_eq!(host.encryption, Encryption::Plain);
        assert!(!host.disable_save);
    }

    #[test]
    fn sans_yaml() {
        let mut form = valid();
        form.yamls.clear();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Ajoutez au moins un fichier YAML."
        );
    }

    #[test]
    fn adresse_vide() {
        let mut form = valid();
        form.public_host = "   ".into();
        assert_eq!(
            build_request(&form).unwrap_err(),
            "Indiquez l'adresse que les autres joueurs utiliseront."
        );
    }

    #[test]
    fn port_invalide() {
        let mut form = valid();
        form.port = "0".into();
        assert!(build_request(&form).unwrap_err().contains("port"));
    }

    #[test]
    fn wss_sans_certificat() {
        let mut form = valid();
        form.encrypted = true;
        assert!(build_request(&form).unwrap_err().contains("certificat"));
    }

    #[test]
    fn mot_de_passe_vide_absent() {
        let (_, host) = build_request(&valid()).unwrap();
        assert_eq!(host.password, None);
    }

    #[test]
    fn apworld_sans_accord_refuse() {
        let mut form = valid();
        form.apworlds = vec![PathBuf::from("monde.apworld")];
        assert!(build_request(&form).unwrap_err().contains("J'ai compris"));
        form.apworld_ack = true;
        let (request, _) = build_request(&form).unwrap();
        assert_eq!(request.apworlds, vec![PathBuf::from("monde.apworld")]);
    }

    #[test]
    fn ajout_sans_doublon_dans_l_ordre() {
        let mut list = vec![PathBuf::from("b.yaml")];
        add_files(
            &mut list,
            vec![
                PathBuf::from("a.yaml"),
                PathBuf::from("b.yaml"),
                PathBuf::from("c.yaml"),
            ],
        );
        assert_eq!(
            list,
            vec![
                PathBuf::from("b.yaml"),
                PathBuf::from("a.yaml"),
                PathBuf::from("c.yaml"),
            ]
        );
    }
}
