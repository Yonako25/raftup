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
    pins, runtime_status, EngineReport, EngineUsed, ExportBackend, GenerateOutput, GenerateRequest,
    GeneratorBackend, PythonBackend, RuntimeStatus, RustBackend,
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
        /// Le moteur qui a vraiment généré, et la raison d'un repli.
        report: EngineReport,
    },
    Failed {
        failure: Failure,
        log: VecDeque<String>,
        /// Absent si la génération n'a pas pu démarrer.
        report: Option<EngineReport>,
    },
}

/// Ce que le thread renvoie à la fin : le résultat et le rapport du moteur.
type Finished = (Result<GenerateOutput, Failure>, Option<EngineReport>);

/// Qui génère la partie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Engine {
    /// Le générateur officiel d'Archipelago, dans le Python embarqué.
    #[default]
    Python,
    /// Le moteur Rust de Raftup (ADR 0027), pour les mondes portés.
    Rust,
    /// Archipelago construit les mondes et écrit la seed, Raftup place les objets
    /// (ADR 0028) ; repli sur Archipelago seul si un monde ne s'y prête pas.
    Export,
}

/// Une génération qui tourne dans un thread.
pub struct Running {
    started: Instant,
    engine: Engine,
    /// Partagé avec le thread : l'annuler ici, il le voit là-bas.
    cancel: CancelToken,
    log: VecDeque<String>,
    /// Lignes de stderr du générateur, envoyées par `py-bridge`.
    log_receiver: Receiver<String>,
    /// Le résultat, un seul message à la fin.
    result_receiver: Receiver<Finished>,
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
/// Avec Python, le runtime est relu sur le disque (petit fichier, appel rapide) : on vient
/// de vérifier qu'il est prêt, mais il faut son chemin exact pour le `PythonBackend`.
/// `note` est une première ligne de journal (par exemple : pourquoi le moteur Rust n'a pas
/// pu servir). `time_balance` : équilibrage par temps de jeu (ADR 0030), pris en compte par
/// les moteurs Rust et export seulement (Archipelago seul ne le connaît pas).
pub fn start(
    ctx: &egui::Context,
    request: GenerateRequest,
    engine: Engine,
    note: Option<String>,
    time_balance: bool,
) -> Generation {
    let fail = |message: String| Generation::Failed {
        failure: Failure {
            message,
            detail: None,
        },
        log: VecDeque::new(),
        report: None,
    };
    let paths = match load_paths() {
        Ok(paths) => paths,
        Err(e) => return fail(format!("Dossier de données de Raftup introuvable : {e}.")),
    };
    // `Box<dyn …>` : une boîte qui contient l'un ou l'autre moteur ; `Send` permet de
    // l'envoyer dans le thread de génération.
    let mut backend: Box<dyn GeneratorBackend + Send> = match engine {
        Engine::Rust => Box::new(RustBackend { time_balance }),
        Engine::Python | Engine::Export => {
            let pins = match pins() {
                Ok(pins) => pins,
                Err(e) => return fail(format!("Le générateur est mal configuré : {e}.")),
            };
            match runtime_status(&paths, &pins) {
                RuntimeStatus::Ready(runtime) if engine == Engine::Export => {
                    let mut backend = ExportBackend::new(runtime);
                    backend.time_balance = time_balance;
                    Box::new(backend)
                }
                RuntimeStatus::Ready(runtime) => Box::new(PythonBackend::new(runtime)),
                _ => return fail(
                    "Le générateur n'est plus installé. Cliquez de nouveau sur « Générer la partie »."
                        .into(),
                ),
            }
        }
    };

    let (log_sender, log_receiver) = mpsc::channel();
    let (result_sender, result_receiver) = mpsc::channel();
    let cancel = CancelToken::new();
    // Copies possédées par le thread (`move`) : l'écran garde les siennes.
    let thread_cancel = cancel.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let result = generate_blocking(
            &paths,
            backend.as_mut(),
            &request,
            &thread_cancel,
            log_sender,
        )
        .map_err(|e| failure(&e));
        // Le moteur sait, après coup, qui a vraiment généré (repli compris).
        let report = backend.report();
        // L'écran a pu disparaître (fenêtre fermée) : un envoi raté n'est pas grave.
        let _ = result_sender.send((result, Some(report)));
        ctx.request_repaint();
    });
    let mut log = VecDeque::new();
    if let Some(note) = note {
        log.push_back(note);
    }
    Generation::Running(Running {
        started: Instant::now(),
        engine,
        cancel,
        log,
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
///
/// `pub(super)` : appelé par `create::background` à chaque image, et par le test réel.
pub(super) fn poll(generation: &mut Generation) {
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
        Err(TryRecvError::Disconnected) => (
            Err(Failure {
                message: "La génération s'est interrompue sans réponse.".into(),
                detail: None,
            }),
            None,
        ),
    };
    // Lignes envoyées entre-temps (le thread a fini, le canal ne grossira plus).
    while let Ok(line) = task.log_receiver.try_recv() {
        push_log(&mut task.log, line);
    }
    // `std::mem::take` : on prend le journal en laissant un journal vide à sa place,
    // parce qu'on va remplacer `*generation` et qu'on ne peut pas garder d'emprunt dessus.
    let log = std::mem::take(&mut task.log);
    let engine = task.engine;
    let (result, report) = finished;
    *generation = match result {
        Ok(output) => Generation::Done {
            output: Box::new(output),
            log,
            // Pas de rapport (ne devrait pas arriver) : on se fie au moteur choisi.
            report: report.unwrap_or_else(|| EngineReport::plain(chosen(engine))),
        },
        Err(failure) => Generation::Failed {
            failure,
            log,
            report,
        },
    };
}

/// Le moteur prévu, au sens du rapport de fin.
fn chosen(engine: Engine) -> EngineUsed {
    match engine {
        Engine::Python => EngineUsed::Python,
        Engine::Rust => EngineUsed::Rust,
        Engine::Export => EngineUsed::Export,
    }
}

/// Phrase qui dit quel moteur a vraiment généré la partie.
fn engine_line(report: &EngineReport) -> &'static str {
    match report.engine {
        EngineUsed::Rust => "Moteur : Raftup seul (moteur Rust, sans Python).",
        EngineUsed::Export => {
            "Moteur : Archipelago a construit les mondes, Raftup a placé les objets."
        }
        EngineUsed::Python => "Moteur : Archipelago seul.",
    }
}

/// Phrase qui explique un repli sur Archipelago seul, s'il y en a eu un.
fn fallback_text(report: &EngineReport) -> Option<String> {
    report.fallback.as_ref().map(|reason| {
        format!("Raftup n'a pas pu placer les objets : {reason}. Archipelago a donc généré seul.")
    })
}

/// Pendant la génération : la raison du repli dès qu'elle apparaît dans le journal.
fn live_fallback(log: &VecDeque<String>) -> Option<&str> {
    log.iter()
        .find_map(|line| line.strip_prefix("Placement par Raftup impossible ("))
        .and_then(|rest| rest.rsplit_once(") : ").map(|(reason, _)| reason))
}

/// Dessine le moteur utilisé et la raison d'un repli (panneaux de fin).
fn report_view(ui: &mut Ui, report: &EngineReport) {
    ui.label(RichText::new(engine_line(report)).size(13.0));
    if let Some(text) = fallback_text(report) {
        ui.label(RichText::new(text).color(theme::AMBER).size(13.0));
    }
}

/// Dessine le panneau de la génération (rien si `Idle`).
///
/// Renvoie vrai si l'hôte clique « Réessayer » : `create` relance alors la
/// vérification du formulaire puis la génération.
pub fn show(ui: &mut Ui, generation: &mut Generation) -> bool {
    // Pas de `poll` ici : `create::background` le fait à chaque image, écran affiché ou non.
    let mut retry = false;
    let mut close = false;
    match generation {
        Generation::Idle => return false,
        Generation::Running(task) => {
            widgets::panel(ui, "Génération de la partie", |ui| {
                let who = match task.engine {
                    Engine::Python => "Archipelago génère la partie…",
                    Engine::Rust => "Raftup génère la partie (moteur Rust)…",
                    Engine::Export => "Archipelago construit les mondes, Raftup place les objets…",
                };
                ui.label(format!("{who} {}", elapsed(task.started.elapsed())));
                if let Some(reason) = live_fallback(&task.log) {
                    ui.label(
                        RichText::new(format!(
                            "Raftup ne peut pas placer les objets : {reason}. Archipelago génère seul."
                        ))
                        .color(theme::AMBER)
                        .size(13.0),
                    );
                }
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
        Generation::Done {
            output,
            log,
            report,
        } => {
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
                ui.add_space(4.0);
                report_view(ui, report);
                ui.add_space(6.0);
                egui::CollapsingHeader::new("Voir le journal")
                    .id_salt("journal-generation")
                    .show(ui, |ui| log_view(ui, log));
            });
        }
        Generation::Failed {
            failure,
            log,
            report,
        } => {
            widgets::panel(ui, "Génération de la partie", |ui| {
                // Après un repli, l'échec vient d'Archipelago seul : on dit d'abord
                // pourquoi Raftup n'a pas placé les objets (le message d'Archipelago peut
                // être très long).
                if let Some(text) = report.as_ref().and_then(fallback_text) {
                    ui.label(RichText::new(text).color(theme::AMBER).size(13.0));
                }
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
            kind,
            message,
            traceback,
        } => {
            let message = match missing_rom(kind, message) {
                Some(file) => format!(
                    "Un des jeux demande votre copie de sa ROM ({file}), introuvable. \
                     Raftup ne fournit aucune ROM : ajoutez votre copie, ou retirez ce joueur."
                ),
                None => format!("Archipelago n'a pas pu générer la partie : {message}."),
            };
            return Failure {
                message,
                detail: Some(traceback.clone()).filter(|t| !t.trim().is_empty()),
            };
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

/// Extensions des fichiers de jeu (ROM, image disque) que demandent certains mondes.
const ROM_EXTENSIONS: [&str; 16] = [
    "sfc", "smc", "gba", "gb", "gbc", "nes", "z64", "n64", "v64", "iso", "bin", "cue", "sms", "gg",
    "md", "pce",
];

/// Si le générateur a échoué faute de ROM, renvoie le nom du fichier attendu.
///
/// Archipelago lève alors un `FileNotFoundError` sous deux formes :
/// « <chemin> » seul (ex. `worlds/soe`), ou « <chemin> does not exist, but
/// XSettings.rom_file is required » (`settings.py`, sans boîte de dialogue).
fn missing_rom(kind: &str, message: &str) -> Option<String> {
    if kind != "FileNotFoundError" {
        return None;
    }
    // Le worker préfixe le message : « generation echouee : FileNotFoundError: … ».
    let text = match message.split_once("FileNotFoundError: ") {
        Some((_, rest)) => rest,
        None => message,
    };
    let path = match text.split_once(" does not exist, but ") {
        Some((path, _)) => path,
        None => text,
    }
    .trim();
    // `Path` sait couper sur `/` et `\` sous Windows ; on garde le seul nom du fichier.
    let file = std::path::Path::new(path).file_name()?.to_string_lossy();
    let extension = file.rsplit_once('.')?.1.to_ascii_lowercase();
    let is_rom = ROM_EXTENSIONS.contains(&extension.as_str()) || text.contains("rom_file");
    is_rom.then(|| file.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn rom_manquante_reconnue() {
        // Forme de `settings.py` (nom relatif, sans boîte de dialogue).
        let message = "generation echouee : FileNotFoundError: Secret of Evermore (USA).sfc \
                       does not exist, but SoESettings.rom_file is required";
        assert_eq!(
            missing_rom("FileNotFoundError", message).as_deref(),
            Some("Secret of Evermore (USA).sfc")
        );
        // Forme de `worlds/soe` : le chemin complet seul.
        let message = r"generation echouee : FileNotFoundError: C:\AP\Secret of Evermore (USA).sfc";
        assert_eq!(
            missing_rom("FileNotFoundError", message).as_deref(),
            Some("Secret of Evermore (USA).sfc")
        );
        // Un autre fichier absent n'est pas une ROM.
        let message = "generation echouee : FileNotFoundError: data/options.yaml";
        assert_eq!(missing_rom("FileNotFoundError", message), None);
        // Autre type d'erreur : pas concerné.
        assert_eq!(missing_rom("FillError", "jeu.sfc"), None);

        let error = GenerationError::GeneratorFailed {
            kind: "FileNotFoundError".into(),
            message: "generation echouee : FileNotFoundError: Jeu.SFC".into(),
            traceback: "Traceback …".into(),
        };
        let f = failure(&error);
        assert!(f.message.contains("ROM (Jeu.SFC)"), "{}", f.message);
        assert!(f.message.contains("Raftup ne fournit aucune ROM"));
        assert_eq!(f.detail.as_deref(), Some("Traceback …"));
    }

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
            engine: Engine::Python,
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
            .send((
                Err(Failure {
                    message: "Génération annulée.".into(),
                    detail: None,
                }),
                None,
            ))
            .unwrap();
        poll(&mut generation);
        match generation {
            Generation::Failed { failure, log, .. } => {
                assert_eq!(failure.message, "Génération annulée.");
                assert_eq!(log, ["ligne 1", "ligne 2"]);
            }
            _ => panic!("la génération devait être finie"),
        }
    }

    #[test]
    fn moteur_affiche_a_la_fin() {
        let rust = EngineReport::plain(EngineUsed::Rust);
        assert!(engine_line(&rust).contains("Raftup seul"));
        assert_eq!(fallback_text(&rust), None);
        let export = EngineReport::plain(EngineUsed::Export);
        assert!(engine_line(&export).contains("Raftup a placé les objets"));
        let repli = EngineReport {
            engine: EngineUsed::Python,
            fallback: Some("règle « x » non traduite".into()),
        };
        assert_eq!(engine_line(&repli), "Moteur : Archipelago seul.");
        assert_eq!(
            fallback_text(&repli).as_deref(),
            Some(
                "Raftup n'a pas pu placer les objets : règle « x » non traduite. \
                 Archipelago a donc généré seul."
            )
        );
    }

    /// La raison du repli est lue dans la ligne du journal écrite par `launcher-core`.
    #[test]
    fn repli_vu_pendant_la_generation() {
        let mut log = VecDeque::new();
        log.push_back("Mondes exportés : 3 règles Python traduites, 1 non traduites".into());
        assert_eq!(live_fallback(&log), None);
        log.push_back(launcher_core::generation::fallback_line(
            "lieu (a) : règle opaque",
        ));
        assert_eq!(live_fallback(&log), Some("lieu (a) : règle opaque"));
    }

    /// Le rapport envoyé par le thread arrive dans l'état « Partie générée » ou « échec ».
    #[test]
    fn rapport_garde_apres_echec() {
        let (_log_sender, log_receiver) = mpsc::channel::<String>();
        let (result_sender, result_receiver) = mpsc::channel();
        let mut generation = Generation::Running(Running {
            started: Instant::now(),
            engine: Engine::Export,
            cancel: CancelToken::new(),
            log: VecDeque::new(),
            log_receiver,
            result_receiver,
        });
        let report = EngineReport {
            engine: EngineUsed::Python,
            fallback: Some("placement impossible".into()),
        };
        result_sender
            .send((
                Err(Failure {
                    message: "échec".into(),
                    detail: None,
                }),
                Some(report.clone()),
            ))
            .unwrap();
        poll(&mut generation);
        match generation {
            Generation::Failed {
                report: Some(got), ..
            } => assert_eq!(got, report),
            _ => panic!("échec avec rapport attendu"),
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
            engine: Engine::Python,
            cancel: CancelToken::new(),
            log: VecDeque::new(),
            log_receiver,
            result_receiver,
        });
        poll(&mut generation);
        assert!(matches!(generation, Generation::Failed { .. }));
    }
}
