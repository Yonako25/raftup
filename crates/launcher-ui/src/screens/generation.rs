//! Écran « Créer une partie », étape 3 : la génération.
//!
//! Une fois le formulaire vérifié et le générateur installé, on lance
//! `generate_blocking` dans un thread `std` (jamais dans le fil de dessin, contrat
//! « Règles d'appel »). L'écran montre une barre indéterminée, le temps écoulé, le
//! journal du générateur en direct et un bouton « Annuler »
//! (`docs/contrat-ui-generation.md`, état 3). L'hébergement est dans `hosting`.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use launcher_core::generation::{
    pins, runtime_status, GenerateOutput, GenerateRequest, PythonBackend, RuntimeStatus,
};
use launcher_core::party::generate_blocking;
use launcher_core::{CancelToken, GenerationError};

use super::library::load_paths;
use crate::theme;
use crate::widgets;

/// Nombre de lignes de journal gardées en mémoire : les plus anciennes partent.
const LOG_LIMIT: usize = 2000;

/// Un échec, déjà écrit pour l'hôte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    /// Détail technique (traceback Python), montré seulement sur demande.
    pub detail: Option<String>,
}

/// Où en est la génération.
#[derive(Default)]
pub enum Generation {
    /// Rien de lancé.
    #[default]
    Idle,
    Running(Running),
    /// Seed prête, à héberger (étape 4).
    Done {
        output: Box<GenerateOutput>,
        log: VecDeque<String>,
    },
    Failed {
        failure: Failure,
        log: VecDeque<String>,
    },
}

/// Une génération qui tourne dans un thread.
pub struct Running {
    started: Instant,
    /// Partagé avec le thread : l'annuler ici, il le voit là-bas.
    cancel: CancelToken,
    log: VecDeque<String>,
    /// Lignes de stderr du générateur, envoyées par `py-bridge`.
    log_receiver: Receiver<String>,
    /// Le résultat, un seul message à la fin.
    result_receiver: Receiver<Result<GenerateOutput, Failure>>,
}

impl Generation {
    /// Vrai pendant la génération : le bouton « Générer » est alors grisé.
    pub fn is_busy(&self) -> bool {
        matches!(self, Generation::Running(_))
    }

    /// La seed générée, une fois la génération réussie.
    pub fn output(&self) -> Option<&GenerateOutput> {
        match self {
            Generation::Done { output, .. } => Some(output),
            _ => None,
        }
    }
}

/// Lance la génération dans un thread et rend l'état « en cours ».
///
/// Le runtime est relu sur le disque (petit fichier, appel rapide) : on vient de
/// vérifier qu'il est prêt, mais il faut son chemin exact pour le `PythonBackend`.
pub fn start(ctx: &egui::Context, request: GenerateRequest) -> Generation {
    let fail = |message: String| Generation::Failed {
        failure: Failure {
            message,
            detail: None,
        },
        log: VecDeque::new(),
    };
    let paths = match load_paths() {
        Ok(paths) => paths,
        Err(e) => return fail(format!("Dossier de données de Raftup introuvable : {e}.")),
    };
    let pins = match pins() {
        Ok(pins) => pins,
        Err(e) => return fail(format!("Le générateur est mal configuré : {e}.")),
    };
    let runtime =
        match runtime_status(&paths, &pins) {
            RuntimeStatus::Ready(runtime) => runtime,
            _ => return fail(
                "Le générateur n'est plus installé. Cliquez de nouveau sur « Générer la partie »."
                    .into(),
            ),
        };

    let (log_sender, log_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    let cancel = CancelToken::new();
    // Copies possédées par le thread (`move`) : l'écran garde les siennes.
    let thread_cancel = cancel.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut backend = PythonBackend::new(runtime);
        let result = generate_blocking(&paths, &mut backend, &request, &thread_cancel, log_sender)
            .map_err(|e| failure(&e));
        // L'écran a pu disparaître (fenêtre fermée) : un envoi raté n'est pas grave.
        let _ = result_sender.send(result);
        ctx.request_repaint();
    });
    Generation::Running(Running {
        started: Instant::now(),
        cancel,
        log: VecDeque::new(),
        log_receiver,
        result_receiver,
    })
}

/// Ajoute une ligne au journal, en oubliant la plus ancienne au-delà de `LOG_LIMIT`.
fn push_log(log: &mut VecDeque<String>, line: String) {
    if log.len() == LOG_LIMIT {
        log.pop_front();
    }
    log.push_back(line);
}

/// Lit les messages du thread, sans jamais attendre.
fn poll(generation: &mut Generation) {
    let Generation::Running(task) = generation else {
        return;
    };
    // On vide d'abord le journal : les dernières lignes arrivent avant le résultat.
    while let Ok(line) = task.log_receiver.try_recv() {
        push_log(&mut task.log, line);
    }
    let finished = match task.result_receiver.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => Err(Failure {
            message: "La génération s'est interrompue sans réponse.".into(),
            detail: None,
        }),
    };
    // Lignes envoyées entre-temps (le thread a fini, le canal ne grossira plus).
    while let Ok(line) = task.log_receiver.try_recv() {
        push_log(&mut task.log, line);
    }
    // `std::mem::take` : on prend le journal en laissant un journal vide à sa place,
    // parce qu'on va remplacer `*generation` et qu'on ne peut pas garder d'emprunt dessus.
    let log = std::mem::take(&mut task.log);
    *generation = match finished {
        Ok(output) => Generation::Done {
            output: Box::new(output),
            log,
        },
        Err(failure) => Generation::Failed { failure, log },
    };
}

/// Dessine le panneau de la génération (rien si `Idle`).
///
/// Renvoie vrai si l'hôte clique « Réessayer » : `create` relance alors la
/// vérification du formulaire puis la génération.
pub fn show(ui: &mut Ui, generation: &mut Generation) -> bool {
    poll(generation);
    let mut retry = false;
    let mut close = false;
    match generation {
        Generation::Idle => return false,
        Generation::Running(task) => {
            widgets::panel(ui, "Génération de la partie", |ui| {
                ui.label(format!(
                    "Archipelago génère la partie… {}",
                    elapsed(task.started.elapsed())
                ));
                ui.add_space(6.0);
                indeterminate_bar(ui, ui.available_width(), ui.input(|i| i.time));
                ui.add_space(8.0);
                log_view(ui, &task.log);
                ui.add_space(8.0);
                if task.cancel.is_cancelled() {
                    ui.label(widgets::dim("Annulation…").size(13.0));
                } else if widgets::ghost_button(ui, "Annuler") {
                    task.cancel.cancel();
                }
            });
            // La barre bouge : on redessine régulièrement, et les lignes du journal
            // (qui ne réveillent pas l'écran d'elles-mêmes) arrivent ainsi en direct.
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        Generation::Done { output, log } => {
            widgets::panel(ui, "Partie générée", |ui| {
                let count = output.players.len();
                let plural = if count > 1 { "s" } else { "" };
                ui.label(
                    RichText::new(format!(
                        "Seed {} générée pour {count} joueur{plural}.",
                        output.seed_name
                    ))
                    .color(theme::GREEN),
                );
                ui.label(widgets::dim(output.archipelago_file.display().to_string()).size(13.0));
                ui.add_space(6.0);
                egui::CollapsingHeader::new("Voir le journal")
                    .id_salt("journal-generation")
                    .show(ui, |ui| log_view(ui, log));
            });
        }
        Generation::Failed { failure, log } => {
            widgets::panel(ui, "Génération de la partie", |ui| {
                ui.label(RichText::new(failure.message.as_str()).color(theme::RED_TEXT));
                if let Some(detail) = &failure.detail {
                    egui::CollapsingHeader::new("Voir le détail")
                        .id_salt("detail-generation")
                        .show(ui, |ui| {
                            ui.label(widgets::mono(detail.as_str(), 12.0));
                        });
                }
                if !log.is_empty() {
                    egui::CollapsingHeader::new("Voir le journal")
                        .id_salt("journal-generation")
                        .show(ui, |ui| log_view(ui, log));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    retry = widgets::primary_button(ui, "Réessayer", true);
                    close = widgets::ghost_button(ui, "Fermer");
                });
            });
        }
    }
    ui.add_space(12.0);
    if close {
        *generation = Generation::Idle;
    }
    retry
}

/// Journal défilant, collé en bas pour suivre les dernières lignes.
fn log_view(ui: &mut Ui, log: &VecDeque<String>) {
    egui::Frame::new()
        .fill(theme::BORDER.gamma_multiply(0.3))
        .inner_margin(8)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("defilement-journal")
                .max_height(180.0)
                .auto_shrink([false, true])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(widgets::dim("En attente des premières lignes…").size(12.0));
                    }
                    for line in log {
                        ui.label(widgets::mono(line.as_str(), 12.0));
                    }
                });
        });
}

/// Barre sans fin connue : un segment qui glisse de gauche à droite en boucle.
fn indeterminate_bar(ui: &mut Ui, width: f32, time: f64) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, theme::BORDER);
    let (start, end) = segment(time);
    let left = rect.left() + rect.width() * start;
    let right = rect.left() + rect.width() * end;
    let filled = egui::Rect::from_x_y_ranges(left..=right, rect.y_range());
    painter.rect_filled(filled, 0.0, theme::ACCENT);
}

/// Début et fin du segment, entre 0 et 1 : un quart de la largeur, un aller en 1,5 s.
fn segment(time: f64) -> (f32, f32) {
    const PERIOD: f64 = 1.5;
    const LENGTH: f64 = 0.25;
    // Le segment part de -LENGTH (caché à gauche) et sort à droite.
    let phase = (time % PERIOD) / PERIOD;
    let start = phase * (1.0 + LENGTH) - LENGTH;
    let end = start + LENGTH;
    (start.max(0.0) as f32, end.min(1.0) as f32)
}

/// Temps écoulé lisible : « 42 s », « 3 min 05 s ».
fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min {:02} s", seconds / 60, seconds % 60)
    }
}

/// Message en français pour l'hôte (contrat, « Messages d'erreur »).
fn failure(error: &GenerationError) -> Failure {
    let name = |path: &std::path::Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    };
    let message = match error {
        GenerationError::NoPlayers => "Ajoutez au moins un fichier YAML.".into(),
        GenerationError::InvalidPlayerFile { path, reason } => {
            format!("Le fichier {} n'est pas utilisable : {reason}.", name(path))
        }
        GenerationError::GeneratorFailed {
            message, traceback, ..
        } => {
            return Failure {
                message: format!("Archipelago n'a pas pu générer la partie : {message}."),
                detail: Some(traceback.clone()).filter(|t| !t.trim().is_empty()),
            }
        }
        GenerationError::IncompatibleApworld { path, .. } => format!(
            "L'apworld {} demande une autre version d'Archipelago.",
            name(path)
        ),
        GenerationError::Cancelled => "Génération annulée.".into(),
        GenerationError::Busy => "Une génération est déjà en cours.".into(),
        other => format!("La génération a échoué : {other}."),
    };
    Failure {
        message,
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn messages_du_contrat() {
        assert_eq!(
            failure(&GenerationError::Cancelled).message,
            "Génération annulée."
        );
        assert_eq!(
            failure(&GenerationError::Busy).message,
            "Une génération est déjà en cours."
        );
        assert_eq!(
            failure(&GenerationError::NoPlayers).message,
            "Ajoutez au moins un fichier YAML."
        );
        let invalid = GenerationError::InvalidPlayerFile {
            path: PathBuf::from("joueurs").join("meta.yaml"),
            reason: "nom réservé".into(),
        };
        assert_eq!(
            failure(&invalid).message,
            "Le fichier meta.yaml n'est pas utilisable : nom réservé."
        );
        let apworld = GenerationError::IncompatibleApworld {
            path: PathBuf::from("monde.apworld"),
            reason: "trop récent".into(),
        };
        assert_eq!(
            failure(&apworld).message,
            "L'apworld monde.apworld demande une autre version d'Archipelago."
        );
    }

    #[test]
    fn echec_du_generateur_avec_detail() {
        let error = GenerationError::GeneratorFailed {
            kind: "FillError".into(),
            message: "pas assez de lieux".into(),
            traceback: "Traceback (most recent call last): …".into(),
        };
        let f = failure(&error);
        assert_eq!(
            f.message,
            "Archipelago n'a pas pu générer la partie : pas assez de lieux."
        );
        assert_eq!(
            f.detail.as_deref(),
            Some("Traceback (most recent call last): …")
        );
        // Traceback vide : pas de « Voir le détail ».
        let error = GenerationError::GeneratorFailed {
            kind: "X".into(),
            message: "m".into(),
            traceback: "  \n".into(),
        };
        assert_eq!(failure(&error).detail, None);
    }

    #[test]
    fn journal_borne() {
        let mut log = VecDeque::new();
        for i in 0..LOG_LIMIT + 5 {
            push_log(&mut log, i.to_string());
        }
        assert_eq!(log.len(), LOG_LIMIT);
        assert_eq!(log.front().map(String::as_str), Some("5"));
    }

    #[test]
    fn temps_ecoule() {
        assert_eq!(elapsed(Duration::from_secs(42)), "42 s");
        assert_eq!(elapsed(Duration::from_secs(185)), "3 min 05 s");
    }

    #[test]
    fn segment_dans_la_barre() {
        for step in 0..60 {
            let (start, end) = segment(step as f64 * 0.05);
            assert!((0.0..=1.0).contains(&start), "{start}");
            assert!((0.0..=1.0).contains(&end), "{end}");
            assert!(start <= end);
        }
    }

    /// Le thread envoie ses dernières lignes avant le résultat : aucune ne se perd.
    #[test]
    fn poll_garde_les_dernieres_lignes() {
        let (log_sender, log_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        let mut generation = Generation::Running(Running {
            started: Instant::now(),
            cancel: CancelToken::new(),
            log: VecDeque::new(),
            log_receiver,
            result_receiver,
        });
        log_sender.send("ligne 1".to_string()).unwrap();
        poll(&mut generation);
        assert!(generation.is_busy());
        log_sender.send("ligne 2".to_string()).unwrap();
        result_sender
            .send(Err(Failure {
                message: "Génération annulée.".into(),
                detail: None,
            }))
            .unwrap();
        poll(&mut generation);
        match generation {
            Generation::Failed { failure, log } => {
                assert_eq!(failure.message, "Génération annulée.");
                assert_eq!(log, ["ligne 1", "ligne 2"]);
            }
            _ => panic!("la génération devait être finie"),
        }
    }

    /// Thread disparu sans réponse : un échec lisible, pas un écran figé.
    #[test]
    fn thread_disparu() {
        let (_, log_receiver) = mpsc::channel::<String>();
        let (result_sender, result_receiver) = mpsc::channel();
        drop(result_sender);
        let mut generation = Generation::Running(Running {
            started: Instant::now(),
            cancel: CancelToken::new(),
            log: VecDeque::new(),
            log_receiver,
            result_receiver,
        });
        poll(&mut generation);
        assert!(matches!(generation, Generation::Failed { .. }));
    }
}
