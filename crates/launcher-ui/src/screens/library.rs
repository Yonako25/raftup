//! Écran « Bibliothèque » : les jeux du catalogue, leur dossier, l'état des mods,
//! et les boutons Installer / Lancer / Désinstaller (ADR 0015). Les mondes natifs de
//! `mondes-natifs/` s'y affichent aussi, comme des jeux, avec leur fiche (ADR 0046,
//! module `native_panel`).
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

use launcher_core::art::{ArtKind, ArtSources};

use crate::toasts::ToastKind;

use super::native_panel::{self, NativeState};
use super::registry_panel::{self, RegistryPanel, Targets};
use crate::app::Page;
use crate::art_ui::{self, Artwork};
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
    /// Jeu dont la fiche est affichée, et depuis quand : la fiche d'un autre jeu
    /// apparaît en fondu.
    detail_since: (Option<String>, std::time::Instant),
    /// Fondus actifs (faux dans les tests, pour des captures nettes).
    fade: bool,
    /// Messages éphémères à montrer (fin d'une installation, d'un lancement…) ; l'app
    /// les reprend à chaque image avec [`LibraryState::take_toasts`].
    toasts_out: Vec<(String, ToastKind)>,
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
    /// Mondes natifs de `mondes-natifs/` (ADR 0046).
    native: NativeState,
    /// Illustrations des jeux, lues sur le disque (ADR 0051).
    pub(crate) art: Artwork,
    /// Vrai une fois toutes les images du catalogue demandées (au démarrage).
    art_preloaded: bool,
}

impl LibraryState {
    /// Charge le catalogue embarqué et les dossiers de Raftup. Une erreur est gardée
    /// en texte pour être affichée : l'écran s'ouvre quand même.
    pub fn new() -> LibraryState {
        let catalog = load_catalog();
        let paths = load_paths();
        let art = local_artwork(&catalog, &paths);
        LibraryState {
            catalog,
            paths,
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
            native: NativeState::new(),
            art,
            art_preloaded: false,
            detail_since: (None, std::time::Instant::now()),
            fade: true,
            toasts_out: Vec::new(),
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
            native: NativeState::new(),
            art: Artwork::new(),
            art_preloaded: false,
            detail_since: (None, std::time::Instant::now()),
            // Pas de fondu dans les tests : les images de référence doivent être nettes.
            fade: false,
            toasts_out: Vec::new(),
        }
    }

    /// Dossier des mondes natifs, si les dossiers de Raftup sont connus.
    fn native_dir(&self) -> Option<PathBuf> {
        self.paths.as_ref().ok().map(Paths::native_worlds_dir)
    }

    /// Les jeux lancés depuis la Bibliothèque qui tournent encore.
    pub fn running_games(&self) -> Vec<String> {
        self.running.names()
    }

    /// Nombre de jeux dans la Bibliothèque (compteur de la barre latérale).
    pub fn game_count(&self) -> usize {
        self.library_ids().len()
    }

    /// Vrai si le jeu est dans la Bibliothèque : trouvé sur ce PC, ou avec des mods posés
    /// par Raftup (reçu d'installation). Les autres jeux du catalogue ne sont montrés que
    /// dans l'écran « Catalogue », qui est en lecture seule.
    ///
    /// Attention : la détection range un état des mods pour **chaque** jeu du catalogue,
    /// « non installé » compris ; seul un autre état compte comme « sur ce PC ».
    pub fn in_library(&self, game_id: &str) -> bool {
        let on_pc = self.scan.as_ref().is_some_and(|scan| {
            let has_mods = !matches!(
                scan.status.get(game_id),
                None | Some(Ok(InstallStatus::NotInstalled))
            );
            scan.found.contains_key(game_id) || has_mods
        });
        on_pc
    }

    /// Les jeux de la Bibliothèque, dans l'ordre du catalogue.
    pub fn library_ids(&self) -> Vec<String> {
        let Ok(catalog) = &self.catalog else {
            return Vec::new();
        };
        catalog
            .profiles
            .keys()
            .filter(|id| self.in_library(id))
            .cloned()
            .collect()
    }

    /// Le catalogue, s'il s'est chargé (l'écran « Rejoindre » y cherche le jeu du lien).
    pub fn catalog(&self) -> Option<&Catalog> {
        self.catalog.as_ref().ok()
    }

    /// La dernière détection des jeux, si elle a abouti.
    pub fn scan_result(&self) -> Option<&Scan> {
        self.scan.as_ref()
    }

    /// Les dossiers de Raftup, s'ils sont connus (écran « Paramètres »).
    pub fn paths(&self) -> Option<&Paths> {
        self.paths.as_ref().ok()
    }

    /// Hors de la Bibliothèque (Accueil, assistant, « Lancer et connecter ») : lance la
    /// détection des jeux si elle n'a jamais été faite, et range les messages des
    /// tâches en cours. Sans danger si la Bibliothèque le fait aussi dans la même image.
    pub fn background(&mut self, ctx: &egui::Context) {
        if self.scan.is_none() && self.scanning.is_none() {
            self.start_scan(ctx);
        }
        self.poll(ctx);
        // Au démarrage, toutes les images du catalogue sont demandées d'un coup : elles
        // sont prêtes avant qu'on ouvre la Bibliothèque ou le Catalogue.
        if !std::mem::replace(&mut self.art_preloaded, true) {
            let ids: Vec<String> = self.summaries().into_iter().map(|g| g.id).collect();
            for id in ids {
                for kind in ArtKind::ALL {
                    let _ = self.art.texture(ctx, &id, kind);
                }
            }
        }
        // Images lues : rangées quel que soit l'écran affiché.
        self.art.poll(ctx);
        // Le registre aussi : ses mondes sont montrés dans le Catalogue.
        poll_registry(self, ctx);
    }

    /// Illustration d'un jeu (ADR 0051), par son identifiant : la jaquette (`Cover`)
    /// ou l'image large (`Hero`). `None` tant qu'elle n'est pas lue, ou s'il n'y en a
    /// pas : l'écran dessine alors sa propre jaquette.
    pub fn art_for(
        &mut self,
        ctx: &egui::Context,
        game_id: &str,
        kind: ArtKind,
    ) -> Option<egui::TextureHandle> {
        self.art.texture(ctx, game_id, kind)
    }

    /// Comme [`LibraryState::art_for`], par le nom du jeu chez Archipelago (le `game`
    /// d'un slot de la partie hébergée). `None` pour un jeu hors catalogue.
    pub fn art_for_ap_game(
        &mut self,
        ctx: &egui::Context,
        ap_game: &str,
        kind: ArtKind,
    ) -> Option<egui::TextureHandle> {
        let id = self
            .catalog
            .as_ref()
            .ok()?
            .games()
            .find(|p| p.game.ap_game == ap_game)?
            .game
            .id
            .clone();
        self.art_for(ctx, &id, kind)
    }

    /// Badge d'un jeu du catalogue : celui de son monde natif s'il y en a un d'installé
    /// (par le nom du jeu chez Archipelago). Sinon aucun badge : le monde d'Archipelago est
    /// le cas normal, l'écrire sur chaque jaquette ne distinguait rien.
    pub fn pin_for(&self, ap_game: &str) -> Option<WorldPin> {
        self.native
            .worlds()
            .iter()
            .find(|world| world.game == ap_game)
            .map(world_pin)
    }

    /// Une ligne par jeu du catalogue, dans l'ordre du catalogue : ce que l'Accueil et
    /// l'écran « Catalogue » montrent, sans redessiner la Bibliothèque.
    pub fn summaries(&self) -> Vec<GameSummary> {
        let Ok(catalog) = &self.catalog else {
            return Vec::new();
        };
        catalog
            .games()
            .map(|profile| {
                let id = &profile.game.id;
                let found = self.scan.as_ref().and_then(|s| s.found.get(id));
                let status = self.scan.as_ref().and_then(|s| s.status.get(id));
                let view = view(profile, found, status);
                GameSummary {
                    id: id.clone(),
                    name: profile.game.name.clone(),
                    ap_game: profile.game.ap_game.clone(),
                    authors: profile.credits.authors.clone(),
                    label: view.label,
                    tone: view.tone,
                    ready: view.can_launch,
                    detected: found.is_some(),
                    from_registry: self.registry.is_from_registry(id),
                    trust: self.registry.profile_trust(id),
                }
            })
            .collect()
    }

    /// Sélectionne un jeu : la Bibliothèque s'ouvrira sur sa fiche.
    pub fn select(&mut self, game_id: &str) {
        if self.selected.as_deref() != Some(game_id) {
            self.confirm_uninstall = false;
            self.notice = None;
        }
        self.selected = Some(game_id.to_string());
        self.native.selected = None;
    }
}

/// Ce que les autres écrans savent d'un jeu du catalogue (voir [`LibraryState::summaries`]).
#[derive(Debug, Clone, PartialEq)]
pub struct GameSummary {
    pub id: String,
    pub name: String,
    /// Nom du jeu dans Archipelago (celui des seeds et des liens d'invitation).
    pub ap_game: String,
    pub authors: Vec<String>,
    /// État affiché (« Prêt à jouer », « Non détecté »…).
    pub label: String,
    pub tone: Tone,
    /// Jeu détecté et mods installés : « Lancer » est possible.
    pub ready: bool,
    pub detected: bool,
    /// Profil venu du registre signé (ADR 0032), pas du catalogue embarqué.
    pub from_registry: bool,
    /// Niveau de confiance du profil (ADR 0045), pour un profil du registre seulement.
    pub trust: Option<launcher_core::registry::Trust>,
}

/// Ouvre la fiche d'un jeu avec le formulaire de connexion pré-rempli (écran « Rejoindre »).
/// Le mot de passe n'est jamais dans un lien : on le vide pour que le joueur le saisisse.
pub fn open_with_invite(state: &mut LibraryState, game_id: &str, address: &str, slot: &str) {
    state.selected = Some(game_id.to_string());
    state.native.selected = None;
    state.confirm_uninstall = false;
    state.notice = None;
    state.form.address = address.to_string();
    state.form.slot = slot.to_string();
    state.form.password.clear();
    // Le joueur doit voir ce que l'invitation a rempli (et taper le mot de passe).
    state.form.open_now = true;
}

/// Ce qu'a donné « Lancer et connecter » ([`launch_into`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchInto {
    /// Le jeu démarre, connecté à la partie.
    Launched,
    /// La partie a un mot de passe : la fiche est ouverte, le joueur le tape.
    NeedsPassword,
    /// Le jeu n'est pas prêt (pas détecté, mods absents) : la fiche est ouverte.
    NotReady,
    /// Le jeu n'est pas dans le catalogue de Raftup : rien à lancer d'ici.
    OutsideCatalog,
}

/// « Lancer et connecter » (écran « Partie en cours ») : lance le jeu `ap_game` avec
/// l'adresse et le slot donnés. Quand ce n'est pas possible tout de suite, la fiche du
/// jeu est préparée (adresse et slot remplis) pour que le joueur finisse à la main.
/// Le mot de passe n'est jamais gardé par Raftup : s'il en faut un, le joueur le tape.
pub fn launch_into(
    state: &mut LibraryState,
    ctx: &egui::Context,
    ap_game: &str,
    address: &str,
    slot: &str,
    needs_password: bool,
) -> LaunchInto {
    let Some(game) = state.summaries().into_iter().find(|g| g.ap_game == ap_game) else {
        return LaunchInto::OutsideCatalog;
    };
    open_with_invite(state, &game.id, address, slot);
    if !game.ready {
        return LaunchInto::NotReady;
    }
    if needs_password {
        return LaunchInto::NeedsPassword;
    }
    let dir = state
        .scan
        .as_ref()
        .and_then(|s| s.found.get(&game.id))
        .map(|found| found.install_dir.clone());
    // `cloned` : copie du profil, pour pouvoir ensuite modifier `state`.
    let profile = state
        .catalog
        .as_ref()
        .ok()
        .and_then(|c| c.get(&game.id))
        .cloned();
    let (Some(dir), Some(profile)) = (dir, profile) else {
        return LaunchInto::NotReady;
    };
    // Tout est rempli : inutile d'ouvrir « Se connecter à la main ».
    state.form.open_now = false;
    state.start_launch(ctx, &profile, &dir);
    LaunchInto::Launched
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

/// Illustrations lues sur ce PC (ADR 0051) : images du créateur installées dans les
/// dossiers de Raftup, sinon celles que Steam garde pour chaque jeu du catalogue.
fn local_artwork(catalog: &Result<Catalog, String>, paths: &Result<Paths, String>) -> Artwork {
    let appids = catalog
        .as_ref()
        .map(|c| {
            c.games()
                .filter_map(|p| Some((p.game.id.clone(), p.game.detect.steam_appid?)))
                .collect()
        })
        .unwrap_or_default();
    let sources = ArtSources {
        paths: paths.as_ref().ok().cloned(),
        steam_dir: DetectionRoots::from_system().steam_dir,
    };
    Artwork::local(sources, appids)
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

/// Vrai pour un profil jamais essayé avec le vrai jeu (ADR 0049).
pub fn is_untested(profile: &Profile) -> bool {
    profile.game.status == launcher_core::profile::GameStatus::Untested
}

/// Message de la fiche d'un profil « non testé » (ADR 0049).
pub const UNTESTED_NOTE: &str =
    "Profil non testé : personne ne l'a encore essayé avec le vrai jeu. \
     L'installation se défait avec « Désinstaller le mod ».";

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
        (None, _) => ("Pas sur ce PC".to_string(), Tone::Dim),
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
                        self.toasts_out.push(toast_of(&result));
                        self.notice = Some(match result {
                            Ok(text) => Notice::Info(text),
                            Err(text) => Notice::Error(text),
                        });
                        finished = true;
                        break;
                    }
                    Ok(TaskMessage::Launched(result)) => {
                        self.toasts_out.push(match &result {
                            Ok(_) => ("Jeu lancé.".to_string(), ToastKind::Success),
                            Err(text) => (text.clone(), ToastKind::Error),
                        });
                        self.notice = Some(match result {
                            Ok(Some(instructions)) => {
                                // Connexion à la main : on copie l'adresse et le slot.
                                ctx.copy_text(format!("{}\n{}", self.form.address, self.form.slot));
                                Notice::Info(format!(
                                    "Jeu lancé. {instructions} (adresse et nom de joueur copiés dans le presse-papiers)"
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
            // Les images du créateur viennent peut-être d'être installées : relues.
            if let Some(task) = self.task.take() {
                self.art.forget(&task.game_id);
            }
            // Le reçu a pu changer : on relit l'état des jeux.
            self.start_scan(ctx);
        }
    }

    /// Les messages éphémères arrivés depuis la dernière image (la liste est vidée).
    pub fn take_toasts(&mut self) -> Vec<(String, ToastKind)> {
        // `mem::take` rend la liste et laisse une liste vide à sa place.
        std::mem::take(&mut self.toasts_out)
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
    state.art.poll(&ctx);
    if let Some(dir) = state.native_dir() {
        state.native.ensure_loaded(&dir);
    }
    poll_registry(state, &ctx);
    prefill(&mut state.form, shared);
    // Monde natif désinstallé à l'image précédente : sa fiche a disparu, le message
    // passe dans le bandeau de l'écran.
    if let Some(message) = native_panel::take_notice(&mut state.native) {
        state.notice = Some(match message {
            Ok(text) => Notice::Info(text),
            Err(text) => Notice::Error(text),
        });
    }

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

        if state.game_count() == 0 && state.native.count() == 0 {
            empty_state(ui, state.scanning.is_some() || state.scan.is_none());
            return;
        }
        // Premier jeu sélectionné par défaut (ou le premier monde natif s'il n'y a pas de
        // jeu au catalogue), sauf si un monde natif est déjà sélectionné.
        if state.selected.is_none() && state.native.selected.is_none() {
            state.selected = state.library_ids().into_iter().next();
            if state.selected.is_none() {
                let first = state.native.worlds().first().map(|w| w.key.clone());
                if let Some(key) = first {
                    state.native.select(&key);
                }
            }
        }

        if ui.available_width() >= super::WIDE {
            ui.horizontal_top(|ui| {
                let list_width = ui.available_width() - 400.0;
                ui.vertical(|ui| {
                    ui.set_width(list_width);
                    grid(ui, state);
                });
                ui.vertical(|ui| detail(ui, state, shared, &ctx));
            });
        } else {
            grid(ui, state);
            ui.add_space(12.0);
            detail(ui, state, shared, &ctx);
        }
    });
    None
}

/// Bas de la colonne : paquets du registre, puis la conversion d'un apworld (repliée).
/// Mondes du registre et conversion d'un apworld (ADR 0045, 0046).
///
/// **Masqués** depuis le 22 novembre 2026, à la demande de l'utilisatrice : ni la
/// Bibliothèque ni le Catalogue ne les affichent. Le code et ses tests sont gardés pour
/// les remettre plus tard (par exemple dans Paramètres) ; il suffira d'appeler cette
/// fonction depuis un écran et de retirer l'`allow` ci-dessous.
#[allow(dead_code)]
pub(crate) fn registry_and_conversion(ui: &mut Ui, state: &mut LibraryState) {
    let Ok(paths) = state.paths.clone() else {
        return;
    };
    let worlds_dir = paths.worlds_dir(AP_VERSION);
    let native_dir = paths.native_worlds_dir();
    let targets = Targets {
        worlds_dir: &worlds_dir,
        native_dir: &native_dir,
    };
    // Mondes natifs illisibles : une ligne discrète, la raison au survol.
    let rejected = state.native.rejected();
    if !rejected.is_empty() {
        let reasons: Vec<String> = rejected
            .iter()
            .map(|(path, reason)| format!("{} : {reason}", path.display()))
            .collect();
        ui.add_space(10.0);
        ui.label(
            widgets::dim(format!(
                "{} monde(s) natif(s) écarté(s) dans mondes-natifs.",
                rejected.len()
            ))
            .size(12.0)
            .color(theme::AMBER),
        )
        .on_hover_text(reasons.join("\n"));
    }
    registry_panel::show(ui, &mut state.registry, Some(targets));
    let mut converted = false;
    native_panel::conversion_block(ui, &mut state.native, &paths, &worlds_dir, &mut converted);
    if converted {
        // La liste est déjà relue (le monde apparaît dans la grille) ; les marques
        // « Installé » du registre suivent.
        state.registry.refresh_installed(targets);
    }
}

/// Lance la lecture du registre (une fois) et ajoute au catalogue les jeux qu'il apporte.
/// Rien ne se passe si le catalogue embarqué est illisible : l'écran affiche déjà l'erreur.
fn poll_registry(state: &mut LibraryState, ctx: &egui::Context) {
    let Ok(catalog) = &mut state.catalog else {
        return;
    };
    state.registry.start(ctx, catalog);
    let worlds_dir = state.paths.as_ref().ok().map(|p| p.worlds_dir(AP_VERSION));
    let native_dir = state.paths.as_ref().ok().map(Paths::native_worlds_dir);
    // `zip` des deux `Option` : des cibles seulement si les deux dossiers sont connus.
    let targets =
        worlds_dir
            .as_deref()
            .zip(native_dir.as_deref())
            .map(|(worlds_dir, native_dir)| Targets {
                worlds_dir,
                native_dir,
            });
    let added = state.registry.poll(catalog, targets);
    // Un monde natif posé par le registre : on relit `mondes-natifs/`.
    if state.registry.take_native_changed() {
        if let Some(dir) = &native_dir {
            state.native.refresh(dir);
        }
    }
    if added {
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
    let mut summary = format!("{total} {jeux} · {ready} {prets}");
    let natives = state.native.count();
    if natives > 0 {
        let mondes = if natives > 1 {
            "mondes natifs"
        } else {
            "monde natif"
        };
        summary.push_str(&format!(" · {natives} {mondes}"));
    }
    let scanning = state.scanning.is_some();
    let mut refresh = false;
    let mut add_dir = false;
    let row = ui.horizontal(|ui| {
        ui.label(widgets::title("Bibliothèque", 28.0));
        ui.label(widgets::dim(summary).size(14.0));
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
fn empty_state(ui: &mut Ui, scanning: bool) {
    if scanning {
        widgets::panel(ui, "Recherche de tes jeux…", |ui| {
            ui.label(widgets::dim(
                "Raftup cherche les jeux installés via Steam et Epic.",
            ));
        });
        return;
    }
    widgets::panel(ui, "Aucun jeu trouvé sur ce PC", |ui| {
        ui.label(widgets::dim(
            "Raftup n'a trouvé aucun jeu du catalogue installé via Steam ou Epic.",
        ));
        ui.add_space(4.0);
        ui.label(widgets::dim(
            "Ouvre « Parcourir » dans le Catalogue pour voir les jeux pris en charge et en ajouter un.",
        ));
    });
}

/// Les cartes des jeux, en grille qui passe à la ligne.
/// Ce qu'un clic sur une carte de la grille sélectionne.
enum CardTarget {
    Game(String),
    Native(String),
}

/// Nombre de cartes par rangée dans une largeur donnée (au moins une).
pub(super) fn cards_per_row(width: f32) -> usize {
    let per_row = ((width + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).floor();
    // `as usize` sur un nombre négatif ou NaN donne 0 : `max(1)` garde une carte.
    (per_row as usize).max(1)
}

fn grid(ui: &mut Ui, state: &mut LibraryState) {
    let Ok(catalog) = &state.catalog else { return };
    let mut clicked = None;
    // D'abord la liste des cartes, ensuite le dessin rangée par rangée : egui ne sait
    // pas faire passer à la ligne un bloc (`ui.vertical`) dont il ne connaît pas encore
    // la taille, et une seule rangée débordait à droite (vu sur les captures, 9 jeux).
    let mut cards: Vec<(CardLook<'_>, bool, CardTarget)> = Vec::new();
    for profile in catalog.games() {
        let id = &profile.game.id;
        // Seuls les jeux de ce PC (ou ajoutés depuis le Catalogue) ont leur carte.
        if !state.in_library(id) {
            continue;
        }
        let found = state.scan.as_ref().and_then(|s| s.found.get(id));
        let status = state.scan.as_ref().and_then(|s| s.status.get(id));
        let view = view(profile, found, status);
        let selected = state.selected.as_deref() == Some(id.as_str());
        let busy = state.task.as_ref().filter(|t| &t.game_id == id);
        let from_registry = state.registry.is_from_registry(id);
        // Jaquette du créateur ou de Steam si elle est prête, sinon la jaquette dessinée.
        let cover = state.art.texture(ui.ctx(), id, ArtKind::Cover);
        let mut look = card_look(profile, &view, busy, from_registry, cover);
        look.pin = state.pin_for(&profile.game.ap_game);
        cards.push((look, selected, CardTarget::Game(id.clone())));
    }
    // Les mondes natifs, après les jeux du catalogue (ADR 0046).
    for world in state.native.worlds() {
        let selected = state.native.selected.as_deref() == Some(world.key.as_str());
        let look = CardLook {
            cover_id: &world.key,
            name: &world.game,
            status: native_panel::card_status(world),
            status_color: theme::TEXT_DIM,
            // Un monde natif est prêt dès qu'il est lu : rien à installer dans un jeu.
            fraction: 1.0,
            marker: Some((native_panel::MARKER, theme::ACCENT)),
            // Un monde natif n'a pas encore d'images : il garde sa jaquette dessinée.
            cover: None,
            pin: Some(world_pin(world)),
        };
        cards.push((look, selected, CardTarget::Native(world.key.clone())));
    }
    let per_row = cards_per_row(ui.available_width());
    // `scope` : l'espacement des cartes ne vaut que pour la grille, pas pour les
    // panneaux dessinés ensuite sous elle.
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
        // `chunks` découpe la liste en rangées de `per_row` cartes (la dernière peut
        // être plus courte).
        for row in cards.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for (look, selected, target) in row {
                    if cover_card(ui, look, *selected) {
                        clicked = Some(match target {
                            CardTarget::Game(id) => CardTarget::Game(id.clone()),
                            CardTarget::Native(key) => CardTarget::Native(key.clone()),
                        });
                    }
                }
            });
        }
    });
    match clicked {
        Some(CardTarget::Game(id)) => {
            if state.selected.as_ref() != Some(&id) {
                state.confirm_uninstall = false;
            }
            state.selected = Some(id);
            state.native.selected = None;
        }
        Some(CardTarget::Native(key)) => {
            state.native.select(&key);
            state.selected = None;
            state.confirm_uninstall = false;
        }
        None => {}
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
pub(super) fn small_cover(ui: &mut Ui, size: f32, game_id: &str, name: &str) {
    small_cover_with(ui, size, game_id, name, None);
}

/// Comme [`small_cover`], avec la jaquette du jeu (ADR 0051) quand elle est prête :
/// découpée au centre pour tenir dans le carré.
pub(super) fn small_cover_with(
    ui: &mut Ui,
    size: f32,
    game_id: &str,
    name: &str,
    texture: Option<&egui::TextureHandle>,
) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    if let Some(texture) = texture {
        art_ui::paint_cover(ui, texture, rect);
        return;
    }
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

/// Hauteur du bandeau en haut de la fiche quand il n'y a pas d'illustration.
const FLAT_BANNER_HEIGHT: f32 = 96.0;

/// Hauteur du bandeau en haut de la fiche : en 16:9 sur toute la largeur, 180 px
/// au plus avec une illustration ; plus bas pour l'aplat, qui ne montre que le titre.
fn hero_height(width: f32, has_image: bool) -> f32 {
    let wide = (width * 9.0 / 16.0).min(180.0);
    if has_image {
        wide
    } else {
        wide.min(FLAT_BANNER_HEIGHT)
    }
}

/// Bandeau en haut de la fiche du jeu, sur toute la largeur du panneau :
/// l'image large du jeu (ADR 0051) si elle est prête ; sinon (aucune image
/// installée ni chez Steam, image pas encore lue) un aplat de la teinte du jeu,
/// avec son titre, comme les jaquettes de la grille.
pub(super) fn hero_banner(
    ui: &mut Ui,
    texture: Option<&egui::TextureHandle>,
    game_id: &str,
    name: &str,
) {
    let width = ui.available_width();
    let height = hero_height(width, texture.is_some());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
    match texture {
        Some(texture) => {
            art_ui::paint_cover(ui, texture, rect);
            // Nom lu par un lecteur d'écran (et par les tests sans fenêtre).
            let label = format!("Illustration de {name}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
        }
        None => {
            let painter = ui.painter();
            painter.rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                theme::cover_color(game_id),
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                name,
                theme::title_font(28.0),
                theme::TEXT,
            );
            let label = format!("Aplat de {name}");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
        }
    }
}

/// Ce qu'affiche la carte d'un jeu du catalogue.
fn card_look<'a>(
    profile: &'a Profile,
    view: &GameView,
    busy: Option<&Task>,
    from_registry: bool,
    cover: Option<egui::TextureHandle>,
) -> CardLook<'a> {
    // Barre sous la jaquette : pleine si le jeu est prêt, à moitié pendant une tâche.
    let fraction = match (busy, view.can_launch) {
        (Some(_), _) => 0.5,
        (None, true) => 1.0,
        (None, false) => 0.0,
    };
    let (status, status_color) = match busy {
        Some(task) => (format!("{}…", task.label), theme::ACCENT),
        None => (view.label.clone(), tone_color(view.tone)),
    };
    CardLook {
        cover_id: &profile.game.id,
        name: &profile.game.name,
        status,
        status_color,
        fraction,
        // Origine : un jeu venu du registre signé le dit (le catalogue embarqué non) ;
        // sinon, un profil jamais essayé avec le vrai jeu est marqué (ADR 0049).
        pin: None,
        marker: if from_registry {
            Some(("Registre", theme::ITEM))
        } else if is_untested(profile) {
            Some(("Non testé", theme::AMBER))
        } else {
            None
        },
        cover,
    }
}

/// Ce qu'une carte de la grille affiche (jeu du catalogue ou monde natif).
pub(super) struct CardLook<'a> {
    /// Identifiant qui choisit la teinte de la jaquette.
    pub(super) cover_id: &'a str,
    pub(super) name: &'a str,
    pub(super) status: String,
    pub(super) status_color: Color32,
    /// Remplissage de la barre sous la jaquette, de 0 à 1.
    pub(super) fraction: f32,
    /// Marqueur en petites capitales sous l'état (« Registre », « Monde natif »).
    pub(super) marker: Option<(&'a str, Color32)>,
    /// Jaquette du jeu (ADR 0051) ; `None` : l'aplat coloré avec le titre.
    pub(super) cover: Option<egui::TextureHandle>,
    /// Badge en haut à gauche de la jaquette : sorte de monde (« Natif »,
    /// « Converti »), voir [`WorldPin`]. `None` : pas de badge (monde d'Archipelago).
    pub(super) pin: Option<WorldPin>,
}

/// Sorte de monde d'un jeu, montrée en badge sur sa jaquette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldPin {
    /// Monde écrit au format natif de Raftup (ADR 0041).
    Native,
    /// Monde natif obtenu en convertissant un apworld (ADR 0042).
    Converted,
}

impl WorldPin {
    pub fn label(self) -> &'static str {
        match self {
            WorldPin::Native => "Natif",
            WorldPin::Converted => "Converti",
        }
    }

    /// Couleur du texte et du fond du badge.
    fn colors(self) -> (Color32, Color32) {
        match self {
            WorldPin::Native => (theme::GREEN, theme::GREEN_BG),
            WorldPin::Converted => (theme::ACCENT, theme::ACCENT_BG),
        }
    }
}

/// Badge d'un monde natif installé : « Converti » si son reçu dit qu'il vient de la
/// conversion d'un apworld, « Natif » sinon.
pub fn world_pin(world: &launcher_core::native_worlds::InstalledNativeWorld) -> WorldPin {
    let converted = world
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.conversion.is_some());
    if converted {
        WorldPin::Converted
    } else {
        WorldPin::Native
    }
}

/// Dessine le badge `pin` dans le coin haut gauche de la jaquette `rect`.
fn paint_pin(ui: &Ui, rect: egui::Rect, pin: WorldPin) {
    let (fg, bg) = pin.colors();
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        pin.label().to_uppercase(),
        egui::FontId::proportional(11.0),
        fg,
    );
    let size = galley.size() + egui::vec2(12.0, 6.0);
    let badge = egui::Rect::from_min_size(rect.min + egui::vec2(8.0, 8.0), size);
    let radius = CornerRadius::same(theme::RADIUS);
    painter.rect_filled(badge, radius, bg);
    painter.rect_stroke(
        badge,
        radius,
        egui::Stroke::new(1.0, fg),
        egui::StrokeKind::Inside,
    );
    painter.galley(badge.center() - galley.size() / 2.0, galley, fg);
}

/// Dessine une carte. Renvoie vrai au clic.
/// Largeur d'une carte de la grille (jaquette comprise).
pub(super) const CARD_WIDTH: f32 = 150.0;
/// Espace entre deux cartes, en largeur comme en hauteur.
pub(super) const CARD_GAP: f32 = 22.0;

pub(super) fn cover_card(ui: &mut Ui, look: &CardLook<'_>, selected: bool) -> bool {
    const WIDTH: f32 = CARD_WIDTH;
    // Jaquette en portrait (2:3), comme les couvertures de la maquette.
    const COVER: egui::Vec2 = egui::vec2(WIDTH, 210.0);
    let response = ui
        .vertical(|ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 6.0;
            // La jaquette : l'illustration du jeu (ADR 0051) si elle est
            // prête ; sinon un aplat coloré avec le titre du jeu, dont la teinte dépend
            // de l'identifiant du jeu.
            let (slot, cover_response) = ui.allocate_exact_size(COVER, Sense::hover());
            // Survol animé : `animate_bool_with_time` fait glisser `lift` de 0 à 1 en
            // 0,15 s quand la souris arrive (et de 1 à 0 quand elle part) ; egui redessine
            // tout seul pendant l'animation.
            let lift = ui.ctx().animate_bool_with_time(
                egui::Id::unique(("survol-jaquette", look.cover_id)),
                cover_response.hovered(),
                HOVER_SECONDS,
            );
            let rect = hover_rect(slot, lift);
            let radius = CornerRadius::same(theme::RADIUS);
            if lift > 0.0 {
                // Ombre douce sous la jaquette soulevée.
                let shadow = egui::epaint::Shadow {
                    offset: [0, 6],
                    blur: 18,
                    spread: 0,
                    color: Color32::from_black_alpha((110.0 * lift) as u8),
                };
                ui.painter().add(shadow.as_shape(rect, radius));
            }
            match &look.cover {
                Some(texture) => {
                    art_ui::paint_cover(ui, texture, rect);
                    // Nom lu par un lecteur d'écran (et par les tests sans fenêtre).
                    let label = format!("Jaquette de {}", look.name);
                    cover_response
                        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &label));
                }
                None => {
                    let painter = ui.painter();
                    painter.rect_filled(rect, radius, theme::cover_color(look.cover_id));
                    // Le titre passe à la ligne dans la jaquette (marge de 12 px de chaque côté).
                    let galley = painter.layout(
                        look.name.to_string(),
                        theme::title_font(22.0),
                        theme::TEXT,
                        rect.width() - 24.0,
                    );
                    // Centré dans la jaquette : on part du coin haut gauche du bloc de texte.
                    let top_left = rect.center() - galley.size() / 2.0;
                    painter.galley(top_left, galley, theme::TEXT);
                }
            }
            if let Some(pin) = look.pin {
                paint_pin(ui, rect, pin);
            }
            let painter = ui.painter();
            if lift > 0.0 {
                // Voile clair : la jaquette « s'allume » sous la souris.
                let glow = Color32::from_white_alpha((14.0 * lift) as u8);
                painter.rect_filled(rect, radius, glow);
            }
            // Jeu sélectionné : liseré bleu de 2 px ; survol : le liseré bleuit peu à peu.
            painter.rect_stroke(
                rect,
                radius,
                card_stroke(selected, lift),
                egui::StrokeKind::Outside,
            );
            widgets::bar(ui, WIDTH, look.fraction, theme::ACCENT);
            ui.label(
                RichText::new(look.name)
                    .strong()
                    .family(theme::bold_family())
                    .size(15.0)
                    .color(theme::TEXT),
            );
            ui.label(
                RichText::new(&look.status)
                    .color(look.status_color)
                    .size(13.0),
            );
            if let Some((marker, color)) = look.marker {
                ui.label(widgets::caps(marker).color(color));
            }
        })
        .response
        // `interact` rend toute la carte cliquable, pas seulement son texte.
        .interact(Sense::click());
    // Nom de la carte entière pour un lecteur d'écran (et pour les tests sans fenêtre).
    let label = format!("Fiche de {}", look.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}

/// Message éphémère tiré du résultat d'une tâche : sa première phrase seulement (le
/// bandeau de l'écran garde le texte entier).
fn toast_of(result: &Result<String, String>) -> (String, ToastKind) {
    let (text, kind) = match result {
        Ok(text) => (text, ToastKind::Success),
        Err(text) => (text, ToastKind::Error),
    };
    let first = match text.find(". ") {
        Some(end) => &text[..=end],
        None => text.as_str(),
    };
    (first.trim().to_string(), kind)
}

/// Durée de l'animation de survol d'une jaquette, en secondes.
const HOVER_SECONDS: f32 = 0.15;

/// Rectangle de la jaquette survolée : `lift` va de 0 (repos) à 1 (souris dessus) ; elle
/// grossit alors de 4 px de chaque côté, autour du même centre.
fn hover_rect(slot: egui::Rect, lift: f32) -> egui::Rect {
    slot.expand(4.0 * lift.clamp(0.0, 1.0))
}

/// Liseré de la jaquette : bleu de 2 px si le jeu est sélectionné ; sinon gris, qui
/// tourne au bleu pendant le survol (`lift` de 0 à 1).
fn card_stroke(selected: bool, lift: f32) -> Stroke {
    if selected {
        return Stroke::new(2.0, theme::ACCENT);
    }
    let lift = lift.clamp(0.0, 1.0);
    // `lerp_to_gamma` : mélange des deux couleurs, `lift` étant la part de la seconde.
    let color = theme::BORDER.lerp_to_gamma(theme::ACCENT, lift);
    Stroke::new(1.0 + lift, color)
}

/// Panneau de droite : le jeu sélectionné.
fn detail(ui: &mut Ui, state: &mut LibraryState, shared: &Shared, ctx: &egui::Context) {
    // Un monde natif sélectionné : sa fiche à lui (ADR 0046).
    if state.native.selected.is_some() {
        let Some(native_dir) = state.native_dir() else {
            return;
        };
        let mut uninstalled = false;
        native_panel::detail(ui, &mut state.native, &native_dir, &mut uninstalled);
        if uninstalled {
            if let Ok(paths) = &state.paths {
                let worlds_dir = paths.worlds_dir(AP_VERSION);
                state.registry.refresh_installed(Targets {
                    worlds_dir: &worlds_dir,
                    native_dir: &native_dir,
                });
            }
        }
        return;
    }
    let Ok(catalog) = &state.catalog else { return };
    let Some(profile) = state.selected.as_ref().and_then(|id| catalog.get(id)) else {
        return;
    };
    // Un autre jeu choisi : sa fiche apparaît en fondu.
    if state.detail_since.0 != state.selected {
        state.detail_since = (state.selected.clone(), std::time::Instant::now());
    }
    if state.fade {
        widgets::apply_fade(ui, state.detail_since.1);
    }
    // Copie du profil : les boutons modifient `state` plus bas, on ne peut pas garder
    // en même temps un emprunt sur `state.catalog`.
    let profile = profile.clone();
    let id = profile.game.id.clone();
    let found = state.scan.as_ref().and_then(|s| s.found.get(&id)).cloned();
    let status = state.scan.as_ref().and_then(|s| s.status.get(&id)).cloned();
    let view = view(&profile, found.as_ref(), status.as_ref());
    let busy = state.task.is_some();
    // Illustrations (ADR 0051) : image large en haut de la fiche, jaquette à côté du
    // titre. Absentes : un aplat en haut, et l'aplat avec les initiales.
    let cover = state.art.texture(ctx, &id, ArtKind::Cover);
    let hero = state.art.texture(ctx, &id, ArtKind::Hero);

    widgets::panel(ui, "", |ui| {
        hero_banner(ui, hero.as_ref(), &profile.game.id, &profile.game.name);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            small_cover_with(
                ui,
                64.0,
                &profile.game.id,
                &profile.game.name,
                cover.as_ref(),
            );
            ui.vertical(|ui| {
                ui.label(widgets::title(&profile.game.name, 26.0));
                ui.label(RichText::new(&view.label).color(tone_color(view.tone)));
                // L'origine ne se dit que pour un jeu ajouté par le registre : le cas
                // normal (catalogue de Raftup) n'a pas besoin d'être annoncé.
                if state.registry.is_from_registry(&id) {
                    ui.label(widgets::dim("Ajouté par le registre signé").size(12.0));
                }
            });
        });
        if is_untested(&profile) {
            ui.add_space(6.0);
            ui.label(RichText::new(UNTESTED_NOTE).size(13.0).color(theme::AMBER));
        }
        ui.add_space(10.0);

        // Boutons d'action : « Lancer » en bleu quand le jeu est prêt ; sinon c'est
        // l'installation qui devient l'action principale.
        // « Réinstaller » d'un jeu déjà prêt n'est pas une action de tous les jours : il
        // descend en petit bouton à côté de « Désinstaller ». « Mettre à jour » et
        // « Réparer » restent en grand, à côté de « Lancer le jeu ».
        let small_reinstall = view.can_launch && view.install_label == "Réinstaller";
        let mut install_clicked = false;
        ui.horizontal(|ui| {
            let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            let full = ui.available_width();
            if let (true, Some(game)) = (view.can_launch, &found) {
                let ready = launch_mode(&state.form) != LaunchMode::Incomplete;
                let width = if view.can_install && !small_reinstall {
                    half
                } else {
                    full
                };
                let response = big_button(ui, "Lancer le jeu", true, ready && !busy, width);
                if response {
                    let dir = game.install_dir.clone();
                    state.start_launch(ctx, &profile, &dir);
                }
            }
            if view.can_install && !small_reinstall {
                let primary = !view.can_launch;
                let width = if view.can_launch { half } else { full };
                install_clicked = big_button(ui, view.install_label, primary, !busy, width);
            }
        });
        if view.can_launch && launch_mode(&state.form) == LaunchMode::Incomplete {
            ui.label(
                widgets::dim(
                    "Pour te connecter, indique l'adresse et ton nom de joueur sous « Se connecter à la main ».",
                )
                .size(12.0),
            );
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
                ui.label(widgets::dim(
                    "Jeu non trouvé sur ce PC. S'il est installé ailleurs que dans Steam ou Epic, indique son dossier.",
                ));
                ui.add_space(4.0);
                // Même action que « Ajouter un dossier de jeu » de l'en-tête, pour ce jeu.
                if widgets::primary_button(ui, "Indiquer le dossier du jeu", true) {
                    state.pending_pick = state.selected.clone();
                }
            }
        }

        // Profil d'installation : replié, c'est un détail (le joueur voit déjà « Prêt »).
        let installed = matches!(
            status,
            Some(Ok(InstallStatus::Installed | InstallStatus::UpdateAvailable))
        );
        let mut open_now = false;
        widgets::advanced(
            ui,
            "library_steps",
            "Ce que Raftup installe",
            &mut open_now,
            |ui| {
                for step in &profile.steps {
                    step_row(ui, step, installed);
                }
            },
        );

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

        // Réinstallation (en petit) et désinstallation, en deux clics.
        if view.can_uninstall || small_reinstall {
            ui.add_space(12.0);
            ui.separator();
        }
        if small_reinstall && !state.confirm_uninstall {
            install_clicked |= widgets::ghost_button(ui, "Réinstaller");
        }
        // L'installation part d'ici, quel que soit le bouton (grand ou petit).
        if let (true, Some(game)) = (view.can_install, &found) {
            let auto = std::mem::take(&mut state.dev_auto_install);
            if (install_clicked || auto) && !busy {
                let dir = game.install_dir.clone();
                let force = matches!(status, Some(Ok(InstallStatus::InProgress)));
                state.start_install(ctx, &profile, dir, force);
            }
        }
        if view.can_uninstall {
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
        StepKind::ModLoader => "Chargeur de mods",
        StepKind::Mod => "Mod du jeu",
        StepKind::Apworld => "Règles Archipelago",
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
                        "Installé · vérifié"
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
            "Aucune partie choisie : ouvre un lien d'invitation (« Rejoindre »).".to_string()
        }
        LaunchMode::Incomplete => {
            "Connexion incomplète : il manque l'adresse ou le nom de joueur.".to_string()
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
            ui.add(egui::TextEdit::singleline(&mut form.address).hint_text("adresse:38281"));
            ui.end_row();

            ui.label(widgets::dim("Joueur"));
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
            "Saisie dans le jeu : Raftup copie l'adresse et le nom de joueur dans le presse-papiers.",
        ));
    }
}

/// Chemin en police monospace dans un cadre sombre.
pub(super) fn code_box(ui: &mut Ui, text: &str) {
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

    /// Bug signalé le 22 novembre 2026 : la Bibliothèque montrait tous les jeux du
    /// catalogue, même ceux absents du PC. Elle ne garde que les jeux trouvés ; les
    /// autres restent dans l'écran « Catalogue ».
    #[test]
    fn seuls_les_jeux_du_pc_sont_dans_la_bibliotheque() {
        let catalog = Catalog::from_texts(&[
            (
                "slime-rancher.toml",
                include_str!("../../../../profiles/slime-rancher.toml"),
            ),
            (
                "hollow-knight.toml",
                include_str!("../../../../profiles/hollow-knight.toml"),
            ),
        ])
        .unwrap();
        let mut scan = Scan::default();
        scan.found.insert(
            "slime-rancher".to_string(),
            DetectedGame {
                game_id: "slime-rancher".to_string(),
                store: launcher_core::Store::Steam,
                install_dir: std::path::PathBuf::from("C:/Jeux/Slime Rancher"),
                build_id: None,
                state: launcher_core::GameState::Ready,
            },
        );
        // Comme la vraie détection : un état des mods pour chaque jeu du catalogue, même
        // « non installé » (c'est ce qui faisait encore tout afficher).
        scan.status
            .insert("slime-rancher".to_string(), Ok(InstallStatus::Installed));
        scan.status
            .insert("hollow-knight".to_string(), Ok(InstallStatus::NotInstalled));
        let root = tempfile::tempdir().unwrap();
        let state = LibraryState::for_tests(catalog, Paths::under(root.path()), scan);

        assert_eq!(state.library_ids(), vec!["slime-rancher".to_string()]);
        assert_eq!(state.game_count(), 1);
        assert!(!state.in_library("hollow-knight"));
    }

    #[test]
    fn message_ephemere_premiere_phrase() {
        assert_eq!(
            toast_of(&Ok("Slime Rancher installé. Les mods sont prêts.".into())),
            ("Slime Rancher installé.".to_string(), ToastKind::Success)
        );
        assert_eq!(
            toast_of(&Err("Téléchargement impossible".into())),
            ("Téléchargement impossible".to_string(), ToastKind::Error)
        );
    }

    #[test]
    fn jaquette_soulevee_au_survol() {
        let slot = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(150.0, 210.0));
        // Au repos : rien ne bouge, liseré gris d'1 px.
        assert_eq!(hover_rect(slot, 0.0), slot);
        assert_eq!(card_stroke(false, 0.0), Stroke::new(1.0, theme::BORDER));
        // Souris dessus : 4 px de plus de chaque côté, même centre, liseré bleu de 2 px.
        let up = hover_rect(slot, 1.0);
        assert_eq!(up.width(), 158.0);
        assert_eq!(up.center(), slot.center());
        assert_eq!(card_stroke(false, 1.0), Stroke::new(2.0, theme::ACCENT));
        // Hors bornes : ramené entre 0 et 1.
        assert_eq!(hover_rect(slot, 3.0), up);
        // Sélectionné : toujours le liseré bleu.
        assert_eq!(card_stroke(true, 0.0), Stroke::new(2.0, theme::ACCENT));
    }

    #[test]
    fn bandeau_de_la_fiche_plus_bas_en_aplat() {
        // Avec une image : 16:9, plafonné à 180 px.
        assert_eq!(hero_height(160.0, true), 90.0);
        assert_eq!(hero_height(640.0, true), 180.0);
        // En aplat : jamais plus de 96 px, et pas plus que le 16:9 d'un panneau étroit.
        assert_eq!(hero_height(640.0, false), FLAT_BANNER_HEIGHT);
        assert_eq!(hero_height(160.0, false), 90.0);
    }

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
        assert_eq!(v.label, "Pas sur ce PC");
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
    fn cartes_par_rangee() {
        // 150 px par carte et 22 px d'espace : 600 px → 3 cartes, 150 px → 1.
        assert_eq!(cards_per_row(600.0), 3);
        assert_eq!(cards_per_row(150.0), 1);
        assert_eq!(cards_per_row(322.0), 2);
        // Trop étroit, ou largeur absurde : une carte quand même.
        assert_eq!(cards_per_row(10.0), 1);
        assert_eq!(cards_per_row(f32::NAN), 1);
    }

    #[test]
    fn profil_non_teste_s_installe_et_se_signale() {
        // ADR 0049 : même comportement qu'un profil « available », mais annoncé.
        let text = DEMO.replace("status = \"available\"", "status = \"untested\"");
        let untested = Catalog::from_texts(&[("demo.toml", &text)]).unwrap();
        let p = untested.get("demo").unwrap();
        assert!(is_untested(p));
        let g = found(GameState::Ready);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::NotInstalled)));
        assert!(v.can_install);
        let v = view(p, Some(&g), Some(&Ok(InstallStatus::Installed)));
        assert!(v.can_launch);
        // Le profil de démonstration tel quel (« available ») n'est pas marqué.
        let available = catalog();
        assert!(!is_untested(available.get("demo").unwrap()));
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

    // --- Mondes natifs (ADR 0046) : l'écran dessiné sans fenêtre ---

    const APQUEST: &str = include_str!("../../../../spec/monde-natif/exemples/apquest/world.toml");

    /// Tous les textes dessinés d'une image (formes imbriquées comprises).
    fn texts(output: &egui::FullOutput) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// Une Bibliothèque sur un dossier temporaire, avec le jeu de démo et APQuest natif,
    /// sans registre ni détection des jeux (rien ne part sur le réseau ni dans un thread).
    fn natif_state(temp: &Path) -> LibraryState {
        let paths = Paths::under(temp);
        let dir = paths.native_worlds_dir().join("apquest");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("world.toml"), APQUEST).unwrap();
        let mut state = LibraryState::new();
        state.catalog = Ok(catalog());
        state.paths = Ok(paths);
        state.scan = Some(Scan::default());
        state.registry = RegistryPanel::disabled();
        state.native = NativeState::default();
        state.native.selected = None;
        state
    }

    /// Dessine une image de l'écran, assez large pour la mise en page à deux colonnes.
    fn draw(ctx: &egui::Context, state: &mut LibraryState) -> Vec<String> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        };
        // `run_ui` : une image entière, l'écran dessiné dans l'`Ui` racine.
        let mut output = ctx.run_ui(input, |ui| {
            show(ui, state, &Shared::default(), None);
        });
        // Sans fenêtre, personne n'envoie les textures (police) au GPU : on les jette
        // exprès, sinon egui s'en plaint à la destruction.
        output.textures_delta.clear();
        texts(&output)
    }

    #[test]
    fn monde_natif_affiche_comme_un_jeu_avec_sa_fiche() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = natif_state(temp.path());
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);

        let shown = draw(&ctx, &mut state);
        assert_eq!(state.native.count(), 1);
        // Le jeu du catalogue n'est pas sur ce PC : il n'est pas dans la Bibliothèque,
        // et c'est le monde natif qui est sélectionné par défaut.
        assert_eq!(state.selected, None);
        assert_eq!(state.native.selected.as_deref(), Some("apquest"));
        assert!(shown.iter().any(|t| t == "MONDE NATIF"), "{shown:?}");
        assert!(
            shown.iter().any(|t| t.contains("1 monde natif")),
            "{shown:?}"
        );
        assert!(shown.iter().any(|t| t.ends_with("lieux · format 1")));

        // Clic sur la carte (comme `grid`) : la fiche du monde remplace celle du jeu.
        state.native.select("apquest");
        state.selected = None;
        let shown = draw(&ctx, &mut state);
        assert_eq!(
            state.selected, None,
            "la sélection par défaut ne doit pas revenir"
        );
        for attendu in [
            "APQuest",
            "Monde natif",
            "format_version",
            "Lieux",
            "Objets",
            "Désinstaller le monde natif",
        ] {
            assert!(shown.iter().any(|t| t == attendu), "{attendu} : {shown:?}");
        }
        assert!(shown.iter().any(|t| t.contains("à la main")));
    }

    /// Badge sur la jaquette : « Natif » pour un monde écrit au format de Raftup,
    /// « Converti » quand son reçu dit qu'il vient d'un apworld (ADR 0042).
    #[test]
    fn badge_natif_ou_converti_sur_la_jaquette() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);

        let temp = tempfile::tempdir().unwrap();
        let mut state = natif_state(temp.path());
        let shown = draw(&ctx, &mut state);
        assert!(shown.iter().any(|t| t == "NATIF"), "{shown:?}");
        assert!(!shown.iter().any(|t| t == "CONVERTI"), "{shown:?}");
        // Un jeu sans monde natif tourne avec le monde d'Archipelago : pas de badge.
        let pin = state.pin_for("Un jeu sans monde natif");
        assert_eq!(pin, None);
        assert_eq!(state.pin_for("APQuest"), Some(WorldPin::Native));

        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::under(temp.path());
        let dir = paths.native_worlds_dir().join("apquest");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(launcher_core::native_worlds::RECEIPT_FILE),
            "format_version = 1\nfiles = [\"world.toml\"]\n\
             [conversion]\napworld = \"apquest.apworld\"\nap_version = \"0.6.8\"\nnotes = 0\n",
        )
        .unwrap();
        let mut state = natif_state(temp.path());
        let shown = draw(&ctx, &mut state);
        assert!(shown.iter().any(|t| t == "CONVERTI"), "{shown:?}");
        assert!(!shown.iter().any(|t| t == "NATIF"), "{shown:?}");
    }

    #[test]
    fn desinstaller_un_monde_natif_depuis_la_fiche() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = natif_state(temp.path());
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        state.native.select("apquest");
        state.selected = None;
        draw(&ctx, &mut state);
        let native_dir = state.native_dir().unwrap();
        // Les deux clics de la fiche, sans souris : la logique qu'ils appellent.
        let message = native_panel::uninstall_message(
            "APQuest",
            launcher_core::native_worlds::uninstall_native_world(&native_dir, "apquest"),
        );
        assert!(message.is_ok());
        state.native.refresh(&native_dir);
        // L'image suivante n'a plus de monde natif ; le jeu du catalogue, absent de ce
        // PC, ne revient pas dans la Bibliothèque.
        let shown = draw(&ctx, &mut state);
        assert_eq!(state.native.count(), 0);
        assert_eq!(state.selected, None);
        assert!(!shown.iter().any(|t| t == "MONDE NATIF"));
        assert!(!native_dir.join("apquest").exists());
    }

    #[test]
    fn seulement_des_mondes_natifs_sans_catalogue() {
        let temp = tempfile::tempdir().unwrap();
        let mut state = natif_state(temp.path());
        state.catalog = Ok(Catalog::from_texts(&[]).unwrap());
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let shown = draw(&ctx, &mut state);
        // Pas d'état vide : le monde natif est sélectionné et sa fiche affichée.
        assert!(!shown.iter().any(|t| t == "Aucun jeu dans le catalogue"));
        assert_eq!(state.native.selected.as_deref(), Some("apquest"));
        assert!(shown.iter().any(|t| t == "Désinstaller le monde natif"));
    }
}
