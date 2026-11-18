//! Mise à jour du launcher : bannière et fils d'exécution en arrière-plan (F12, ADR 0022).
//!
//! Au démarrage, un thread lit le manifeste ; s'il n'y a pas de réseau ou que la lecture
//! échoue, on ne dit rien (jamais bloquant). Si une version plus récente existe, une
//! bannière propose « Installer » : rien n'est téléchargé ni remplacé sans ce clic.
//! Le téléchargement est vérifié (sha256 + signature minisign) par `launcher_core::update`,
//! puis `apply_update` remplace l'exécutable (l'ancien devient `.old`, nettoyé au
//! démarrage suivant). « Redémarrer » ferme la fenêtre ; `main` relance le nouvel exécutable
//! une fois le verrou d'instance unique relâché (`relaunch_if_requested`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use launcher_core::update::{self, UpdateManifest, UpdateStatus};
use launcher_core::{CancelToken, Downloader, HttpFetcher, Paths};

use crate::{theme, widgets};

/// Où en est la mise à jour.
#[derive(Debug, Clone)]
enum Phase {
    /// Rien à montrer (pas encore de réponse, à jour, ou réseau indisponible).
    Idle,
    /// Une version plus récente existe ; on attend le clic sur « Installer ».
    Available(UpdateManifest),
    /// Téléchargement en cours (octets reçus, plafond annoncé).
    Downloading { received: u64, total: u64 },
    /// Le nouvel exécutable est en place ; il sera utilisé au prochain démarrage.
    Installed,
    /// Échec du téléchargement, de la vérification ou du remplacement.
    Failed(String),
}

/// Chemin de l'exécutable noté AU DÉMARRAGE : sous Windows, `current_exe()` rendrait
/// le chemin `.old` une fois le fichier renommé.
static EXE: OnceLock<PathBuf> = OnceLock::new();
/// Mis à vrai par « Redémarrer » ; lu par `main` quand la fenêtre est fermée.
static RESTART: AtomicBool = AtomicBool::new(false);

/// État partagé entre l'interface et les fils d'exécution.
// `Arc<Mutex<…>>` : plusieurs threads se partagent l'état, un seul à la fois le modifie.
pub struct UpdateState {
    phase: Arc<Mutex<Phase>>,
    started: bool,
}

impl UpdateState {
    pub fn new() -> UpdateState {
        UpdateState {
            phase: Arc::new(Mutex::new(Phase::Idle)),
            started: false,
        }
    }
}

impl Default for UpdateState {
    fn default() -> Self {
        UpdateState::new()
    }
}

/// Un thread qui a paniqué ne doit pas bloquer l'interface : on reprend la valeur telle quelle.
fn lock(phase: &Mutex<Phase>) -> MutexGuard<'_, Phase> {
    phase.lock().unwrap_or_else(|e| e.into_inner())
}

fn set(phase: &Mutex<Phase>, ctx: &egui::Context, value: Phase) {
    *lock(phase) = value;
    ctx.request_repaint();
}

/// Vérification au démarrage : nettoie l'ancien exécutable puis lit le manifeste en arrière-plan.
fn start(state: &mut UpdateState, ctx: &egui::Context) {
    state.started = true;
    if let Ok(exe) = std::env::current_exe() {
        update::cleanup_old(&exe);
        let _ = EXE.set(exe);
    }
    // Pratique en développement : pas de réseau à chaque lancement.
    if std::env::var_os("RAFTUP_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let phase = Arc::clone(&state.phase);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        if let Some(manifest) = check_for_update() {
            set(&phase, &ctx, Phase::Available(manifest));
        }
    });
}

/// `None` dans tous les cas où il n'y a rien à proposer, y compris les erreurs : silence.
fn check_for_update() -> Option<UpdateManifest> {
    let manifest = update::fetch_manifest(&HttpFetcher::new(), update::UPDATE_MANIFEST_URL)
        .map_err(|e| tracing::debug!("vérification de mise à jour ignorée : {e}"))
        .ok()?;
    let current = update::current_version().ok()?;
    match update::check(&manifest, &current) {
        Ok(UpdateStatus::Available(_)) => Some(manifest),
        _ => None,
    }
}

/// Téléchargement, vérification puis remplacement de l'exécutable (dans un thread).
fn install(manifest: UpdateManifest, phase: Arc<Mutex<Phase>>, ctx: egui::Context) {
    std::thread::spawn(move || match run_install(&manifest, &phase, &ctx) {
        Ok(()) => set(&phase, &ctx, Phase::Installed),
        Err(message) => {
            tracing::warn!("mise à jour échouée : {message}");
            set(&phase, &ctx, Phase::Failed(message));
        }
    });
}

fn run_install(
    manifest: &UpdateManifest,
    phase: &Arc<Mutex<Phase>>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let exe = EXE
        .get()
        .cloned()
        .ok_or("exécutable du launcher introuvable")?;
    let paths = Paths::from_os().map_err(|e| e.to_string())?;
    let downloader = Downloader::new(paths.downloads_dir(), Box::new(HttpFetcher::new()));
    let total = manifest.max_bytes;
    let file = update::download_verified(
        manifest,
        update::UPDATE_PUBLIC_KEY,
        &downloader,
        &mut |received| set(phase, ctx, Phase::Downloading { received, total }),
        &CancelToken::new(),
    )
    .map_err(|e| e.to_string())?;
    let applied = update::apply_update(&exe, &file).map_err(|e| e.to_string());
    // Le fichier vient de notre propre cache de téléchargements : on le range.
    let _ = std::fs::remove_file(&file);
    applied
}

/// Bannière en haut de la fenêtre (rien tant qu'il n'y a rien à dire). À appeler à chaque image.
pub fn banner(ui: &mut Ui, state: &mut UpdateState) {
    let ctx = ui.ctx().clone();
    if !state.started {
        start(state, &ctx);
    }
    let current = lock(&state.phase).clone();
    if matches!(current, Phase::Idle) {
        return;
    }
    let mut next: Option<Phase> = None;
    let mut install_clicked: Option<UpdateManifest> = None;
    let mut restart = false;
    egui::Panel::top("update_banner")
        .resizable(false)
        .frame(Frame::NONE)
        .show(ui, |ui| {
            Frame::new()
                .fill(theme::ACCENT_BG)
                .stroke(Stroke::new(1.0, theme::ACCENT))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| match &current {
                        Phase::Idle => {}
                        Phase::Available(manifest) => {
                            ui.label(
                                RichText::new(format!(
                                    "Mise à jour {} disponible",
                                    manifest.version
                                ))
                                .color(theme::TEXT),
                            );
                            if widgets::primary_button(ui, "Installer", true) {
                                install_clicked = Some(manifest.clone());
                            }
                            if widgets::ghost_button(ui, "Plus tard") {
                                next = Some(Phase::Idle);
                            }
                        }
                        Phase::Downloading { received, total } => {
                            ui.label(RichText::new("Téléchargement de la mise à jour…"));
                            let fraction = (*received as f32 / (*total).max(1) as f32).min(1.0);
                            widgets::bar(ui, 160.0, fraction, theme::ACCENT);
                        }
                        Phase::Installed => {
                            ui.label(
                                RichText::new(
                                    "Mise à jour installée : redémarrez pour l'utiliser.",
                                )
                                .color(theme::GREEN),
                            );
                            if widgets::primary_button(ui, "Redémarrer", true) {
                                restart = true;
                            }
                        }
                        Phase::Failed(message) => {
                            ui.label(
                                RichText::new(format!("Mise à jour impossible : {message}"))
                                    .color(theme::RED_TEXT),
                            );
                            if widgets::ghost_button(ui, "Fermer") {
                                next = Some(Phase::Idle);
                            }
                        }
                    });
                });
        });
    if let Some(manifest) = install_clicked {
        // Le clic lance le téléchargement : rien n'est jamais installé sans lui.
        set(
            &state.phase,
            &ctx,
            Phase::Downloading {
                received: 0,
                total: manifest.max_bytes,
            },
        );
        install(manifest, Arc::clone(&state.phase), ctx.clone());
    }
    if let Some(next) = next {
        set(&state.phase, &ctx, next);
    }
    if restart {
        RESTART.store(true, Ordering::SeqCst);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Appelé par `main` APRÈS la fermeture de la fenêtre et le relâchement du verrou
/// d'instance unique : relance le nouvel exécutable si le joueur a cliqué « Redémarrer ».
pub fn relaunch_if_requested() {
    if !RESTART.load(Ordering::SeqCst) {
        return;
    }
    if let Some(exe) = EXE.get() {
        if let Err(e) = std::process::Command::new(exe).spawn() {
            tracing::warn!("redémarrage impossible : {e}");
        }
    }
}
