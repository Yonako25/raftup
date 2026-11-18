//! Écran « Créer une partie », étape 2 : installer le générateur s'il manque.
//!
//! Le générateur officiel d'Archipelago a besoin d'un Python autonome, du code
//! d'Archipelago et de quelques bibliothèques (ADR 0017), environ 72 Mo en tout.
//! Rien n'est téléchargé sans l'accord de l'hôte : on montre d'abord les sources et
//! la taille, puis on installe dans un thread à part, avec la progression et un
//! bouton « Annuler » (`docs/contrat-ui-generation.md`, état 2).

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use egui::{RichText, Ui};
use launcher_core::generation::{
    ensure_runtime_cancellable, pins, runtime_status, GeneratorPins, RuntimeStatus, SetupStep,
};
use launcher_core::{CancelToken, Downloader, GenerationError, HttpFetcher};

use super::library::{load_paths, megabytes};
use crate::theme;
use crate::widgets;

/// On n'envoie la progression à l'écran qu'à chaque changement d'étape ou tous les
/// 256 Kio : assez pour une barre fluide, sans noyer le canal de messages.
const PROGRESS_STEP: u64 = 256 * 1024;

/// Message du thread d'installation vers l'écran.
enum SetupMessage {
    /// Étape en cours et octets reçus pour cette étape.
    Progress(SetupStep, u64),
    /// Fin : rien, ou le message d'erreur déjà écrit pour l'hôte.
    Done(Result<(), String>),
}

/// Où en est le générateur.
#[derive(Default)]
pub enum Setup {
    /// Pas encore regardé (ou l'hôte a refusé de l'installer).
    #[default]
    Unknown,
    /// Prêt : la génération peut commencer.
    Ready,
    /// Absent ou abîmé : on attend l'accord de l'hôte avant de télécharger.
    NeedsConsent {
        pins: GeneratorPins,
        /// Raison si une installation existe mais est inutilisable.
        broken: Option<String>,
    },
    /// Installation en cours dans un thread.
    Installing(Installing),
    /// Installation finie à l'instant (message de réussite à montrer).
    Installed,
    /// L'installation ou la vérification a échoué ; `pins` permet « Réessayer ».
    Failed {
        message: String,
        pins: Option<GeneratorPins>,
    },
}

/// L'état d'une installation qui tourne.
pub struct Installing {
    pins: GeneratorPins,
    step: Option<SetupStep>,
    bytes: u64,
    /// `CancelToken` est partagé avec le thread : l'annuler ici, il le voit là-bas.
    cancel: CancelToken,
    receiver: Receiver<SetupMessage>,
}

impl Setup {
    /// Vrai pendant l'installation : le bouton « Générer » est alors grisé.
    pub fn is_busy(&self) -> bool {
        matches!(self, Setup::Installing(_))
    }
}

/// Regarde le disque (sans rien télécharger) et dit où en est le générateur.
///
/// Lecture d'un petit fichier `runtime.json` et de quelques `exists()` : assez rapide
/// pour être faite dans le fil de l'interface, au clic.
pub fn check() -> Setup {
    let pins = match pins() {
        Ok(pins) => pins,
        Err(e) => {
            return Setup::Failed {
                message: format!("Le générateur est mal configuré : {e}."),
                pins: None,
            }
        }
    };
    let paths = match load_paths() {
        Ok(paths) => paths,
        Err(e) => {
            return Setup::Failed {
                message: format!("Dossier de données de Raftup introuvable : {e}."),
                pins: None,
            }
        }
    };
    match runtime_status(&paths, &pins) {
        RuntimeStatus::Ready(_) => Setup::Ready,
        RuntimeStatus::Missing { .. } => Setup::NeedsConsent { pins, broken: None },
        RuntimeStatus::Broken(reason) => Setup::NeedsConsent {
            pins,
            broken: Some(reason),
        },
    }
}

/// Dessine le panneau du générateur (rien si `Unknown` ou `Ready`).
pub fn show(ui: &mut Ui, setup: &mut Setup) {
    poll(setup);
    // On calcule le prochain état pendant le dessin, puis on le pose après :
    // on ne peut pas remplacer `*setup` pendant qu'on l'emprunte pour le lire.
    let mut next = None;
    match setup {
        Setup::Unknown | Setup::Ready => return,
        Setup::NeedsConsent { pins, broken } => {
            consent_panel(ui, pins, broken.as_deref(), &mut next);
        }
        Setup::Installing(task) => {
            progress_panel(ui, task);
            // Le thread réveille l'écran à chaque message ; ceci couvre le reste.
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        Setup::Installed => {
            widgets::panel(ui, "Générateur Archipelago", |ui| {
                ui.label(
                    RichText::new(
                        "Générateur installé et vérifié. Cliquez de nouveau sur « Générer la \
                         partie ».",
                    )
                    .color(theme::GREEN)
                    .size(13.0),
                );
            });
        }
        Setup::Failed { message, pins } => {
            widgets::panel(ui, "Générateur Archipelago", |ui| {
                ui.label(RichText::new(message.as_str()).color(theme::RED_TEXT));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(pins) = pins {
                        if widgets::primary_button(ui, "Réessayer", true) {
                            next = Some(start(ui.ctx(), pins.clone()));
                        }
                    }
                    if widgets::ghost_button(ui, "Fermer") {
                        next = Some(Setup::Unknown);
                    }
                });
            });
        }
    }
    ui.add_space(12.0);
    if let Some(next) = next {
        *setup = next;
    }
}

/// Le consentement : sources, taille, vérification par hash, puis les deux boutons.
fn consent_panel(
    ui: &mut Ui,
    pins: &GeneratorPins,
    broken: Option<&str>,
    next: &mut Option<Setup>,
) {
    widgets::panel(ui, "Installer le générateur Archipelago", |ui| {
        if let Some(reason) = broken {
            ui.label(
                RichText::new(format!(
                    "L'installation actuelle est inutilisable ({reason}) : il faut la refaire."
                ))
                .color(theme::AMBER)
                .size(13.0),
            );
            ui.add_space(4.0);
        }
        ui.label(
            "Pour générer une partie, Raftup utilise le générateur officiel d'Archipelago. \
             Il n'est pas encore sur ce PC. Raftup va télécharger :",
        );
        ui.add_space(4.0);
        for line in sources(pins) {
            ui.label(format!("•  {line}"));
        }
        ui.add_space(4.0);
        ui.label(
            widgets::dim(format!(
                "Total : {}. Chaque fichier est vérifié par son empreinte (sha256) avant \
                 usage. Rien n'est téléchargé sans votre accord.",
                megabytes(pins.download_bytes())
            ))
            .size(13.0),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if widgets::primary_button(ui, &install_label(pins.download_bytes()), true) {
                *next = Some(start(ui.ctx(), pins.clone()));
            }
            if widgets::ghost_button(ui, "Pas maintenant") {
                *next = Some(Setup::Unknown);
            }
        });
    });
}

fn progress_panel(ui: &mut Ui, task: &Installing) {
    widgets::panel(ui, "Installation du générateur", |ui| {
        let text = match &task.step {
            None => "Préparation…".to_string(),
            Some(SetupStep::Finalize) => "Mise en place des fichiers…".to_string(),
            Some(step) => format!(
                "{} : {} sur {}",
                step_label(step),
                megabytes(task.bytes),
                megabytes(step_size(&task.pins, step))
            ),
        };
        ui.label(text);
        ui.add_space(6.0);
        let fraction = overall_fraction(&task.pins, task.step.as_ref(), task.bytes);
        widgets::bar(ui, ui.available_width(), fraction, theme::ACCENT);
        ui.add_space(10.0);
        if task.cancel.is_cancelled() {
            ui.label(widgets::dim("Annulation…").size(13.0));
        } else if matches!(task.step, Some(SetupStep::Finalize)) {
            // L'extraction va au bout (ADR 0017) : on ne propose plus d'annuler.
            ui.label(widgets::dim("Presque fini, ne fermez pas Raftup.").size(13.0));
        } else if widgets::ghost_button(ui, "Annuler") {
            task.cancel.cancel();
        }
    });
}

/// Lance l'installation dans un thread et rend l'état « en cours ».
fn start(ctx: &egui::Context, pins: GeneratorPins) -> Setup {
    let paths = match load_paths() {
        Ok(paths) => paths,
        Err(e) => {
            return Setup::Failed {
                message: format!("Dossier de données de Raftup introuvable : {e}."),
                pins: Some(pins),
            }
        }
    };
    let (sender, receiver) = mpsc::channel();
    let cancel = CancelToken::new();
    // Copies possédées par le thread (`move`) : l'écran garde les siennes.
    let thread_cancel = cancel.clone();
    let thread_pins = pins.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let downloader = Downloader::new(paths.downloads_dir(), Box::new(HttpFetcher::new()));
        let mut last: Option<(SetupStep, u64)> = None;
        let result = ensure_runtime_cancellable(
            &paths,
            &thread_pins,
            &downloader,
            &mut |step, bytes| {
                let changed = match &last {
                    None => true,
                    Some((s, b)) => *s != step || bytes >= b + PROGRESS_STEP,
                };
                if changed {
                    last = Some((step.clone(), bytes));
                    let _ = sender.send(SetupMessage::Progress(step, bytes));
                    ctx.request_repaint();
                }
            },
            &thread_cancel,
        );
        // L'écran a pu disparaître (fenêtre fermée) : un envoi raté n'est pas grave.
        let _ = sender.send(SetupMessage::Done(
            result.map(|_| ()).map_err(|e| error_message(&e)),
        ));
        ctx.request_repaint();
    });
    Setup::Installing(Installing {
        pins,
        step: None,
        bytes: 0,
        cancel,
        receiver,
    })
}

/// Lit les messages du thread, sans jamais attendre.
fn poll(setup: &mut Setup) {
    let Setup::Installing(task) = setup else {
        return;
    };
    let mut finished = None;
    // `loop` : on vide tous les messages arrivés depuis la dernière image.
    loop {
        match task.receiver.try_recv() {
            Ok(SetupMessage::Progress(step, bytes)) => {
                task.step = Some(step);
                task.bytes = bytes;
            }
            Ok(SetupMessage::Done(result)) => {
                finished = Some(result);
                break;
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                finished = Some(Err("L'installation s'est interrompue sans réponse.".into()));
                break;
            }
        }
    }
    if let Some(result) = finished {
        *setup = match result {
            Ok(()) => Setup::Installed,
            Err(message) => Setup::Failed {
                message,
                pins: Some(task.pins.clone()),
            },
        };
    }
}

/// Texte du bouton, avec la taille arrondie au mégaoctet : « Installer le générateur (≈ 72 Mo) ».
fn install_label(bytes: u64) -> String {
    let mo = (bytes as f64 / 1_000_000.0).round();
    format!("Installer le générateur (≈ {mo} Mo)")
}

/// Une ligne par source, avec le site d'où vient le fichier.
fn sources(pins: &GeneratorPins) -> Vec<String> {
    let mut lines = vec![
        format!(
            "Python {} autonome, depuis {} ({})",
            pins.python.version,
            host(&pins.python.url),
            megabytes(pins.python.size)
        ),
        format!(
            "Archipelago {}, depuis {} ({})",
            pins.archipelago.version,
            host(&pins.archipelago.url),
            megabytes(pins.archipelago.size)
        ),
    ];
    if !pins.wheels.is_empty() {
        // `BTreeSet` : les sites dans un ordre stable, sans doublon.
        let hosts: std::collections::BTreeSet<&str> =
            pins.wheels.iter().map(|w| host(&w.url)).collect();
        let names: Vec<&str> = pins.wheels.iter().map(|w| w.name.as_str()).collect();
        let size: u64 = pins.wheels.iter().map(|w| w.size).sum();
        let count = pins.wheels.len();
        let plural = if count > 1 { "s" } else { "" };
        lines.push(format!(
            "{count} bibliothèque{plural} Python ({}), depuis {} ({})",
            names.join(", "),
            hosts.into_iter().collect::<Vec<_>>().join(", "),
            megabytes(size)
        ));
    }
    lines
}

/// Le nom de domaine d'une adresse : `https://github.com/a/b` → `github.com`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split('/').next().unwrap_or(rest)
}

fn step_label(step: &SetupStep) -> String {
    match step {
        SetupStep::Python => "Python".into(),
        SetupStep::Archipelago => "Archipelago".into(),
        SetupStep::Wheel(name) => format!("Bibliothèque {name}"),
        SetupStep::Finalize => "Mise en place".into(),
    }
}

/// Taille attendue du fichier de cette étape (0 pour la mise en place).
fn step_size(pins: &GeneratorPins, step: &SetupStep) -> u64 {
    match step {
        SetupStep::Python => pins.python.size,
        SetupStep::Archipelago => pins.archipelago.size,
        SetupStep::Wheel(name) => pins
            .wheels
            .iter()
            .find(|w| &w.name == name)
            .map_or(0, |w| w.size),
        SetupStep::Finalize => 0,
    }
}

/// Avancement global entre 0 et 1 : les étapes finies, plus la part de l'étape en cours.
/// Les étapes suivent l'ordre d'`ensure_runtime` : Python, Archipelago, chaque wheel.
fn overall_fraction(pins: &GeneratorPins, step: Option<&SetupStep>, bytes: u64) -> f32 {
    let total = pins.download_bytes();
    let Some(step) = step else { return 0.0 };
    if total == 0 || *step == SetupStep::Finalize {
        return 1.0;
    }
    let mut order = vec![SetupStep::Python, SetupStep::Archipelago];
    order.extend(pins.wheels.iter().map(|w| SetupStep::Wheel(w.name.clone())));
    let done: u64 = order
        .iter()
        .take_while(|s| *s != step)
        .map(|s| step_size(pins, s))
        .sum();
    // La taille réelle peut dépasser un peu la valeur épinglée : on borne l'étape.
    let current = bytes.min(step_size(pins, step));
    ((done + current) as f64 / total as f64).clamp(0.0, 1.0) as f32
}

/// Message en français pour l'hôte (contrat, « Messages d'erreur »).
fn error_message(error: &GenerationError) -> String {
    match error {
        GenerationError::Cancelled => "Installation annulée.".into(),
        GenerationError::Busy => "Une génération est déjà en cours.".into(),
        GenerationError::Download(e) => {
            format!("Le téléchargement a échoué ({e}). Vérifiez votre connexion.")
        }
        other => format!("L'installation du générateur a échoué : {other}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use launcher_core::generation::parse_pins;

    fn test_pins() -> GeneratorPins {
        let hash = "0".repeat(64);
        parse_pins(&format!(
            r#"
format_version = 1
[python]
version = "3.12.0+1"
url = "https://github.com/t/python.tar.gz"
sha256 = "{hash}"
size = 30000000
max_bytes = 40000000
strip_prefix = "python"
max_entries = 100
[archipelago]
version = "0.6.8"
url = "https://github.com/t/ap.zip"
sha256 = "{hash}"
size = 40000000
max_bytes = 50000000
strip_prefix = "ap"
max_entries = 100
[[wheels]]
name = "pyyaml"
version = "6"
filename = "pyyaml-6-py3-none-any.whl"
url = "https://files.pythonhosted.org/pyyaml-6-py3-none-any.whl"
sha256 = "{hash}"
size = 2000000
max_bytes = 3000000
"#
        ))
        .unwrap()
    }

    #[test]
    fn libelle_du_bouton_arrondi() {
        assert_eq!(
            install_label(72_000_000),
            "Installer le générateur (≈ 72 Mo)"
        );
        assert_eq!(
            install_label(71_600_000),
            "Installer le générateur (≈ 72 Mo)"
        );
    }

    #[test]
    fn sources_avec_sites_et_tailles() {
        let lines = sources(&test_pins());
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "Python 3.12.0+1 autonome, depuis github.com (30,0 Mo)"
        );
        assert_eq!(lines[1], "Archipelago 0.6.8, depuis github.com (40,0 Mo)");
        assert_eq!(
            lines[2],
            "1 bibliothèque Python (pyyaml), depuis files.pythonhosted.org (2,0 Mo)"
        );
    }

    #[test]
    fn nom_de_domaine() {
        assert_eq!(host("https://github.com/a/b"), "github.com");
        assert_eq!(host("https://exemple.fr"), "exemple.fr");
    }

    #[test]
    fn avancement_global() {
        let pins = test_pins();
        assert_eq!(overall_fraction(&pins, None, 0), 0.0);
        // Moitié de Python : 15 Mo sur 72.
        let f = overall_fraction(&pins, Some(&SetupStep::Python), 15_000_000);
        assert!((f - 15.0 / 72.0).abs() < 1e-6, "{f}");
        // Début d'Archipelago : Python fini.
        let f = overall_fraction(&pins, Some(&SetupStep::Archipelago), 0);
        assert!((f - 30.0 / 72.0).abs() < 1e-6, "{f}");
        // Wheel en cours, avec plus d'octets que prévu : bornée.
        let f = overall_fraction(&pins, Some(&SetupStep::Wheel("pyyaml".into())), 9_000_000);
        assert!((f - 1.0).abs() < 1e-6, "{f}");
        assert_eq!(overall_fraction(&pins, Some(&SetupStep::Finalize), 0), 1.0);
    }

    #[test]
    fn messages_d_erreur() {
        assert_eq!(
            error_message(&GenerationError::Cancelled),
            "Installation annulée."
        );
        assert_eq!(
            error_message(&GenerationError::Busy),
            "Une génération est déjà en cours."
        );
        let download = GenerationError::Download(launcher_core::download::DownloadError::Http {
            url: "https://github.com/x".into(),
            status: 503,
        });
        let text = error_message(&download);
        assert!(text.starts_with("Le téléchargement a échoué ("), "{text}");
        assert!(text.ends_with("Vérifiez votre connexion."), "{text}");
    }

    #[test]
    fn libelles_des_etapes() {
        assert_eq!(
            step_label(&SetupStep::Wheel("pyyaml".into())),
            "Bibliothèque pyyaml"
        );
        assert_eq!(
            step_size(&test_pins(), &SetupStep::Wheel("absente".into())),
            0
        );
    }
}
