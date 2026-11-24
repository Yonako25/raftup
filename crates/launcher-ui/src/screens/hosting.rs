//! Écran « Créer une partie », étape 4 : héberger la seed qui vient d'être générée.
//!
//! L'hôte (`Host`) appartient au worker (`worker.rs`), comme pour l'écran « Héberger
//! une partie » : on lui envoie `Command::Start` avec la seed générée, puis on regarde
//! l'état partagé. Si le serveur démarre, on bascule vers « Partie en cours » (adresse,
//! slots, « Copier l'adresse »). S'il échoue (port occupé…), la seed reste en mémoire :
//! « Réessayer » relit les réglages du formulaire et redémarre SANS régénérer
//! (`docs/contrat-ui-generation.md`, états 4 et 5).

use egui::{RichText, Ui};
use launcher_core::generation::GenerateOutput;
use launcher_core::{HostOptions, PartyError, PartyHostOptions};

use crate::theme;
use crate::widgets;
use crate::worker::{Command, Shared, Worker};

/// Message quand une partie tourne déjà : le worker ignorerait la demande en silence.
pub const ALREADY_HOSTING: &str =
    "Une partie est déjà hébergée. Arrêtez-la avant d'héberger celle-ci.";

/// Où en est l'hébergement de la seed générée.
#[derive(Default)]
pub enum Hosting {
    /// Rien de demandé.
    #[default]
    Idle,
    /// `Command::Start` envoyé, on attend la réponse du worker.
    Starting {
        output: Box<GenerateOutput>,
        port: u16,
    },
    /// Le démarrage a échoué ; la seed est gardée pour « Réessayer ».
    Failed {
        output: Box<GenerateOutput>,
        message: String,
    },
}

/// Ce que l'écran doit faire après avoir lu la réponse du worker.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// Le serveur tourne : aller à « Partie en cours ».
    Started,
}

impl Hosting {
    /// Vrai pendant le démarrage : le bouton « Générer » est alors grisé.
    pub fn is_busy(&self) -> bool {
        matches!(self, Hosting::Starting { .. })
    }

    /// La seed d'un démarrage raté, à réutiliser pour « Réessayer ».
    pub fn failed_output(&self) -> Option<&GenerateOutput> {
        match self {
            Hosting::Failed { output, .. } => Some(output),
            _ => None,
        }
    }
}

/// Prépare les réglages de l'hôte pour cette seed, ou un message pour l'hôte.
///
/// Fonction pure (sans worker ni écran) : c'est elle que les tests vérifient.
fn host_options(
    output: &GenerateOutput,
    options: PartyHostOptions,
    already_hosting: bool,
) -> Result<HostOptions, String> {
    if already_hosting {
        return Err(ALREADY_HOSTING.into());
    }
    options
        .into_host_options(output.archipelago_file.clone())
        .map_err(|e| party_error_message(&e))
}

/// Message en français pour une erreur de `PartyHostOptions` (contrat, « Messages d'erreur »).
fn party_error_message(error: &PartyError) -> String {
    match error {
        PartyError::EmptyPublicHost => {
            "Indique l'adresse que les autres joueurs utiliseront.".into()
        }
        PartyError::AlreadyRunning => ALREADY_HOSTING.into(),
        other => start_failed(&other.to_string()),
    }
}

/// « Impossible de démarrer le serveur : {raison}. », sans doubler le point final.
fn start_failed(reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    format!("Impossible de démarrer le serveur : {reason}.")
}

/// Raison lisible : « port déjà utilisé » au lieu du long texte du système.
///
/// Windows écrit « … (os error 10048) », Linux « … (os error 98) » et macOS
/// « … (os error 48) » ; les deux derniers disent aussi « Address already in use ».
fn readable_reason(reason: &str, port: u16) -> String {
    let in_use = ["(os error 10048)", "(os error 98)", "(os error 48)"]
        .iter()
        .any(|code| reason.contains(code))
        || reason.contains("Address already in use");
    if in_use {
        format!("le port {port} est déjà utilisé par un autre programme, choisis-en un autre")
    } else {
        reason.to_string()
    }
}

/// Demande au worker d'héberger la seed. Rend le nouvel état.
pub fn start(
    worker: &Worker,
    shared: &Shared,
    output: GenerateOutput,
    options: PartyHostOptions,
) -> Hosting {
    let port = options.port;
    match host_options(&output, options, shared.status.is_some()) {
        Ok(host) => {
            worker.send(Command::Start(host));
            Hosting::Starting {
                output: Box::new(output),
                port,
            }
        }
        Err(message) => Hosting::Failed {
            output: Box::new(output),
            message,
        },
    }
}

/// Regarde la réponse du worker, sans attendre.
///
/// `error` est l'erreur de l'écran : `App` y range celle que le worker a renvoyée
/// (port pris, certificat refusé…). Pendant un démarrage, on la reprend ici pour
/// l'afficher dans le panneau, avec « Réessayer ».
pub fn poll(hosting: &mut Hosting, shared: &Shared, error: &mut Option<String>) -> Outcome {
    let Hosting::Starting { output, port } = hosting else {
        return Outcome::Nothing;
    };
    if shared.status.is_some() {
        *hosting = Hosting::Idle;
        return Outcome::Started;
    }
    if shared.starting {
        return Outcome::Nothing;
    }
    if let Some(reason) = error.take() {
        // `std::mem::take` sur la `Box` n'existe pas (pas de valeur par défaut) :
        // on reconstruit l'état à partir d'une copie de la sortie.
        let output = output.clone();
        *hosting = Hosting::Failed {
            output,
            message: start_failed(&readable_reason(&reason, *port)),
        };
    }
    Outcome::Nothing
}

/// Dessine le panneau (rien si `Idle`). Rend vrai si l'hôte clique « Réessayer ».
///
/// `poll` est appelé à part (`create::background`, à chaque image, avant le dessin) :
/// sinon l'erreur du worker s'afficherait d'abord dans un bandeau, ou sur un autre écran.
pub fn show(ui: &mut Ui, hosting: &mut Hosting) -> bool {
    let mut retry = false;
    let mut close = false;
    match hosting {
        Hosting::Idle => return false,
        Hosting::Starting { output, port } => {
            widgets::panel(ui, "Hébergement de la partie", |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!(
                        "Démarrage du serveur sur le port {port} (partie {})…",
                        output.seed_name
                    ));
                });
            });
            // Le worker réveille l'écran à chaque seconde ; ceci rend la bascule plus vive.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        Hosting::Failed { output, message } => {
            widgets::panel(ui, "Hébergement de la partie", |ui| {
                ui.label(RichText::new(message.as_str()).color(theme::RED_TEXT));
                ui.label(
                    widgets::dim(format!(
                        "La partie {} est gardée : corrige les réglages du serveur (port…) \
                         puis réessayez, sans régénérer.",
                        output.seed_name
                    ))
                    .size(13.0),
                );
                // Le chemin de la seed est déjà dans le panneau « Partie générée » au-dessus.
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, "Réessayer", true) {
                        retry = true;
                    }
                    close = widgets::ghost_button(ui, "Fermer");
                });
            });
        }
    }
    ui.add_space(12.0);
    if close {
        *hosting = Hosting::Idle;
    }
    retry
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::{Encryption, SeedSummary};
    use std::path::PathBuf;

    fn output() -> GenerateOutput {
        GenerateOutput {
            archipelago_file: PathBuf::from("generations").join("AP_1.archipelago"),
            zip_file: PathBuf::from("generations").join("AP_1.zip"),
            seed_name: "1234".into(),
            players: Vec::new(),
            summary: SeedSummary {
                seed_name: "1234".into(),
                generator_version: "0.6.8".into(),
                slots: Vec::new(),
                players: 1,
                has_password: false,
                release_mode: "auto".into(),
                collect_mode: "auto".into(),
                hint_cost: 10,
            },
        }
    }

    fn options() -> PartyHostOptions {
        PartyHostOptions {
            port: 38281,
            password: Some("secret".into()),
            encryption: Encryption::Plain,
            public_host: "exemple.fr".into(),
            disable_save: false,
            webhook: None,
        }
    }

    #[test]
    fn reglages_avec_la_seed_generee() {
        let host = host_options(&output(), options(), false).unwrap();
        assert_eq!(host.seed, output().archipelago_file);
        assert_eq!(host.port, 38281);
        assert_eq!(host.password.as_deref(), Some("secret"));
        assert_eq!(host.public_host, "exemple.fr");
        assert_eq!(host.encryption, Encryption::Plain);
    }

    #[test]
    fn refuse_si_une_partie_tourne() {
        assert_eq!(
            host_options(&output(), options(), true).unwrap_err(),
            ALREADY_HOSTING
        );
    }

    #[test]
    fn adresse_vide() {
        let mut options = options();
        options.public_host = " ".into();
        assert_eq!(
            host_options(&output(), options, false).unwrap_err(),
            "Indique l'adresse que les autres joueurs utiliseront."
        );
    }

    #[test]
    fn message_de_demarrage_rate() {
        assert_eq!(
            start_failed("port 38281 déjà utilisé."),
            "Impossible de démarrer le serveur : port 38281 déjà utilisé."
        );
        assert_eq!(
            start_failed("certificat refusé"),
            "Impossible de démarrer le serveur : certificat refusé."
        );
    }

    #[test]
    fn port_deja_utilise_lisible() {
        let windows = "serveur : erreur réseau : Une seule utilisation de chaque adresse de \
                       socket (protocole/adresse réseau/port) est habituellement autorisée. \
                       (os error 10048)";
        assert_eq!(
            start_failed(&readable_reason(windows, 38296)),
            "Impossible de démarrer le serveur : le port 38296 est déjà utilisé par un autre \
             programme, choisis-en un autre."
        );
        let linux = "serveur : erreur réseau : Address already in use (os error 98)";
        assert!(readable_reason(linux, 1).contains("le port 1 est déjà utilisé"));
        // Une autre erreur reste telle quelle.
        assert_eq!(readable_reason("certificat refusé", 1), "certificat refusé");
    }

    fn starting() -> Hosting {
        Hosting::Starting {
            output: Box::new(output()),
            port: 38281,
        }
    }

    #[test]
    fn attente_puis_echec_garde_la_seed() {
        let mut hosting = starting();
        let mut shared = Shared {
            starting: true,
            ..Shared::default()
        };
        let mut error = None;
        assert_eq!(poll(&mut hosting, &shared, &mut error), Outcome::Nothing);
        assert!(hosting.is_busy());

        shared.starting = false;
        error = Some("port déjà utilisé".into());
        assert_eq!(poll(&mut hosting, &shared, &mut error), Outcome::Nothing);
        // L'erreur passe de l'écran au panneau, avec la seed pour « Réessayer ».
        assert_eq!(error, None);
        match &hosting {
            Hosting::Failed { message, .. } => assert_eq!(
                message,
                "Impossible de démarrer le serveur : port déjà utilisé."
            ),
            _ => panic!("le démarrage devait avoir échoué"),
        }
        assert_eq!(
            hosting.failed_output().map(|o| o.seed_name.as_str()),
            Some("1234")
        );
    }

    /// Juste après l'envoi, le worker n'a pas encore commencé : on attend sans conclure.
    #[test]
    fn rien_avant_la_reponse_du_worker() {
        let mut hosting = starting();
        let mut error = None;
        assert_eq!(
            poll(&mut hosting, &Shared::default(), &mut error),
            Outcome::Nothing
        );
        assert!(hosting.is_busy());
    }
}
