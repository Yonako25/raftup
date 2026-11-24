//! Les mondes natifs dans la Bibliothèque (ADR 0046).
//!
//! - les mondes de `mondes-natifs/` s'affichent comme des jeux, avec le marqueur
//!   « Monde natif » et leur fiche (nom, version, `format_version`, lieux, objets…) ;
//! - la fiche propose « Désinstaller le monde natif », en deux clics ;
//! - en options avancées, « Convertir un apworld en monde natif » produit l'export JSON
//!   d'un apworld installé (générateur officiel, options par défaut) puis le convertit
//!   (ADR 0042). S'il manque le générateur, c'est le même panneau d'installation que dans
//!   « Créer une partie » (`generator_setup`).
//!
//! La liste se relit à l'ouverture de l'écran et après chaque installation,
//! désinstallation ou conversion : quelques fichiers TOML bornés, lus dans le fil de
//! l'interface. La conversion, elle, tourne dans un thread à part.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::{RichText, Ui};
use launcher_core::generation::{
    installed_apworlds, pins, runtime_status, PythonBackend, RuntimeStatus,
};
use launcher_core::native_worlds::{
    apworld_game, convert_installed_apworld, list_native_worlds, uninstall_native_world,
    InstalledNativeWorld, NativeLibrary, NativeUninstall, EXPORT_FILE,
};
use launcher_core::registry::Trust;
use launcher_core::{CancelToken, Paths};

use super::generator_setup;
use super::registry_panel;
use crate::{theme, widgets};

/// Marqueur des cartes et de la fiche.
pub const MARKER: &str = "Monde natif";

/// Variable d'environnement (debug seulement) : sélectionne ce monde natif à l'ouverture
/// (captures de la macro : `RAFTUP_DEV_NATIF=apquest`).
const DEV_SELECT: &str = "RAFTUP_DEV_NATIF";

/// L'état des mondes natifs, gardé par l'écran Bibliothèque.
pub struct NativeState {
    /// Dernière lecture de `mondes-natifs/` ; `None` avant la première.
    library: Option<NativeLibrary>,
    /// Monde natif sélectionné (sa clé) ; exclusif avec le jeu sélectionné.
    pub selected: Option<String>,
    /// Premier clic sur « Désinstaller » : on attend la confirmation.
    confirm_uninstall: bool,
    /// Résultat de la dernière désinstallation.
    notice: Option<Result<String, String>>,
    conversion: Conversion,
}

/// Le bloc « Convertir un apworld en monde natif ».
#[derive(Default)]
struct Conversion {
    /// Apworld choisi.
    apworld: Option<PathBuf>,
    /// Nom du jeu, pré-rempli depuis `archipelago.json`, modifiable.
    game: String,
    /// L'apworld dont on a pré-rempli le jeu (pour ne pas écraser ce que le joueur a tapé).
    game_from: Option<PathBuf>,
    /// Le générateur officiel : même parcours que « Créer une partie ».
    setup: generator_setup::Setup,
    task: Option<ConvertTask>,
    /// Résultat de la dernière conversion.
    message: Option<Result<String, String>>,
}

/// Une conversion qui tourne.
struct ConvertTask {
    logs: Receiver<String>,
    result: Receiver<Result<String, String>>,
    /// Dernière ligne du journal du générateur (affichée sous la roue).
    last_line: String,
    /// Partagé avec le thread : l'annuler ici, il le voit là-bas.
    cancel: CancelToken,
}

impl NativeState {
    pub fn new() -> NativeState {
        let selected = if cfg!(debug_assertions) {
            std::env::var(DEV_SELECT).ok()
        } else {
            None
        };
        NativeState {
            library: None,
            selected,
            confirm_uninstall: false,
            notice: None,
            conversion: Conversion::default(),
        }
    }

    /// Relit `mondes-natifs/`. Une sélection qui n'existe plus est oubliée.
    pub fn refresh(&mut self, native_dir: &Path) {
        let library = list_native_worlds(native_dir);
        for (path, reason) in &library.rejected {
            tracing::warn!("monde natif écarté : {} ({reason})", path.display());
        }
        if let Some(key) = &self.selected {
            if library.get(key).is_none() {
                self.selected = None;
            }
        }
        self.library = Some(library);
    }

    /// Lit le dossier la première fois seulement.
    pub fn ensure_loaded(&mut self, native_dir: &Path) {
        if self.library.is_none() {
            self.refresh(native_dir);
        }
    }

    /// Les mondes natifs lus (vide avant la première lecture).
    pub fn worlds(&self) -> &[InstalledNativeWorld] {
        self.library.as_ref().map_or(&[], |l| l.worlds.as_slice())
    }

    /// Mondes écartés (illisibles, jeu en double), pour la ligne d'information.
    pub fn rejected(&self) -> &[(PathBuf, String)] {
        self.library.as_ref().map_or(&[], |l| l.rejected.as_slice())
    }

    pub fn count(&self) -> usize {
        self.worlds().len()
    }

    /// Sélectionne un monde natif (et oublie une confirmation en attente).
    pub fn select(&mut self, key: &str) {
        if self.selected.as_deref() != Some(key) {
            self.confirm_uninstall = false;
            self.notice = None;
        }
        self.selected = Some(key.to_string());
    }

    /// Le monde sélectionné, s'il existe encore.
    pub fn selected_world(&self) -> Option<&InstalledNativeWorld> {
        let key = self.selected.as_deref()?;
        self.library.as_ref()?.get(key)
    }

    /// Désinstalle le monde sélectionné, relit le dossier et rend le message.
    fn uninstall_selected(&mut self, native_dir: &Path) -> Option<Result<String, String>> {
        let world = self.selected_world()?.clone();
        let result = uninstall_native_world(native_dir, &world.key);
        self.confirm_uninstall = false;
        self.refresh(native_dir);
        Some(uninstall_message(&world.game, result))
    }
}

impl Default for NativeState {
    fn default() -> Self {
        NativeState::new()
    }
}

/// Le message d'une désinstallation.
pub fn uninstall_message(
    game: &str,
    result: Result<NativeUninstall, launcher_core::native_worlds::NativeWorldError>,
) -> Result<String, String> {
    match result {
        Ok(report) if report.left.is_empty() => Ok(format!(
            "Monde natif « {game} » retiré : {} fichier(s) effacé(s).",
            report.removed
        )),
        Ok(report) => Ok(format!(
            "Monde natif « {game} » retiré ({} fichier(s) effacé(s)). Laissé(s) dans son \
             dossier, car ajouté(s) à la main : {}.",
            report.removed,
            report.left.join(", ")
        )),
        Err(e) => Err(format!("Désinstallation impossible : {e}")),
    }
}

/// Texte d'état d'une carte : « 284 lieux · format 2 ».
pub fn card_status(world: &InstalledNativeWorld) -> String {
    let lieux = if world.locations > 1 { "lieux" } else { "lieu" };
    format!(
        "{} {lieux} · format {}",
        world.locations, world.format_version
    )
}

/// Les lignes de la fiche : (libellé, valeur), dans l'ordre d'affichage.
pub fn fact_rows(world: &InstalledNativeWorld) -> Vec<(&'static str, String)> {
    let version = world
        .version()
        .map_or_else(|| "non précisée".to_string(), |v| format!("v{v}"));
    vec![
        ("Version", version),
        ("format_version", world.format_version.to_string()),
        ("Lieux", world.locations.to_string()),
        ("Événements", world.events.to_string()),
        ("Objets", world.items.to_string()),
        ("Régions", world.regions.to_string()),
        ("Options", world.options.to_string()),
    ]
}

/// Fiche d'un monde natif (panneau de droite de la Bibliothèque).
/// `uninstalled` passe à vrai après une désinstallation (le registre doit relire ses marques).
pub fn detail(ui: &mut Ui, state: &mut NativeState, native_dir: &Path, uninstalled: &mut bool) {
    let Some(world) = state.selected_world().cloned() else {
        return;
    };
    widgets::panel(ui, "", |ui| {
        ui.horizontal(|ui| {
            super::library::small_cover(ui, 64.0, &world.key, &world.game);
            ui.vertical(|ui| {
                ui.label(widgets::title(&world.game, 26.0));
                ui.horizontal(|ui| {
                    widgets::pill(ui, MARKER, theme::ACCENT, theme::ACCENT_BG);
                    if let Some(origin) = world.receipt.as_ref().and_then(|r| r.registry.as_ref()) {
                        let trust = Trust {
                            level: origin.trust,
                            detail: None,
                        };
                        registry_panel::trust_badge(ui, &trust);
                    }
                });
                ui.label(widgets::dim(world.origin_label()).size(12.0));
            });
        });
        ui.add_space(10.0);
        match &state.notice {
            Some(Ok(text)) => {
                ui.label(RichText::new(text).color(theme::GREEN).size(13.0));
            }
            Some(Err(text)) => {
                ui.label(RichText::new(text).color(theme::AMBER).size(13.0));
            }
            None => {}
        }

        ui.label(widgets::caps("Fiche du monde"));
        egui::Grid::new("native_facts")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for (label, value) in fact_rows(&world) {
                    ui.label(widgets::dim(label));
                    ui.label(RichText::new(value).color(theme::TEXT));
                    ui.end_row();
                }
                if let Some(origin) = world.receipt.as_ref().and_then(|r| r.registry.as_ref()) {
                    ui.label(widgets::dim("Auteurs"));
                    ui.label(RichText::new(origin.authors.join(", ")).color(theme::TEXT));
                    ui.end_row();
                    ui.label(widgets::dim("Licence"));
                    ui.label(RichText::new(&origin.license).color(theme::TEXT));
                    ui.end_row();
                }
            });

        if let Some(conversion) = world.receipt.as_ref().and_then(|r| r.conversion.as_ref()) {
            ui.add_space(8.0);
            ui.label(
                widgets::dim(format!(
                    "Converti depuis {} (Archipelago {}) : {} point(s) à vérifier, listés en \
                     tête de world.toml. Le monde décrit les options par défaut seulement.",
                    conversion.apworld, conversion.ap_version, conversion.notes
                ))
                .size(12.0),
            );
            ui.label(widgets::caps("Export JSON"));
            super::library::code_box(ui, &world.path.join(EXPORT_FILE).display().to_string());
        }

        ui.add_space(12.0);
        ui.label(widgets::caps("Emplacement"));
        super::library::code_box(ui, &world.path.display().to_string());
        ui.label(
            widgets::dim(
                "Lu par le moteur Rust de « Créer une partie » (case « Moteur Rust ») : un \
                 joueur dont le YAML nomme ce jeu est généré sans Python.",
            )
            .size(12.0),
        );

        ui.add_space(12.0);
        ui.separator();
        if state.confirm_uninstall {
            ui.label(widgets::dim(
                "Les fichiers posés par Raftup seront effacés ; un fichier ajouté à la main \
                 dans le dossier du monde reste en place.",
            ));
            ui.horizontal(|ui| {
                if widgets::danger_button(ui, "Confirmer la désinstallation") {
                    state.notice = state.uninstall_selected(native_dir);
                    *uninstalled = true;
                }
                if widgets::ghost_button(ui, "Annuler") {
                    state.confirm_uninstall = false;
                }
            });
        } else if widgets::danger_button(ui, "Désinstaller le monde natif") {
            state.confirm_uninstall = true;
        }
    });
}

/// Message de fin de désinstallation, montré par la Bibliothèque quand la fiche a disparu.
pub fn take_notice(state: &mut NativeState) -> Option<Result<String, String>> {
    if state.selected_world().is_none() {
        state.notice.take()
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Conversion (options avancées).
// ---------------------------------------------------------------------------

/// Le bloc replié « Convertir un apworld en monde natif ». `converted` passe à vrai quand
/// une conversion vient de poser un monde (la liste a déjà été relue).
pub fn conversion_block(
    ui: &mut Ui,
    state: &mut NativeState,
    paths: &Paths,
    worlds_dir: &Path,
    converted: &mut bool,
) {
    poll_conversion(state, &paths.native_worlds_dir(), converted);
    let mut open_now = false;
    ui.add_space(8.0);
    widgets::advanced(
        ui,
        "library_convert_apworld",
        "Convertir un apworld en monde natif",
        &mut open_now,
        |ui| conversion_contents(ui, state, paths, worlds_dir),
    );
}

fn conversion_contents(ui: &mut Ui, state: &mut NativeState, paths: &Paths, worlds_dir: &Path) {
    let conv = &mut state.conversion;
    ui.label(widgets::dim(
        "Raftup lance le générateur officiel d'Archipelago sur l'apworld choisi (un joueur, \
         options par défaut), garde son export JSON (celui que lit « raftup-monde importer ») \
         puis le convertit en monde natif dans mondes-natifs. Le monde obtenu décrit ces \
         options-là seulement : relisez les points à vérifier en tête du fichier.",
    ));
    ui.add_space(6.0);
    generator_setup::show(ui, &mut conv.setup);

    let apworlds = installed_apworlds(worlds_dir);
    if apworlds.is_empty() {
        ui.label(
            widgets::dim(format!(
                "Aucun apworld installé dans {} : installes-en un depuis « Mondes du registre » \
                 ou avec le profil d'un jeu.",
                worlds_dir.display()
            ))
            .size(12.0),
        );
        return;
    }
    // Un apworld disparu (désinstallé) n'est plus proposé.
    if conv.apworld.as_ref().is_none_or(|a| !apworlds.contains(a)) {
        conv.apworld = apworlds.first().cloned();
    }
    let busy = conv.task.is_some() || conv.setup.is_busy();
    egui::Grid::new("native_convert")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(widgets::dim("Apworld"));
            let current = conv.apworld.as_deref().map(file_name).unwrap_or_default();
            egui::ComboBox::from_id_salt("native_convert_apworld")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for apworld in &apworlds {
                        let selected = conv.apworld.as_ref() == Some(apworld);
                        if ui.selectable_label(selected, file_name(apworld)).clicked() {
                            conv.apworld = Some(apworld.clone());
                        }
                    }
                });
            ui.end_row();

            // Jeu pré-rempli depuis `archipelago.json` quand l'apworld change.
            if conv.game_from != conv.apworld {
                conv.game = conv
                    .apworld
                    .as_deref()
                    .and_then(apworld_game)
                    .unwrap_or_default();
                conv.game_from = conv.apworld.clone();
            }
            ui.label(widgets::dim("Jeu"));
            ui.add(
                egui::TextEdit::singleline(&mut conv.game).hint_text("nom du jeu côté Archipelago"),
            );
            ui.end_row();
        });
    ui.add_space(6.0);
    let mut start = false;
    ui.horizontal(|ui| {
        start = widgets::primary_button(ui, "Exporter et convertir", !busy);
        if let Some(task) = &conv.task {
            ui.spinner();
            let line = if task.last_line.is_empty() {
                "Export en cours…"
            } else {
                task.last_line.as_str()
            };
            ui.label(widgets::dim(line).size(12.0));
            if widgets::ghost_button(ui, "Annuler") {
                task.cancel.cancel();
            }
        }
    });
    match &conv.message {
        Some(Ok(text)) => {
            ui.label(RichText::new(text).color(theme::GREEN).size(12.0));
        }
        Some(Err(text)) => {
            ui.label(RichText::new(text).color(theme::AMBER).size(12.0));
        }
        None => {}
    }
    if start {
        request_conversion(ui.ctx(), conv, paths);
    }
}

/// Clic sur « Exporter et convertir » : vérifie le formulaire, puis le générateur ; s'il
/// manque, son panneau d'installation s'affiche (on recliquera ensuite).
fn request_conversion(ctx: &egui::Context, conv: &mut Conversion, paths: &Paths) {
    let Some(apworld) = conv.apworld.clone() else {
        return;
    };
    let game = conv.game.trim().to_string();
    if let Err(reason) = check_form(&game) {
        conv.message = Some(Err(reason));
        return;
    }
    conv.message = None;
    conv.setup = generator_setup::check();
    if !matches!(conv.setup, generator_setup::Setup::Ready) {
        return;
    }
    conv.task = Some(start_conversion(ctx, paths.clone(), apworld, game));
}

/// Le nom du jeu est obligatoire (il va dans le YAML du joueur).
pub fn check_form(game: &str) -> Result<(), String> {
    if game.trim().is_empty() {
        return Err(
            "Indique le nom du jeu (celui des YAML d'Archipelago) : cet apworld ne le donne pas."
                .to_string(),
        );
    }
    Ok(())
}

/// Lance l'export puis la conversion dans un thread.
fn start_conversion(
    ctx: &egui::Context,
    paths: Paths,
    apworld: PathBuf,
    game: String,
) -> ConvertTask {
    let (log_sender, logs) = mpsc::channel();
    let (sender, result) = mpsc::channel();
    let cancel = CancelToken::new();
    // Copies possédées par le thread (`move`) : l'écran garde les siennes.
    let thread_cancel = cancel.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let outcome = run_conversion(&paths, &apworld, &game, &thread_cancel, log_sender);
        let _ = sender.send(outcome);
        ctx.request_repaint();
    });
    ConvertTask {
        logs,
        result,
        last_line: String::new(),
        cancel,
    }
}

/// Le travail du thread : le générateur installé, puis `convert_installed_apworld`.
fn run_conversion(
    paths: &Paths,
    apworld: &Path,
    game: &str,
    cancel: &CancelToken,
    log: mpsc::Sender<String>,
) -> Result<String, String> {
    let pins = pins().map_err(|e| format!("Générateur mal configuré : {e}"))?;
    let runtime = match runtime_status(paths, &pins) {
        RuntimeStatus::Ready(runtime) => runtime,
        _ => return Err("Le générateur n'est pas installé.".to_string()),
    };
    let backend = PythonBackend::new(runtime);
    let converted = convert_installed_apworld(paths, &backend, apworld, game, cancel, log)
        .map_err(|e| format!("Conversion impossible : {e}"))?;
    Ok(format!(
        "Monde natif « {} » créé dans {} : {} point(s) à vérifier, en tête de world.toml.",
        converted.game,
        converted.path.display(),
        converted.notes.len()
    ))
}

/// Lit le journal et le résultat de la conversion, sans attendre.
fn poll_conversion(state: &mut NativeState, native_dir: &Path, converted: &mut bool) {
    let Some(task) = &mut state.conversion.task else {
        return;
    };
    // `while let` : on vide toutes les lignes arrivées depuis la dernière image.
    while let Ok(line) = task.logs.try_recv() {
        task.last_line = line.chars().take(120).collect();
    }
    let finished = match task.result.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err(
            "La conversion s'est interrompue sans réponse.".to_string(),
        )),
    };
    if let Some(result) = finished {
        if result.is_ok() {
            state.refresh(native_dir);
            *converted = true;
        }
        state.conversion.message = Some(result);
        state.conversion.task = None;
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    const APQUEST: &str = include_str!("../../../../spec/monde-natif/exemples/apquest/world.toml");

    fn dossier_avec_apquest() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mondes-natifs");
        std::fs::create_dir_all(dir.join("apquest")).unwrap();
        std::fs::write(dir.join("apquest").join("world.toml"), APQUEST).unwrap();
        (tmp, dir)
    }

    #[test]
    fn lecture_puis_selection_puis_desinstallation() {
        let (_tmp, dir) = dossier_avec_apquest();
        let mut state = NativeState::new();
        assert_eq!(state.count(), 0);
        state.ensure_loaded(&dir);
        assert_eq!(state.count(), 1);
        state.select("apquest");
        let monde = state.selected_world().unwrap();
        assert_eq!(monde.game, "APQuest");
        assert!(card_status(monde).ends_with("format 1"));

        // Deux clics : le premier demande confirmation, le second retire le monde.
        state.confirm_uninstall = true;
        let message = state.uninstall_selected(&dir).unwrap().unwrap();
        assert!(message.contains("APQuest") && message.contains("retiré"));
        assert_eq!(state.count(), 0);
        // La sélection disparue est oubliée ; le message reste pour la Bibliothèque.
        assert!(state.selected.is_none());
        assert!(!dir.join("apquest").exists());
    }

    #[test]
    fn fiche_complete() {
        let (_tmp, dir) = dossier_avec_apquest();
        let library = list_native_worlds(&dir);
        let rows = fact_rows(&library.worlds[0]);
        let labels: Vec<&str> = rows.iter().map(|(l, _)| *l).collect();
        assert_eq!(
            labels,
            [
                "Version",
                "format_version",
                "Lieux",
                "Événements",
                "Objets",
                "Régions",
                "Options"
            ]
        );
        // Un `world.toml` posé à la main n'a pas de version.
        assert_eq!(rows[0].1, "non précisée");
        assert_eq!(rows[1].1, "1");
        assert_ne!(rows[2].1, "0");
    }

    #[test]
    fn message_de_desinstallation() {
        let fait = NativeUninstall {
            removed: 2,
            left: Vec::new(),
        };
        assert_eq!(
            uninstall_message("APQuest", Ok(fait)).unwrap(),
            "Monde natif « APQuest » retiré : 2 fichier(s) effacé(s)."
        );
        let reste = NativeUninstall {
            removed: 2,
            left: vec!["notes.txt".into()],
        };
        assert!(uninstall_message("APQuest", Ok(reste))
            .unwrap()
            .ends_with("notes.txt."));
        let erreur = launcher_core::native_worlds::NativeWorldError::NotInstalled("x".into());
        assert!(uninstall_message("APQuest", Err(erreur)).is_err());
    }

    #[test]
    fn formulaire_de_conversion() {
        assert!(check_form("  ").is_err());
        assert!(check_form("APQuest").is_ok());
    }

    #[test]
    fn fin_de_conversion_relit_le_dossier() {
        let (_tmp, dir) = dossier_avec_apquest();
        let mut state = NativeState::new();
        let (log_sender, logs) = mpsc::channel();
        let (sender, result) = mpsc::channel();
        log_sender.send("Mondes exportés".to_string()).unwrap();
        state.conversion.task = Some(ConvertTask {
            logs,
            result,
            last_line: String::new(),
            cancel: CancelToken::new(),
        });
        let mut converti = false;
        // Pas encore fini : la dernière ligne du journal est gardée.
        poll_conversion(&mut state, &dir, &mut converti);
        assert!(!converti);
        assert_eq!(
            state.conversion.task.as_ref().unwrap().last_line,
            "Mondes exportés"
        );
        sender.send(Ok("Monde natif créé".to_string())).unwrap();
        poll_conversion(&mut state, &dir, &mut converti);
        assert!(converti);
        assert!(state.conversion.task.is_none());
        assert_eq!(state.count(), 1);
        assert_eq!(
            state.conversion.message,
            Some(Ok("Monde natif créé".to_string()))
        );
    }
}
