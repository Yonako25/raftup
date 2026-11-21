//! Écran « Bibliothèque » : les jeux du catalogue, leur dossier, l'état des mods,
//! et les boutons Installer / Lancer / Désinstaller (ADR 0015).
//!
//! Tout ce qui peut durer (détection des jeux, installation, désinstallation, lancement)
//! tourne dans un thread à part. La boucle d'affichage ne fait que lire un canal
//! (`try_recv`, qui n'attend jamais) : la fenêtre reste fluide pendant un téléchargement.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};
use launcher_core::activity::RunningGames;
use launcher_core::{
    detect_games, install, launch, plan_launch, plan_launch_offline, status, uninstall,
    validate_manual_dir, Catalog, Credentials, DetectRule, DetectedGame, DetectionRoots,
    Downloader, GameState, HttpFetcher, InstallOptions, InstallStatus, Paths, Profile, Store,
};

use super::registry_panel::{self, RegistryPanel};
use crate::app::Page;
use crate::format;
use crate::theme;
use crate::widgets;
use crate::worker::Shared;

/// Version d'Archipelago visée (dossier des apworlds), la même que celle de la barre latérale.
pub const AP_VERSION: &str = "0.6.8";

/// Écart minimal (en octets) entre deux messages de progression : sans lui, le thread
/// enverrait un message par paquet réseau reçu.
const PROGRESS_STEP: u64 = 256 * 1024;

/// Où se trouve un jeu sur ce PC, et ce que l'on sait de ses mods.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// Dossier de chaque jeu trouvé, rangé par `game.id`.
    pub found: BTreeMap<String, DetectedGame>,
    /// État des mods de chaque jeu (`Err` : reçu illisible, message en clair).
    pub status: BTreeMap<String, Result<InstallStatus, String>>,
}

/// Ce qu'une tâche (installation, désinstallation, lancement) renvoie à l'interface.
enum TaskMessage {
    /// Téléchargement en cours : nom de l'étape et octets reçus.
    Progress { step: String, bytes: u64 },
    /// Fin de la tâche : texte à afficher, en vert (`Ok`) ou en rouge (`Err`).
    Done(Result<String, String>),
    /// Le jeu est lancé. `Some` : la connexion se fait à la main dans le jeu.
    Launched(Result<Option<String>, String>),
}

/// Une tâche en cours sur un jeu.
struct Task {
    game_id: String,
    /// Texte court : « Installation », « Désinstallation », « Lancement ».
    label: &'static str,
    progress: Option<(String, u64)>,
    receiver: Receiver<TaskMessage>,
}

/// Adresse, slot et mot de passe pour « Lancer ».
#[derive(Default)]
struct LaunchForm {
    address: String,
    slot: String,
    password: String,
    /// Adresse de la partie hébergée déjà recopiée : on ne la recopie qu'une fois,
    /// pour ne pas effacer ce que le joueur a tapé.
    prefilled_from: Option<String>,
    /// Ouvre « Se connecter à la main » à la prochaine image (invitation reçue).
    open_now: bool,
}

/// Message affiché sous l'en-tête.
enum Notice {
    Info(String),
    Error(String),
}

/// L'état de l'écran, gardé d'une image à l'autre.
pub struct LibraryState {
    catalog: Result<Catalog, String>,
    paths: Result<Paths, String>,
    /// Dernière détection ; `None` tant qu'elle n'a jamais abouti.
    scan: Option<Scan>,
    /// Détection en cours.
    scanning: Option<Receiver<Scan>>,
    selected: Option<String>,
    task: Option<Task>,
    form: LaunchForm,
    notice: Option<Notice>,
    /// Le joueur a cliqué une première fois sur « Désinstaller » : on attend la confirmation.
    confirm_uninstall: bool,
    /// Choix d'un dossier demandé pour ce jeu (la boîte de dialogue s'ouvre hors du dessin).
    pending_pick: Option<String>,
    /// Debug seulement (`RAFTUP_DEV_INSTALLER=1`) : lancer l'installation du jeu
    /// sélectionné dès qu'elle est possible, pour vérifier la progression sans cliquer.
    dev_auto_install: bool,
    /// Registre de paquets signés (ADR 0032) : jeux en plus et apworlds.
    registry: RegistryPanel,
    /// Le registre a ajouté des jeux pendant une détection : en relancer une à la fin.
    rescan_pending: bool,
    /// Jeux lancés depuis la Bibliothèque qui tournent encore (présence « en partie »).
    running: RunningGames,
}

impl LibraryState {
    /// Charge le catalogue embarqué et les dossiers de Raftup. Une erreur est gardée
    /// en texte pour être affichée : l'écran s'ouvre quand même.
    pub fn new() -> LibraryState {
        LibraryState {
            catalog: load_catalog(),
            paths: load_paths(),
            scan: None,
            scanning: None,
            selected: None,
            task: None,
            form: LaunchForm::default(),
            notice: None,
            confirm_uninstall: false,
            pending_pick: None,
            dev_auto_install: cfg!(debug_assertions)
                && std::env::var_os("RAFTUP_DEV_INSTALLER").is_some(),
            registry: RegistryPanel::new(),
            rescan_pending: false,
            running: RunningGames::new(),
        }
    }

    /// Tests de l'interface (`ui_tests`) : catalogue, dossiers et détection donnés.
    /// La détection est déjà « faite » (`scan`) et le registre est coupé : l'écran ne
    /// lance aucun thread, ne lit pas Steam et n'appelle pas le réseau.
    #[cfg(test)]
    pub(crate) fn for_tests(catalog: Catalog, paths: Paths, scan: Scan) -> LibraryState {
        LibraryState {
            catalog: Ok(catalog),
            paths: Ok(paths),
            scan: Some(scan),
            scanning: None,
            selected: None,
            task: None,
            form: LaunchForm::default(),
            notice: None,
            confirm_uninstall: false,
            pending_pick: None,
            dev_auto_install: false,
            registry: RegistryPanel::disabled(),
            rescan_pending: false,
            running: RunningGames::new(),
        }
    }

    /// Les jeux lancés depuis la Bibliothèque qui tournent encore.
    pub fn running_games(&self) -> Vec<String> {
        self.running.names()
    }

    /// Nombre de jeux du catalogue (compteur de la barre latérale).
    pub fn game_count(&self) -> usize {
        self.catalog.as_ref().map_or(0, |c| c.profiles.len())
    }

    /// Le catalogue, s'il s'est chargé (l'écran « Rejoindre » y cherche le jeu du lien).
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref().ok()
    }

    /// La dernière détection des jeux, si elle a abouti.
    pub fn scan_result(&self) -> Option<&Scan> {
        self.scan.as_ref()
    }
}

/// Ouvre la fiche d'un jeu avec le formulaire de connexion pré-rempli (écran « Rejoindre »).
/// Le mot de passe n'est jamais dans un lien : on le vide pour que le joueur le saisisse.
pub fn open_with_invite(state: &mut LibraryState, game_id: &str, address: &str, slot: &str) {
    state.selected = Some(game_id.to_string());
    state.confirm_uninstall = false;
    state.notice = None;
    state.form.address = address.to_string();
    state.form.slot = slot.to_string();
    state.form.password.clear();
    // Le joueur doit voir ce que l'invitation a rempli (et taper le mot de passe).
    state.form.open_now = true;
}

/// Variable d'environnement (builds de debug seulement) : dossier de profils `.toml`
/// chargés à la place du catalogue embarqué. Sert à essayer l'écran avec un profil de
/// démonstration, sans jamais en mettre un dans `profiles/`.
const DEV_PROFILES: &str = "RAFTUP_DEV_PROFILS";
/// Variable d'environnement (debug seulement) : racine où ranger config, reçus et cache
/// au lieu des vrais dossiers du joueur (`Paths::under`).
const DEV_ROOT: &str = "RAFTUP_DEV_RACINE";

/// Le catalogue embarqué ou, en debug, celui de `RAFTUP_DEV_PROFILS`.
pub(crate) fn load_catalog() -> Result<Catalog, String> {
    // `cfg!(debug_assertions)` vaut faux en `--release` : le binaire livré ignore la variable.
    if let (true, Some(dir)) = (cfg!(debug_assertions), std::env::var_os(DEV_PROFILES)) {
        return dev_catalog(Path::new(&dir));
    }
    Catalog::embedded().map_err(|e| e.to_string())
}

/// Lit tous les `.toml` d'un dossier, triés par nom (ordre stable).
/// `pub(crate)` : les tests de l'interface lisent ainsi le dossier `profiles/`.
pub(crate) fn dev_catalog(dir: &Path) -> Result<Catalog, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    let mut texts: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{} : {e}", path.display()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            texts.push((name, text));
        }
    }
    texts.sort();
    // `from_texts` attend des `&str` : on emprunte les chaînes possédées par `texts`.
    let borrowed: Vec<(&str, &str)> = texts
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    Catalog::from_texts(&borrowed).map_err(|e| e.to_string())
}

/// Les vrais dossiers de Raftup ou, en debug, ceux sous `RAFTUP_DEV_RACINE`.
pub(crate) fn load_paths() -> Result<Paths, String> {
    if let (true, Some(root)) = (cfg!(debug_assertions), std::env::var_os(DEV_ROOT)) {
        return Ok(Paths::under(Path::new(&root)));
    }
    Paths::from_os().map_err(|e| e.to_string())
}

impl Default for LibraryState {
    fn default() -> Self {
        LibraryState::new()
    }
}

// ---------------------------------------------------------------------------
// Logique sans interface (testée plus bas).
// ---------------------------------------------------------------------------

/// Règle de détection d'un profil.
pub fn rule(profile: &Profile) -> DetectRule {
    let detect = &profile.game.detect;
    DetectRule {
        game_id: profile.game.id.clone(),
        steam_appid: detect.steam_appid,
        epic_app_name: detect.epic_app_name.clone(),
        expected_exe: detect.expected_exe.clone(),
    }
}

/// Cherche les jeux du catalogue et lit l'état de leurs mods. Bloquant : à appeler
/// depuis un thread.
///
/// Un dossier choisi à la main (`config.toml`) passe avant Steam et Epic.
pub fn scan(catalog: &Catalog, paths: &Paths, roots: &DetectionRoots) -> Scan {
    let rules: Vec<DetectRule> = catalog.games().map(rule).collect();
    let mut found = BTreeMap::new();
    // `detect_games` trie par jeu puis par lanceur (Steam avant Epic) : on garde le
    // premier trouvé pour chaque jeu (`entry().or_insert` n'écrase pas).
    for game in detect_games(roots, &rules) {
        found.entry(game.game_id.clone()).or_insert(game);
    }
    // Dossiers choisis à la main. Une config illisible n'empêche pas la détection.
    if let Ok(loaded) = launcher_core::config::load(&paths.config_file()) {
        for (game_id, settings) in &loaded.config.games {
            let Some(dir) = &settings.game_dir else {
                continue;
            };
            if let Some(rule) = rules.iter().find(|r| &r.game_id == game_id) {
                if let Ok(game) = validate_manual_dir(rule, dir) {
                    found.insert(game_id.clone(), game);
                }
            }
        }
    }
    let status = catalog
        .games()
        .map(|p| {
            let state = status(paths, p).map_err(|e| e.to_string());
            (p.game.id.clone(), state)
        })
        .collect();
    Scan { found, status }
}

/// Ce que l'écran montre pour un jeu : texte d'état, couleur, boutons permis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameView {
    pub label: String,
    pub tone: Tone,
    /// Installer (ou mettre à jour, ou réparer) est possible.
    pub can_install: bool,
    pub install_label: &'static str,
    pub can_launch: bool,
    pub can_uninstall: bool,
}

/// Couleur du texte d'état.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ready,
    Accent,
    Warning,
    Dim,
}

/// Calcule l'état affiché d'un jeu d'après sa détection et son reçu d'installation.
pub fn view(
    profile: &Profile,
    found: Option<&DetectedGame>,
    status: Option<&Result<InstallStatus, String>>,
) -> GameView {
    let broken = profile.game.status == launcher_core::profile::GameStatus::Broken;
    let installed = matches!(
        status,
        Some(Ok(InstallStatus::Installed | InstallStatus::UpdateAvailable))
    );
    let has_record = matches!(
        status,
        Some(Ok(InstallStatus::Installed
            | InstallStatus::UpdateAvailable
            | InstallStatus::InProgress))
    );
    // Seul un jeu au repos accepte des mods (ADR 0015, détection).
    let ready = found.is_some_and(|g| g.state == GameState::Ready);

    let (label, tone) = match (found, status) {
        (_, Some(Err(e))) => (format!("Reçu illisible : {e}"), Tone::Warning),
        (None, _) => ("Jeu non trouvé".to_string(), Tone::Dim),
        (Some(g), _) if g.state == GameState::Updating => {
            ("Mise à jour du jeu en cours".to_string(), Tone::Warning)
        }
        (Some(g), _) if g.state == GameState::Running => {
            ("Jeu déjà lancé".to_string(), Tone::Accent)
        }
        (_, Some(Ok(InstallStatus::Installed))) => ("Prêt".to_string(), Tone::Ready),
        (_, Some(Ok(InstallStatus::UpdateAvailable))) => {
            ("Mise à jour disponible".to_string(), Tone::Accent)
        }
        (_, Some(Ok(InstallStatus::InProgress))) => {
            ("Installation interrompue".to_string(), Tone::Warning)
        }
        _ if broken => ("Profil désactivé".to_string(), Tone::Warning),
        _ => ("Mod non installé".to_string(), Tone::Dim),
    };
    let install_label = match status {
        Some(Ok(InstallStatus::UpdateAvailable)) => "Mettre à jour",
        Some(Ok(InstallStatus::InProgress)) => "Réparer",
        Some(Ok(InstallStatus::Installed)) => "Réinstaller",
        _ => "Installer",
    };
    GameView {
        label,
        tone,
        can_install: ready && !broken && !matches!(status, Some(Err(_))),
        install_label,
        can_launch: installed && found.is_some(),
        can_uninstall: has_record,
    }
}

/// Identifiants du formulaire, ou `None` s'il manque l'adresse ou le slot.
/// Ce que « Lancer le jeu » fera avec le formulaire de connexion.
#[derive(Debug, PartialEq, Eq)]
enum LaunchMode {
    /// Adresse et slot remplis : le jeu se connecte à la partie.
    Connect(Credentials),
    /// Les deux vides : le jeu démarre seul (premier lancement, jeu hors partie).
    Offline,
    /// Un seul des deux rempli : on attend l'autre, le bouton reste grisé.
    Incomplete,
}

fn launch_mode(form: &LaunchForm) -> LaunchMode {
    match credentials(form) {
        Some(credentials) => LaunchMode::Connect(credentials),
        None if form.address.trim().is_empty() && form.slot.trim().is_empty() => {
            LaunchMode::Offline
        }
        None => LaunchMode::Incomplete,
    }
}

fn credentials(form: &LaunchForm) -> Option<Credentials> {
    let address = form.address.trim();
    let slot = form.slot.trim();
    if address.is_empty() || slot.is_empty() {
        return None;
    }
    Some(Credentials {
        address: address.to_string(),
        slot: slot.to_string(),
        password: Some(form.password.clone()).filter(|p| !p.is_empty()),
    })
}

/// Nom lisible d'une étape du profil (« MelonLoader ») à partir de son `id`.
fn step_name(profile: &Profile, step_id: &str) -> String {
    profile
        .steps
        .iter()
        .find(|s| s.id == step_id)
        .map_or_else(|| step_id.to_string(), |s| s.name.clone())
}

/// « Installation interrompue sans réponse. » ; accordé au masculin pour « Lancement ».
fn interrupted(label: &str) -> String {
    let ending = if label == "Lancement" {
        "interrompu"
    } else {
        "interrompue"
    };
    format!("{label} {ending} sans réponse.")
}

/// Taille lisible : « 12,3 Mo ».
pub(super) fn megabytes(bytes: u64) -> String {
    format!("{:.1} Mo", bytes as f64 / 1_000_000.0).replace('.', ",")
}

// ---------------------------------------------------------------------------
// Threads.
// ---------------------------------------------------------------------------

/// Lance `work` dans un thread et rend le côté « réception » du canal.
///
/// `Send + 'static` : la fermeture part dans un autre thread, elle doit donc posséder
/// tout ce qu'elle utilise (rien d'emprunté à l'écran) et pouvoir changer de thread.
/// Quand le thread se termine, l'émetteur est détruit : si le travail panique sans
/// avoir répondu, `try_recv` renvoie `Disconnected` et l'écran le signale.
fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    work: impl FnOnce(&Sender<T>) + Send + 'static,
) -> Receiver<T> {
    let (sender, receiver) = mpsc::channel();
    // `Context` est un pointeur partagé : le cloner ne copie pas l'interface.
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        work(&sender);
        // Réveille l'interface pour qu'elle lise le résultat tout de suite.
        ctx.request_repaint();
    });
    receiver
}

impl LibraryState {
    /// Relance la détection (sauf si elle tourne déjà).
    fn start_scan(&mut self, ctx: &egui::Context) {
        if self.scanning.is_some() {
            return;
        }
        let (Ok(catalog), Ok(paths)) = (&self.catalog, &self.paths) else {
            return;
        };
        // Copies possédées par le thread : l'écran garde les siennes.
        let catalog = catalog.clone();
        let paths = paths.clone();
        self.scanning = Some(spawn(ctx, move |sender| {
            let roots = DetectionRoots::from_system();
            let _ = sender.send(scan(&catalog, &paths, &roots));
        }));
    }

    /// Lit les messages des threads, sans jamais attendre.
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.scanning {
            match receiver.try_recv() {
                Ok(scan) => {
                    self.scan = Some(scan);
                    self.scanning = None;
                    // Des jeux du registre sont arrivés pendant la détection : on refait
                    // une détection qui les connaît.
                    if std::mem::take(&mut self.rescan_pending) {
                        self.start_scan(ctx);
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.scanning = None;
                    self.notice = Some(Notice::Error("La détection des jeux a échoué.".into()));
                }
            }
        }

        let mut finished = false;
        if let Some(task) = &mut self.task {
            // `loop` : on vide tous les messages arrivés depuis la dernière image.
            loop {
                match task.receiver.try_recv() {
                    Ok(TaskMessage::Progress { step, bytes }) => {
                        task.progress = Some((step, bytes));
                    }
                    Ok(TaskMessage::Done(result)) => {
                        self.notice = Some(match result {
                            Ok(text) => Notice::Info(text),
                            Err(text) => Notice::Error(text),
                        });
                        finished = true;
                        break;
                    }
                    Ok(TaskMessage::Launched(result)) => {
                        self.notice = Some(match result {
                            Ok(Some(instructions)) => {
                                // Connexion à la main : on copie l'adresse et le slot.
                                ctx.copy_text(format!("{}\n{}", self.form.address, self.form.slot));
                                Notice::Info(format!(
                                    "Jeu lancé. {instructions} (adresse et slot copiés dans le presse-papiers)"
                                ))
                            }
                            Ok(None) => Notice::Info("Jeu lancé.".into()),
                            Err(text) => Notice::Error(text),
                        });
                        finished = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.notice = Some(Notice::Error(interrupted(task.label)));
                        finished = true;
                        break;
                    }
                }
            }
        }
        if finished {
            self.task = None;
            // Le reçu a pu changer : on relit l'état des jeux.
            self.start_scan(ctx);
        }
    }

    /// `force` : défaire d'abord une installation interrompue (« Réparer »).
    fn start_install(
        &mut self,
        ctx: &egui::Context,
        profile: &Profile,
        game_dir: PathBuf,
        force: bool,
    ) {
        let Ok(paths) = &self.paths else { return };
        let paths = paths.clone();
        let game_id = profile.game.id.clone();
        let profile = profile.clone();
        let receiver = spawn(ctx, move |sender| {
            let downloader = Downloader::new(paths.downloads_dir(), Box::new(HttpFetcher::new()));
            let options = InstallOptions {
                force,
                ap_version: AP_VERSION.to_string(),
            };
            let mut last = (String::new(), 0u64);
            // Fermeture appelée par `install` à chaque paquet reçu.
            let mut progress = |step_id: &str, bytes: u64| {
                if step_id != last.0 || bytes >= last.1 + PROGRESS_STEP {
                    last = (step_id.to_string(), bytes);
                    let step = step_name(&profile, step_id);
                    let _ = sender.send(TaskMessage::Progress { step, bytes });
                }
            };
            let result = install(
                &paths,
                &profile,
                &game_dir,
                &downloader,
                &options,
                &mut progress,
            );
            let text = match result {
                Ok(report) if report.already_up_to_date => {
                    Ok("Les mods étaient déjà à jour.".into())
                }
                Ok(report) => Ok(format!(
                    "{} installé : {} étape(s), {} fichier(s) posé(s).",
                    profile.game.name, report.steps_installed, report.files_written
                )),
                Err(e) => Err(format!("Installation impossible : {e}")),
            };
            let _ = sender.send(TaskMessage::Done(text));
        });
        self.begin(game_id, "Installation", receiver);
    }

    fn start_uninstall(&mut self, ctx: &egui::Context, game_id: String) {
        let Ok(paths) = &self.paths else { return };
        let paths = paths.clone();
        let id = game_id.clone();
        let receiver = spawn(ctx, move |sender| {
            let text = match uninstall(&paths, &id, false) {
                Ok(report) if report.conflicts.is_empty() => Ok(format!(
                    "Mods retirés : {} fichier(s) supprimé(s), {} original(aux) restauré(s).",
                    report.removed, report.restored
                )),
                Ok(report) => Err(format!(
                    "{} fichier(s) modifié(s) depuis l'installation ont été laissés en place.",
                    report.conflicts.len()
                )),
                Err(e) => Err(format!("Désinstallation impossible : {e}")),
            };
            let _ = sender.send(TaskMessage::Done(text));
        });
        self.begin(game_id, "Désinstallation", receiver);
    }

    fn start_launch(&mut self, ctx: &egui::Context, profile: &Profile, game_dir: &Path) {
        let Ok(paths) = &self.paths else { return };
        let worlds_dir = paths.worlds_dir(AP_VERSION);
        // Le plan est une fonction pure : on le calcule ici, ses erreurs arrivent tout de suite.
        let planned = match launch_mode(&self.form) {
            LaunchMode::Connect(credentials) => {
                plan_launch(profile, game_dir, &worlds_dir, &credentials)
            }
            LaunchMode::Offline => plan_launch_offline(profile, game_dir, &worlds_dir),
            LaunchMode::Incomplete => return,
        };
        let plan = match planned {
            Ok(plan) => plan,
            Err(e) => {
                self.notice = Some(Notice::Error(format!("Lancement impossible : {e}")));
                return;
            }
        };
        // Nom du jeu pour la présence « en partie » sur Rafthub (rien d'autre du plan :
        // ni adresse, ni slot, ni mot de passe).
        let game_name = profile.game.name.clone();
        let running = self.running.clone();
        // `launch` écrit la config du mod puis démarre le jeu : dans le thread.
        let receiver = spawn(ctx, move |sender| {
            let mut child = match launch(&plan) {
                Ok(child) => child,
                Err(e) => {
                    let message = format!("Lancement impossible : {e}");
                    let _ = sender.send(TaskMessage::Launched(Err(message)));
                    return;
                }
            };
            // Le jeu compte comme lancé tant que `_playing` existe, c'est-à-dire jusqu'à
            // la fin de son processus (`wait` bloque ce thread, pas l'interface).
            let _playing = running.start(game_name);
            let _ = sender.send(TaskMessage::Launched(Ok(plan.manual_instructions.clone())));
            // Un jeu qui passe par un autre lanceur (Steam…) peut rendre la main tout de
            // suite : la présence revient alors à « en ligne » plus tôt que prévu.
            let _ = child.wait();
        });
        self.begin(profile.game.id.clone(), "Lancement", receiver);
    }

    fn begin(&mut self, game_id: String, label: &'static str, receiver: Receiver<TaskMessage>) {
        self.notice = None;
        self.confirm_uninstall = false;
        self.task = Some(Task {
            game_id,
            label,
            progress: None,
            receiver,
        });
    }
}

/// Ouvre la boîte « Choisir le dossier du jeu » si elle a été demandée.
///
/// Appelée par `app.rs` après le dessin, verrou du worker relâché : la boîte bloque.
pub fn open_pending_dialog(state: &mut LibraryState, ctx: &egui::Context) {
    let Some(game_id) = state.pending_pick.take() else {
        return;
    };
    let Some(dir) = rfd::FileDialog::new()
        .set_title("Dossier du jeu")
        .pick_folder()
    else {
        return;
    };
    let (Ok(catalog), Ok(paths)) = (&state.catalog, &state.paths) else {
        return;
    };
    let Some(profile) = catalog.get(&game_id) else {
        return;
    };
    // Vérifier un dossier ne lit qu'un nom de fichier : rapide, pas besoin de thread.
    if let Err(e) = validate_manual_dir(&rule(profile), &dir) {
        state.notice = Some(Notice::Error(format!("Dossier refusé : {e}")));
        return;
    }
    let config_file = paths.config_file();
    let saved = launcher_core::config::load(&config_file)
        .map_err(|e| e.to_string())
        .and_then(|loaded| {
            let mut config = loaded.config;
            config.games.entry(game_id).or_default().game_dir = Some(dir);
            config.save(&config_file).map_err(|e| e.to_string())
        });
    match saved {
        Ok(()) => state.start_scan(ctx),
        Err(e) => state.notice = Some(Notice::Error(format!("Réglage non enregistré : {e}"))),
    }
}

// ---------------------------------------------------------------------------
// Dessin.
// ---------------------------------------------------------------------------

/// `error` : erreur du worker à montrer (la partie hébergée s'est arrêtée…).
pub fn show(
    ui: &mut Ui,
    state: &mut LibraryState,
    shared: &Shared,
    error: Option<&str>,
) -> Option<Page> {
    let ctx = ui.ctx().clone();
    // Première ouverture : on lance la détection.
    if state.scan.is_none() && state.scanning.is_none() {
        state.start_scan(&ctx);
    }
    state.poll(&ctx);
    poll_registry(state, &ctx);
    prefill(&mut state.form, shared);

    super::page(ui, |ui| {
        header(ui, state, &ctx);

        if let Err(e) = &state.catalog {
            widgets::error_banner(ui, &format!("Catalogue illisible : {e}"));
            return;
        }
        if let Err(e) = &state.paths {
            widgets::error_banner(ui, &format!("Dossiers de Raftup introuvables : {e}"));
            return;
        }
        if let Some(message) = error {
            widgets::error_banner(ui, message);
            ui.add_space(10.0);
        }
        match &state.notice {
            Some(Notice::Error(text)) => {
                widgets::error_banner(ui, text);
                ui.add_space(10.0);
            }
            Some(Notice::Info(text)) => {
                info_banner(ui, text);
                ui.add_space(10.0);
            }
            None => {}
        }

        let worlds_dir = state.paths.as_ref().ok().map(|p| p.worlds_dir(AP_VERSION));
        if state.game_count() == 0 {
            empty_state(ui);
            registry_panel::show(ui, &mut state.registry, worlds_dir.as_deref());
            return;
        }
        // Premier jeu sélectionné par défaut.
        if state.selected.is_none() {
            state.selected = state
                .catalog
                .as_ref()
                .ok()
                .and_then(|c| c.profiles.keys().next().cloned());
        }

        if ui.available_width() >= super::WIDE {
            ui.horizontal_top(|ui| {
                let list_width = ui.available_width() - 400.0;
                ui.vertical(|ui| {
                    ui.set_width(list_width);
                    grid(ui, state);
                    missing_game_note(ui);
                    registry_panel::show(ui, &mut state.registry, worlds_dir.as_deref());
                });
                ui.vertical(|ui| detail(ui, state, shared, &ctx));
            });
        } else {
            grid(ui, state);
            ui.add_space(12.0);
            detail(ui, state, shared, &ctx);
            missing_game_note(ui);
            registry_panel::show(ui, &mut state.registry, worlds_dir.as_deref());
        }
    });
    None
}

/// Lance la lecture du registre (une fois) et ajoute au catalogue les jeux qu'il apporte.
/// Rien ne se passe si le catalogue embarqué est illisible : l'écran affiche déjà l'erreur.
fn poll_registry(state: &mut LibraryState, ctx: &egui::Context) {
    let Ok(catalog) = &mut state.catalog else {
        return;
    };
    state.registry.start(ctx, catalog);
    let worlds_dir = state.paths.as_ref().ok().map(|p| p.worlds_dir(AP_VERSION));
    if state.registry.poll(catalog, worlds_dir.as_deref()) {
        // Nouveaux jeux : il faut les chercher sur le PC.
        if state.scanning.is_some() {
            state.rescan_pending = true;
        } else {
            state.start_scan(ctx);
        }
    }
}

/// Recopie l'adresse de la partie hébergée, une seule fois par partie.
fn prefill(form: &mut LaunchForm, shared: &Shared) {
    if let Some(status) = &shared.status {
        prefill_address(form, &status.address);
    }
}

/// La logique de `prefill`, sans l'état partagé (testée).
fn prefill_address(form: &mut LaunchForm, hosted: &str) {
    if form.prefilled_from.as_deref() == Some(hosted) {
        return;
    }
    // On ne remplace que ce que Raftup avait lui-même recopié (ou un champ vide) :
    // une adresse tapée par le joueur ou venue d'un lien d'invitation est gardée.
    let untouched = form.address.trim().is_empty()
        || form.prefilled_from.as_deref() == Some(form.address.as_str());
    if untouched {
        form.address = hosted.to_string();
    }
    form.prefilled_from = Some(hosted.to_string());
}

fn header(ui: &mut Ui, state: &mut LibraryState, ctx: &egui::Context) {
    let total = state.game_count();
    let ready = state.scan.as_ref().map_or(0, |scan| {
        scan.status
            .values()
            .filter(|s| matches!(s, Ok(InstallStatus::Installed)))
            .count()
    });
    let jeux = if total > 1 { "jeux" } else { "jeu" };
    let prets = if ready > 1 { "prêts" } else { "prêt" };
    let scanning = state.scanning.is_some();
    let mut refresh = false;
    let mut add_dir = false;
    let row = ui.horizontal(|ui| {
        ui.label(widgets::title("Bibliothèque", 28.0));
        ui.label(widgets::dim(format!("{total} {jeux} · {ready} {prets}")).size(14.0));
        // De droite à gauche : le premier bouton ajouté est le plus à droite.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if total == 0 {
                return;
            }
            add_dir = big_button(ui, "Ajouter un dossier de jeu", true, true, 0.0);
            if scanning {
                ui.spinner();
                ui.label(widgets::dim("Recherche des jeux…"));
            } else {
                refresh = big_button(ui, "Rechercher des jeux", false, true, 0.0);
            }
        });
    });
    if refresh {
        state.start_scan(ctx);
    }
    // Le dossier est demandé pour le jeu sélectionné (un profil = un exécutable attendu).
    if add_dir {
        state.pending_pick = state.selected.clone();
    }
    // Trait fin sous l'en-tête, sur toute la largeur, comme sur la maquette.
    let y = row.response.rect.bottom() + 12.0;
    ui.painter()
        .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, theme::BORDER));
    ui.add_space(32.0);
}

/// Bouton haut (40 px) des en-têtes et du panneau du jeu : bleu si `primary`, discret sinon.
/// `width` à 0 : largeur ajustée au texte.
fn big_button(ui: &mut Ui, text: &str, primary: bool, enabled: bool, width: f32) -> bool {
    let (color, fill, border) = if primary {
        (
            Color32::from_rgb(0x08, 0x12, 0x22),
            theme::ACCENT,
            theme::ACCENT,
        )
    } else {
        (theme::TEXT, theme::FIELD, theme::BORDER)
    };
    let button = egui::Button::new(
        RichText::new(text)
            .color(color)
            .strong()
            .family(theme::bold_family())
            .size(15.0),
    )
    .fill(fill)
    .stroke(Stroke::new(1.0, border))
    .corner_radius(CornerRadius::same(theme::RADIUS))
    .min_size(egui::vec2(width, 40.0));
    ui.add_enabled(enabled, button).clicked()
}

/// Catalogue vide : aucun profil n'est encore publié.
fn empty_state(ui: &mut Ui) {
    widgets::panel(ui, "Aucun jeu dans le catalogue", |ui| {
        ui.label(widgets::dim(
            "Les profils d'installation arrivent avec les prochaines versions de Raftup : \
             chaque jeu attend l'accord de l'auteur de son mod.",
        ));
        ui.add_space(4.0);
        ui.label(widgets::dim(
            "Vous pouvez déjà héberger une partie depuis « En cours » : chaque joueur installe son mod à la main.",
        ));
    });
}

/// Encadré en pointillés « Un jeu manque ? », comme sur la maquette.
fn missing_game_note(ui: &mut Ui) {
    ui.add_space(14.0);
    let response = Frame::new()
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new("Un jeu manque ?")
                    .strong()
                    .family(theme::bold_family())
                    .color(theme::TEXT),
            );
            ui.label(widgets::dim(
                "Raftup détecte les jeux installés via Steam et Epic. Pour une autre \
                 installation, ajoutez son dossier : Raftup vérifie que l'exécutable attendu \
                 s'y trouve avant de proposer le profil.",
            ));
        })
        .response;
    let r = response.rect;
    let points = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    ui.painter().add(egui::Shape::dashed_line(
        &points,
        Stroke::new(1.0, theme::BORDER),
        4.0,
        3.0,
    ));
}

/// Les cartes des jeux, en grille qui passe à la ligne.
fn grid(ui: &mut Ui, state: &mut LibraryState) {
    let Ok(catalog) = &state.catalog else { return };
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(22.0, 22.0);
        for profile in catalog.games() {
            let id = &profile.game.id;
            let found = state.scan.as_ref().and_then(|s| s.found.get(id));
            let status = state.scan.as_ref().and_then(|s| s.status.get(id));
            let view = view(profile, found, status);
            let selected = state.selected.as_deref() == Some(id.as_str());
            let busy = state.task.as_ref().filter(|t| &t.game_id == id);
            let from_registry = state.registry.is_from_registry(id);
            if card(ui, profile, &view, selected, busy, from_registry) {
                clicked = Some(id.clone());
            }
        }
    });
    if let Some(id) = clicked {
        if state.selected.as_ref() != Some(&id) {
            state.confirm_uninstall = false;
        }
        state.selected = Some(id);
    }
}

/// Couleur d'un état.
fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Ready => theme::TEXT_DIM,
        Tone::Accent => theme::ACCENT,
        Tone::Warning => theme::AMBER,
        Tone::Dim => theme::TEXT_DIM,
    }
}

/// Petite jaquette carrée de la fiche : l'aplat du jeu avec ses initiales (le titre
/// entier ne tient pas dans 64 px ; il est écrit juste à côté).
fn small_cover(ui: &mut Ui, size: f32, game_id: &str, name: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::cover_color(game_id),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format::initials(name),
        theme::title_font(size * 0.38),
        theme::TEXT,
    );
}

/// Une carte de jeu. Renvoie vrai au clic.
fn card(
    ui: &mut Ui,
    profile: &Profile,
    view: &GameView,
    selected: bool,
    busy: Option<&Task>,
    from_registry: bool,
) -> bool {
    const WIDTH: f32 = 150.0;
    // Jaquette en portrait (2:3), comme les couvertures de la maquette.
    const COVER: egui::Vec2 = egui::vec2(WIDTH, 210.0);
    let response = ui
        .vertical(|ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            // La « jaquette » : un aplat coloré avec le titre du jeu (SPEC § 7 : jamais de
            // visuel officiel sans autorisation). La teinte dépend de l'identifiant du jeu.
            let (rect, _) = ui.allocate_exact_size(COVER, Sense::hover());
            let painter = ui.painter();
            let radius = CornerRadius::same(theme::RADIUS);
            painter.rect_filled(rect, radius, theme::cover_color(&profile.game.id));
            // Le titre passe à la ligne dans la jaquette (marge de 12 px de chaque côté).
            let galley = painter.layout(
                profile.game.name.clone(),
                theme::title_font(22.0),
                theme::TEXT,
                rect.width() - 24.0,
            );
            // Centré dans la jaquette : on part du coin haut gauche du bloc de texte.
            let top_left = rect.center() - galley.size() / 2.0;
            painter.galley(top_left, galley, theme::TEXT);
            // Jeu sélectionné : liseré bleu de 2 px autour de la jaquette seulement.
            let stroke = if selected {
                Stroke::new(2.0, theme::ACCENT)
            } else {
                Stroke::new(1.0, theme::BORDER)
            };
            painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Outside);
            // Barre sous la jaquette : pleine si le jeu est prêt, à moitié pendant une tâche.
            let fraction = match (busy, view.can_launch) {
                (Some(_), _) => 0.5,
                (None, true) => 1.0,
                (None, false) => 0.0,
            };
            widgets::bar(ui, WIDTH, fraction, theme::ACCENT);
            ui.label(
                RichText::new(&profile.game.name)
                    .strong()
                    .family(theme::bold_family())
                    .size(15.0)
                    .color(theme::TEXT),
            );
            let (text, color) = match busy {
                Some(task) => (format!("{}…", task.label), theme::ACCENT),
                None => (view.label.clone(), tone_color(view.tone)),
            };
            ui.label(RichText::new(text).color(color).size(13.0));
            // Origine : un jeu venu du registre signé le dit (le catalogue embarqué non).
            if from_registry {
                ui.label(widgets::caps("Registre").color(theme::ITEM));
            }
        })
        .response
        // `interact` rend toute la carte cliquable, pas seulement son texte.
        .interact(Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Panneau de droite : le jeu sélectionné.
fn detail(ui: &mut Ui, state: &mut LibraryState, shared: &Shared, ctx: &egui::Context) {
    let Ok(catalog) = &state.catalog else { return };
    let Some(profile) = state.selected.as_ref().and_then(|id| catalog.get(id)) else {
        return;
    };
    // Copie du profil : les boutons modifient `state` plus bas, on ne peut pas garder
    // en même temps un emprunt sur `state.catalog`.
    let profile = profile.clone();
    let id = profile.game.id.clone();
    let found = state.scan.as_ref().and_then(|s| s.found.get(&id)).cloned();
    let status = state.scan.as_ref().and_then(|s| s.status.get(&id)).cloned();
    let view = view(&profile, found.as_ref(), status.as_ref());
    let busy = state.task.is_some();

    widgets::panel(ui, "", |ui| {
        ui.horizontal(|ui| {
            small_cover(ui, 64.0, &profile.game.id, &profile.game.name);
            ui.vertical(|ui| {
                ui.label(widgets::title(&profile.game.name, 26.0));
                ui.label(RichText::new(&view.label).color(tone_color(view.tone)));
                let origin = if state.registry.is_from_registry(&id) {
                    "Profil du registre signé"
                } else {
                    "Profil du catalogue de Raftup"
                };
                ui.label(widgets::dim(origin).size(12.0));
            });
        });
        ui.add_space(10.0);

        // Boutons d'action : « Lancer » en bleu quand le jeu est prêt ; sinon c'est
        // l'installation qui devient l'action principale.
        ui.horizontal(|ui| {
            let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            if let (true, Some(game)) = (view.can_launch, &found) {
                let ready = launch_mode(&state.form) != LaunchMode::Incomplete;
                if big_button(ui, "Lancer le jeu", true, ready && !busy, half) {
                    let dir = game.install_dir.clone();
                    state.start_launch(ctx, &profile, &dir);
                }
            }
            if let (true, Some(game)) = (view.can_install, &found) {
                let primary = !view.can_launch;
                let clicked = big_button(ui, view.install_label, primary, !busy, half);
                let auto = std::mem::take(&mut state.dev_auto_install);
                if (clicked || auto) && !busy {
                    let dir = game.install_dir.clone();
                    let force = matches!(status, Some(Ok(InstallStatus::InProgress)));
                    state.start_install(ctx, &profile, dir, force);
                }
            }
        });
        if view.can_launch {
            let hint = match launch_mode(&state.form) {
                LaunchMode::Connect(_) => None,
                LaunchMode::Offline => {
                    Some("Sans adresse ni slot, le jeu démarre sans se connecter à une partie.")
                }
                LaunchMode::Incomplete => Some(
                    "Pour vous connecter, renseignez l'adresse et le slot sous « Se connecter à la main ».",
                ),
            };
            if let Some(hint) = hint {
                ui.label(widgets::dim(hint).size(12.0));
            }
        }
        if let Some(task) = &state.task {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spinner();
                let text = match &task.progress {
                    Some((step, bytes)) => {
                        format!("{} : {step}, {}", task.label, megabytes(*bytes))
                    }
                    None => format!("{}…", task.label),
                };
                ui.label(widgets::dim(text));
            });
        }

        // Emplacement.
        ui.add_space(12.0);
        ui.label(widgets::caps("Emplacement"));
        match &found {
            Some(game) => {
                let store = match game.store {
                    Store::Steam => "Détecté via Steam",
                    Store::Epic => "Détecté via Epic",
                    Store::Manual => "Dossier choisi à la main",
                };
                ui.label(RichText::new(store).color(theme::TEXT));
                code_box(ui, &game.install_dir.display().to_string());
            }
            None => {
                ui.label(widgets::dim("Jeu non trouvé sur ce PC."));
            }
        }

        // Profil d'installation.
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(widgets::caps("Profil d'installation"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(widgets::dim("versions épinglées").size(12.0));
            });
        });
        let installed = matches!(
            status,
            Some(Ok(InstallStatus::Installed | InstallStatus::UpdateAvailable))
        );
        for step in &profile.steps {
            step_row(ui, step, installed);
        }

        // Connexion : d'habitude remplie par une invitation (écran « Rejoindre ») ; la
        // saisie à la main est repliée.
        if view.can_launch {
            ui.add_space(12.0);
            ui.label(widgets::caps("Connexion à une partie"));
            ui.label(widgets::dim(connection_summary(&state.form)).size(13.0));
            // `open_now` est copié à part : `state.form` est emprunté par la fermeture.
            let mut open_now = state.form.open_now;
            widgets::advanced(
                ui,
                "library_connect_by_hand",
                "Se connecter à la main",
                &mut open_now,
                |ui| connection_form(ui, &mut state.form, &profile, shared),
            );
            state.form.open_now = open_now;
        }

        // Désinstallation, en deux clics.
        if view.can_uninstall {
            ui.add_space(12.0);
            ui.separator();
            if state.confirm_uninstall {
                ui.label(widgets::dim(
                    "Les fichiers posés par Raftup seront retirés et les originaux restaurés.",
                ));
                ui.horizontal(|ui| {
                    if widgets::danger_button(ui, "Confirmer la désinstallation") && !busy {
                        state.start_uninstall(ctx, id.clone());
                    }
                    if widgets::ghost_button(ui, "Annuler") {
                        state.confirm_uninstall = false;
                    }
                });
            } else if widgets::danger_button(ui, "Désinstaller le mod") && !busy {
                state.confirm_uninstall = true;
            }
        }
    });
}

/// Une étape du profil : nom, type et version, état.
fn step_row(ui: &mut Ui, step: &launcher_core::profile::Step, installed: bool) {
    use launcher_core::profile::StepKind;
    let kind = match step.kind {
        StepKind::ModLoader => "Mod loader",
        StepKind::Mod => "Mod client",
        StepKind::Apworld => "World",
    };
    Frame::new()
        .fill(theme::FIELD)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        RichText::new(&step.name)
                            .strong()
                            .family(theme::bold_family())
                            .color(theme::TEXT),
                    );
                    ui.label(widgets::dim(format!("{kind} · {}", step.version)).size(12.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let text = if installed {
                        "Installé · hash vérifié"
                    } else {
                        "À installer"
                    };
                    ui.label(widgets::dim(text).size(12.0));
                });
            });
        });
}

/// Une ligne qui dit à quelle partie « Lancer le jeu » connectera.
fn connection_summary(form: &LaunchForm) -> String {
    match launch_mode(form) {
        LaunchMode::Connect(c) => format!("Slot {} sur {}", c.slot, c.address),
        LaunchMode::Offline => {
            "Aucune partie choisie : ouvrez un lien d'invitation (« Invitations »).".to_string()
        }
        LaunchMode::Incomplete => {
            "Connexion incomplète : il manque l'adresse ou le slot.".to_string()
        }
    }
}

/// Adresse, slot (avec la liste des slots de la partie hébergée) et mot de passe.
fn connection_form(ui: &mut Ui, form: &mut LaunchForm, profile: &Profile, shared: &Shared) {
    egui::Grid::new("library_connect")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label(widgets::dim("Adresse"));
            ui.add(egui::TextEdit::singleline(&mut form.address).hint_text("ws://hote:38281"));
            ui.end_row();

            ui.label(widgets::dim("Slot"));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut form.slot).desired_width(140.0));
                // Les slots de ce jeu dans la partie hébergée, pour ne pas taper le nom.
                if let Some(status) = &shared.status {
                    let slots: Vec<&str> = status
                        .seed
                        .slots
                        .iter()
                        .filter(|s| !s.is_group && s.game == profile.game.ap_game)
                        .map(|s| s.name.as_str())
                        .collect();
                    if !slots.is_empty() {
                        egui::ComboBox::from_id_salt("library_slot")
                            .selected_text("Choisir")
                            .show_ui(ui, |ui| {
                                for name in slots {
                                    if ui.selectable_label(form.slot == name, name).clicked() {
                                        form.slot = name.to_string();
                                    }
                                }
                            });
                    }
                }
            });
            ui.end_row();

            ui.label(widgets::dim("Mot de passe"));
            ui.add(egui::TextEdit::singleline(&mut form.password).password(true));
            ui.end_row();
        });
    if let launcher_core::profile::ConnectMethod::Manual { .. } = profile.launch.connect {
        ui.label(widgets::dim(
            "Saisie dans le jeu : Raftup copie l'adresse et le slot dans le presse-papiers.",
        ));
    }
}

/// Chemin en police monospace dans un cadre sombre.
fn code_box(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(theme::LOG_BG)
        .stroke(Stroke::new(1.0, theme::BORDER))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(widgets::mono(text, 12.0).color(theme::TEXT));
        });
}

/// Message vert (action réussie).
fn info_banner(ui: &mut Ui, text: &str) {
    Frame::new()
        .fill(theme::GREEN_BG)
        .stroke(Stroke::new(1.0, theme::GREEN))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(theme::TEXT));
        });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn tache_interrompue_accordee() {
        assert_eq!(
            interrupted("Installation"),
            "Installation interrompue sans réponse."
        );
        assert_eq!(
            interrupted("Lancement"),
            "Lancement interrompu sans réponse."
        );
    }

    /// Profil de démonstration : il n'existe que dans les tests, jamais dans `profiles/`.
    const DEMO: &str = r#"
format_version = 1

[game]
id = "demo"
name = "Jeu de démo"
ap_game = "Demo Game"
status = "available"

[game.detect]
expected_exe = "Demo.exe"

[credits]
authors = ["Quelqu'un"]
license = "MIT"

[[steps]]
id = "le-mod"
kind = "mod"
name = "Le mod"
version = "1.0.0"
source = { url = "https://github.com/x/y/releases/download/v1/m.zip" }
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
install = { copy_to = "{game_dir}/Mods/m.dll" }

[launch]
exe = "{game_dir}/Demo.exe"
connect = { method = "cli_args", args = ["--connect", "{address}", "--name", "{slot}"] }
"#;

    fn catalog() -> Catalog {
        Catalog::from_texts(&[("demo.toml", DEMO)]).unwrap()
    }

    fn found(state: GameState) -> DetectedGame {
        DetectedGame {
            game_id: "demo".into(),
            store: Store::Manual,
            install_dir: PathBuf::from("C:/Jeux/Demo"),
            build_id: None,
            state,
        }
    }

    #[test]
    fn le_catalogue_embarque_se_charge() {
        // Vide tant qu'aucun profil réel n'est autorisé : l'écran affiche alors l'état vide.
        assert!(Catalog::embedded().is_ok());
    }

    #[test]
    fn jeu_absent_rien_n_est_possible() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let v = view(p, None, Some(&Ok(InstallStatus::NotInstalled)));
        assert_eq!(v.label, "Jeu non trouvé");
        assert!(!v.can_install && !v.can_launch && !v.can_uninstall);
    }

    #[test]
    fn jeu_trouve_sans_mod_on_peut_installer() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let g = found(GameState::Ready);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::NotInstalled)));
        assert_eq!(v.label, "Mod non installé");
        assert!(v.can_install && !v.can_launch && !v.can_uninstall);
        assert_eq!(v.install_label, "Installer");
    }

    #[test]
    fn jeu_pret_on_peut_lancer_et_desinstaller() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let g = found(GameState::Ready);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::Installed)));
        assert_eq!(v.label, "Prêt");
        assert!(v.can_launch && v.can_uninstall);
    }

    #[test]
    fn jeu_en_mise_a_jour_bloque_l_installation() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let g = found(GameState::Updating);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::NotInstalled)));
        assert!(!v.can_install);
        assert_eq!(v.tone, Tone::Warning);
    }

    #[test]
    fn installation_interrompue_propose_de_reparer() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let g = found(GameState::Ready);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::InProgress)));
        assert_eq!(v.install_label, "Réparer");
        assert!(v.can_uninstall && !v.can_launch);
    }

    #[test]
    fn mode_de_lancement() {
        // Rien de rempli (ou des espaces) : lancement hors partie.
        let mut form = LaunchForm {
            address: "  ".into(),
            ..LaunchForm::default()
        };
        assert_eq!(launch_mode(&form), LaunchMode::Offline);
        // Un seul des deux : on attend l'autre.
        form.address = "ws://localhost:38281".into();
        assert_eq!(launch_mode(&form), LaunchMode::Incomplete);
        form.address.clear();
        form.slot = "Alice".into();
        assert_eq!(launch_mode(&form), LaunchMode::Incomplete);
        // Les deux : connexion.
        form.address = "ws://localhost:38281".into();
        assert!(matches!(launch_mode(&form), LaunchMode::Connect(_)));
    }

    #[test]
    fn resume_de_la_connexion() {
        let mut form = LaunchForm::default();
        assert!(connection_summary(&form).contains("Aucune partie"));
        form.address = "ws://localhost:38281".into();
        assert!(connection_summary(&form).contains("incomplète"));
        form.slot = "Alice".into();
        assert_eq!(
            connection_summary(&form),
            "Slot Alice sur ws://localhost:38281"
        );
    }

    #[test]
    fn une_invitation_ouvre_la_saisie_a_la_main() {
        let mut state = LibraryState::new();
        assert!(!state.form.open_now);
        open_with_invite(
            &mut state,
            "slime_rancher",
            "wss://h.example:38281",
            "Alice",
        );
        assert!(state.form.open_now);
    }

    #[test]
    fn identifiants_obligatoires() {
        let mut form = LaunchForm::default();
        assert!(credentials(&form).is_none());
        form.address = " ws://localhost:38281 ".into();
        form.slot = "Alice".into();
        let c = credentials(&form).unwrap();
        assert_eq!(c.address, "ws://localhost:38281");
        assert_eq!(c.password, None);
    }

    #[test]
    fn scan_trouve_un_dossier_choisi_a_la_main() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::under(temp.path());
        let game_dir = temp.path().join("Demo");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(game_dir.join("Demo.exe"), b"").unwrap();
        let mut config = launcher_core::UserConfig::default();
        config.games.entry("demo".into()).or_default().game_dir = Some(game_dir.clone());
        config.save(&paths.config_file()).unwrap();

        let result = scan(&catalog(), &paths, &DetectionRoots::default());
        let game = result.found.get("demo").unwrap();
        assert_eq!(game.store, Store::Manual);
        assert_eq!(game.install_dir, game_dir);
        assert_eq!(
            result.status.get("demo"),
            Some(&Ok(InstallStatus::NotInstalled))
        );
    }

    #[test]
    fn plan_de_lancement_avec_le_slot_de_la_partie() {
        let catalog = catalog();
        let p = catalog.get("demo").unwrap();
        let form = LaunchForm {
            address: "ws://localhost:38281".into(),
            slot: "Alice".into(),
            ..LaunchForm::default()
        };
        let c = credentials(&form).unwrap();
        let plan = plan_launch(p, Path::new("C:/Jeux/Demo"), Path::new("w"), &c).unwrap();
        assert_eq!(
            plan.args,
            ["--connect", "ws://localhost:38281", "--name", "Alice"]
        );
    }

    #[test]
    fn catalogue_de_dev_lu_dans_un_dossier() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("demo.toml"), DEMO).unwrap();
        // Un fichier qui n'est pas un profil est ignoré.
        std::fs::write(temp.path().join("notes.txt"), "rien").unwrap();
        let catalog = dev_catalog(temp.path()).unwrap();
        assert_eq!(catalog.profiles.len(), 1);
        assert!(catalog.get("demo").is_some());
    }

    #[test]
    fn ouvrir_avec_une_invitation_remplit_le_formulaire() {
        let mut state = LibraryState::new();
        state.form.password = "ancien".into();
        open_with_invite(&mut state, "demo", "wss://h.example:38281", "Alice");
        assert_eq!(state.selected.as_deref(), Some("demo"));
        assert_eq!(state.form.address, "wss://h.example:38281");
        assert_eq!(state.form.slot, "Alice");
        assert!(state.form.password.is_empty());
    }

    /// La partie hébergée ne doit pas écraser l'adresse d'une invitation (ni ce que le
    /// joueur a tapé) ; elle remplit un champ vide et suit ses propres changements.
    #[test]
    fn adresse_hebergee_n_ecrase_pas_une_invitation() {
        let mut form = LaunchForm::default();
        prefill_address(&mut form, "ws://localhost:38281");
        assert_eq!(form.address, "ws://localhost:38281");
        // Nouvelle partie hébergée : l'adresse recopiée suit.
        prefill_address(&mut form, "ws://localhost:38282");
        assert_eq!(form.address, "ws://localhost:38282");
        // Une invitation remplace l'adresse ; la partie hébergée ne la reprend pas.
        form.address = "wss://ami.example:38281".into();
        prefill_address(&mut form, "ws://localhost:38282");
        prefill_address(&mut form, "ws://localhost:38283");
        assert_eq!(form.address, "wss://ami.example:38281");
    }

    #[test]
    fn taille_lisible() {
        assert_eq!(megabytes(12_345_678), "12,3 Mo");
    }
}
