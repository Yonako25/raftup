//! Orchestration d'une génération : préparer les dossiers, appeler le générateur,
//! extraire et lire la seed produite (ADR 0017).
//!
//! Le générateur lui-même est caché derrière le trait [`GeneratorBackend`] : le vrai
//! (`PythonBackend`) parle au worker Python, les tests mettent un faux.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use py_bridge::{
    BridgeError, BridgeOptions, CancelToken, GenerateParams, GenerateResult, GeneratedPlayer,
    PyBridge, Sandbox, ERR_GENERATION_FAILED, ERR_IMPORT_FAILED,
};

use super::GenerationError;
use crate::archive::{extract_zip, ArchiveError, ArchiveLimits};
use crate::paths::Paths;
use crate::seed::SeedSummary;

/// Taille maximale d'un fichier joueur (1 Mio).
const MAX_PLAYER_FILE_BYTES: u64 = 1024 * 1024;
/// Taille maximale du `archipelago.json` lu dans un apworld.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Noms de fichiers que le générateur lit comme réglages globaux, pas comme joueurs.
const FORBIDDEN_PLAYER_FILES: [&str; 2] = ["meta.yaml", "weights.yaml"];

/// Délai par défaut d'une génération : 15 minutes.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Niveau de spoiler par défaut : 3 (gardé pour l'hôte, ADR 0017 décision 9).
pub const DEFAULT_SPOILER: u8 = 3;

/// Ce que le joueur demande.
#[derive(Debug, Clone)]
pub struct GenerateRequest {
    /// Fichiers YAML des joueurs.
    pub player_files: Vec<PathBuf>,
    /// Apworlds supplémentaires à poser dans `custom_worlds`.
    pub apworlds: Vec<PathBuf>,
    pub seed: Option<u64>,
    /// Niveau de spoiler, de 0 à 3.
    pub spoiler: u8,
    pub race: bool,
    pub timeout: Duration,
    /// Contenu d'un `meta.yaml` (réglages globaux du générateur, ex. `progression_balancing`).
    /// Posé tel quel dans le dossier des joueurs, où Archipelago le lit sous ce nom (F06, ADR 0023).
    pub meta_yaml: Option<String>,
}

impl Default for GenerateRequest {
    /// Aucun fichier ; seed aléatoire, spoiler [`DEFAULT_SPOILER`], délai [`DEFAULT_TIMEOUT`].
    fn default() -> Self {
        GenerateRequest {
            player_files: Vec::new(),
            apworlds: Vec::new(),
            seed: None,
            spoiler: DEFAULT_SPOILER,
            race: false,
            timeout: DEFAULT_TIMEOUT,
            meta_yaml: None,
        }
    }
}

/// Ce que la génération rend.
#[derive(Debug, Clone)]
pub struct GenerateOutput {
    /// Le `.archipelago` extrait.
    pub archipelago_file: PathBuf,
    /// Le zip produit par le générateur.
    pub zip_file: PathBuf,
    pub seed_name: String,
    pub players: Vec<GeneratedPlayer>,
    pub summary: SeedSummary,
}

/// Qui sait vraiment générer. Un trait = un contrat que plusieurs types peuvent remplir
/// (le vrai worker Python, ou un faux dans les tests), comme une interface en Java.
pub trait GeneratorBackend {
    /// Dossier d'Archipelago utilisé par ce générateur : la seule source de ce chemin
    /// (c'est là que `generate` pose les apworlds).
    fn archipelago_dir(&self) -> &Path;

    /// Ce moteur lit-il les apworlds ? Le moteur Rust (ses mondes sont dans Raftup) répond
    /// non : on ne touche alors ni au dossier d'Archipelago ni à `custom_worlds`.
    fn uses_apworlds(&self) -> bool {
        true
    }

    /// Qui a fait la dernière génération (à lire après `run`). Par défaut : Archipelago
    /// seul ; le moteur Rust et le moteur « export » le précisent.
    fn report(&self) -> super::EngineReport {
        super::EngineReport::plain(super::EngineUsed::Python)
    }

    fn run(
        &mut self,
        p: &GenerateParams,
        timeout: Duration,
        cancel: &CancelToken,
        log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError>;
}

/// Ce qu'il faut pour lancer le vrai générateur.
#[derive(Debug, Clone)]
pub struct Runtime {
    pub python_exe: PathBuf,
    pub archipelago_dir: PathBuf,
    pub site_packages: Option<PathBuf>,
    pub ap_version: String,
}

impl Runtime {
    /// Runtime déjà présent sur le disque : pour les tests et le développement.
    ///
    /// Si `archipelago_dir` n'est PAS sous le dossier de données de Raftup, `generate` ne
    /// vide jamais son `custom_worlds` : sans apworld demandé il n'y touche pas, avec des
    /// apworlds il refuse (`GenerationError::UnmanagedArchipelago`). Les apworlds ne sont
    /// donc jamais copiés dans un dossier qui n'appartient pas à Raftup.
    pub fn from_local(
        python_exe: impl Into<PathBuf>,
        archipelago_dir: impl Into<PathBuf>,
        site_packages: Option<PathBuf>,
        ap_version: impl Into<String>,
    ) -> Runtime {
        Runtime {
            python_exe: python_exe.into(),
            archipelago_dir: archipelago_dir.into(),
            site_packages,
            ap_version: ap_version.into(),
        }
    }
}

/// Le vrai générateur : un worker Python neuf par génération (ADR 0017, décision 5).
#[derive(Debug, Clone)]
pub struct PythonBackend {
    pub runtime: Runtime,
    /// Refuse toute connexion réseau sortante du worker (`RAFTUP_BLOCK_NETWORK=1`).
    /// Activé par défaut (ADR 0019, décision 3) ; ce n'est pas un bac à sable.
    pub block_network: bool,
    /// Bac à sable du worker (ADR 0034) : sous Windows, intégrité basse, écriture permise
    /// seulement dans le dossier de la génération. Activé par défaut ; hors Windows, sans effet.
    pub sandbox: bool,
}

impl PythonBackend {
    /// Backend avec les réglages par défaut : réseau bloqué, bac à sable.
    pub fn new(runtime: Runtime) -> PythonBackend {
        PythonBackend {
            runtime,
            block_network: true,
            sandbox: true,
        }
    }
}

impl PythonBackend {
    /// Lance un worker neuf (ADR 0017, décision 5) ; ses lignes de stderr vont dans `log`.
    /// Avec le bac à sable, le worker n'écrit que dans le dossier de la génération : le
    /// parent de `p.output_dir` (`generations/<horodatage>/`).
    pub(super) fn start_bridge(
        &self,
        p: &GenerateParams,
        log: Sender<String>,
    ) -> Result<PyBridge, GenerationError> {
        let mut options = BridgeOptions::new(&self.runtime.python_exe);
        options.working_dir = Some(self.runtime.archipelago_dir.clone());
        // Archipelago ne doit ni lancer pip ni toucher au réseau.
        options
            .env
            .insert("SKIP_REQUIREMENTS_UPDATE".to_string(), "1".to_string());
        if self.block_network {
            options
                .env
                .insert("RAFTUP_BLOCK_NETWORK".to_string(), "1".to_string());
        }
        if self.sandbox {
            let work_dir = p.output_dir.parent().unwrap_or(&p.output_dir);
            options.sandbox = Some(Sandbox::new(work_dir));
            ensure_host_yaml(&self.runtime.archipelago_dir)?;
        }
        options.stderr_lines = Some(log.clone());
        let bridge = start_with_sandbox_fallback(options, &log, PyBridge::start)?;
        if bridge.sandbox_applied() {
            let _ = log.send("Bac à sable : générateur en intégrité basse".to_string());
        } else if self.sandbox {
            let _ = log.send(
                "Bac à sable indisponible sur ce système : le générateur garde vos droits"
                    .to_string(),
            );
        }
        Ok(bridge)
    }

    /// Complète les paramètres avec ce que seul le runtime connaît.
    pub(super) fn complete(&self, p: &GenerateParams) -> GenerateParams {
        let mut params = p.clone();
        if self.runtime.site_packages.is_some() {
            params.site_packages = self.runtime.site_packages.clone();
        }
        params
    }
}

/// Lance le worker ; si le bac à sable ne peut pas être posé (disque sans étiquettes
/// d'intégrité : FAT32, lecteur réseau…), relance sans lui et le dit dans le journal,
/// comme hors Windows. Raftup n'avait pas de bac à sable avant l'ADR 0034 : on ne bloque
/// pas la génération pour autant. Toute autre erreur est rendue telle quelle.
///
/// Générique sur `T` (le pont démarré) pour être testée avec un faux démarrage : `start`
/// est une fonction qui reçoit les réglages (`FnMut` : elle peut être appelée deux fois).
pub(super) fn start_with_sandbox_fallback<T>(
    options: BridgeOptions,
    log: &Sender<String>,
    mut start: impl FnMut(BridgeOptions) -> Result<T, BridgeError>,
) -> Result<T, BridgeError> {
    // `clone` : on garde les réglages pour pouvoir relancer sans bac à sable.
    match start(options.clone()) {
        Err(BridgeError::SandboxSetup(reason)) if options.sandbox.is_some() => {
            let _ = log.send(format!(
                "Bac à sable impossible ici ({reason}) : le générateur garde vos droits"
            ));
            let mut options = options;
            options.sandbox = None;
            start(options)
        }
        other => other,
    }
}

impl GeneratorBackend for PythonBackend {
    fn archipelago_dir(&self) -> &Path {
        &self.runtime.archipelago_dir
    }

    fn run(
        &mut self,
        p: &GenerateParams,
        timeout: Duration,
        cancel: &CancelToken,
        log: Sender<String>,
    ) -> Result<GenerateResult, GenerationError> {
        let mut bridge = self.start_bridge(p, log)?;
        let params = self.complete(p);
        let result = bridge.generate_cancellable(&params, timeout, cancel);
        // Un worker neuf par génération : on le ferme dans tous les cas (erreur de fermeture ignorée).
        let _ = bridge.shutdown();
        Ok(result?)
    }
}

// Une seule génération à la fois dans ce processus (Archipelago écrit dans son dossier).
// `AtomicBool` = un booléen que plusieurs threads peuvent lire/écrire sans Mutex.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Réservation de « la » génération, libérée par `Drop` même en cas d'erreur.
//
// `pub(super)` : `ensure_runtime` (installation) prend le même verrou. Il ne vaut que pour
// CE process : deux instances du launcher ne se voient pas.
pub(super) struct LockGuard;

impl LockGuard {
    pub(super) fn acquire() -> Result<LockGuard, GenerationError> {
        // `swap` renvoie l'ancienne valeur : si c'était déjà `true`, quelqu'un génère.
        if ACTIVE.swap(true, Ordering::SeqCst) {
            return Err(GenerationError::Busy);
        }
        Ok(LockGuard)
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Dans le bac à sable, Archipelago ne peut pas créer son `host.yaml` (son dossier est en
/// lecture seule pour lui) : on pose un fichier vide s'il manque. Archipelago le lit comme
/// « aucun réglage », donc avec ses valeurs par défaut, comme un fichier qu'il aurait créé.
fn ensure_host_yaml(archipelago_dir: &Path) -> Result<(), GenerationError> {
    if archipelago_dir.as_os_str().is_empty() || !archipelago_dir.is_dir() {
        return Ok(());
    }
    let host = archipelago_dir.join("host.yaml");
    if !host.exists() {
        fs::write(&host, "").map_err(|e| io_err(&host, e))?;
    }
    Ok(())
}

fn io_err(path: &Path, source: std::io::Error) -> GenerationError {
    GenerationError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Lance une génération complète. Bloquant.
///
/// Le dossier d'Archipelago vient du backend ([`GeneratorBackend::archipelago_dir`]).
/// Après un échec ou une annulation, le dossier `generations/<horodatage>/` créé pour
/// cette génération est supprimé ; après un succès il reste (la seed y est).
pub fn generate(
    paths: &Paths,
    backend: &mut dyn GeneratorBackend,
    req: &GenerateRequest,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<GenerateOutput, GenerationError> {
    if req.player_files.is_empty() {
        return Err(GenerationError::NoPlayers);
    }
    let _guard = LockGuard::acquire()?;
    if cancel.is_cancelled() {
        return Err(GenerationError::Cancelled);
    }

    // Les chemins envoyés au worker sont absolus : il change de dossier courant.
    let archipelago_dir = if backend.uses_apworlds() {
        make_absolute(backend.archipelago_dir())?
    } else {
        PathBuf::new()
    };
    let (generation_dir, players_dir, output_dir) = make_generation_dirs(paths)?;

    let result = generate_in(
        paths,
        archipelago_dir,
        players_dir,
        output_dir,
        backend,
        req,
        cancel,
        log,
    );
    if result.is_err() {
        // Meilleur effort : l'erreur d'origine est plus utile que celle du nettoyage.
        let _ = remove_generation_dir(paths, &generation_dir);
    }
    result
}

/// Le travail de `generate` une fois les dossiers créés (séparé pour nettoyer sur toute erreur).
#[allow(clippy::too_many_arguments)]
fn generate_in(
    paths: &Paths,
    archipelago_dir: PathBuf,
    players_dir: PathBuf,
    output_dir: PathBuf,
    backend: &mut dyn GeneratorBackend,
    req: &GenerateRequest,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<GenerateOutput, GenerationError> {
    let _ = log.send("Préparation des fichiers joueurs".to_string());
    copy_player_files(&req.player_files, &players_dir)?;

    if let Some(meta) = &req.meta_yaml {
        // Pas un fichier joueur : Archipelago l'écarte et le lit comme réglages globaux.
        let meta_path = players_dir.join("meta.yaml");
        fs::write(&meta_path, meta).map_err(|e| io_err(&meta_path, e))?;
    }

    if backend.uses_apworlds() {
        let _ = log.send("Préparation des apworlds".to_string());
        install_apworlds(paths, &archipelago_dir, &req.apworlds)?;
    }

    let params = GenerateParams {
        archipelago_dir,
        site_packages: None,
        player_files_dir: players_dir,
        output_dir: output_dir.clone(),
        seed: req.seed,
        spoiler: req.spoiler,
        race: req.race,
    };
    let result = backend
        .run(&params, req.timeout, cancel, log.clone())
        .map_err(map_worker_error)?;
    if cancel.is_cancelled() {
        return Err(GenerationError::Cancelled);
    }

    let _ = log.send("Lecture de la seed produite".to_string());
    // On ne fait pas confiance au chemin annoncé par le worker : on le reconstruit.
    let zip_file = expected_zip(&result.seed_name, &output_dir)?;
    let archipelago_file = extract_archipelago(&zip_file, &output_dir)?;
    let (summary, _data) =
        SeedSummary::load(&archipelago_file).map_err(|e| GenerationError::Seed(e.to_string()))?;

    Ok(GenerateOutput {
        archipelago_file,
        zip_file,
        seed_name: result.seed_name,
        players: result.players,
        summary,
    })
}

/// Supprime `generations/<horodatage>/`. Refuse tout dossier qui n'est pas un enfant direct
/// de `paths.generations_dir()` : c'est la seule suppression récursive de ce fichier.
fn remove_generation_dir(paths: &Paths, dir: &Path) -> Result<(), GenerationError> {
    let root = make_absolute(&paths.generations_dir())?;
    let refused = || {
        io_err(
            dir,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "suppression refusée : pas un dossier de génération de Raftup",
            ),
        )
    };
    if dir.parent() != Some(root.as_path()) || dir.file_name().is_none() {
        return Err(refused());
    }
    // `symlink_metadata` : un lien n'est jamais suivi ni vidé.
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(dir).map_err(|e| io_err(dir, e)),
        Ok(_) => Err(refused()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_err(dir, e)),
    }
}

fn make_absolute(path: &Path) -> Result<PathBuf, GenerationError> {
    std::path::absolute(path).map_err(|e| io_err(path, e))
}

/// Crée `generations/<horodatage>/{players,output}` et rend le dossier puis les deux sous-dossiers.
fn make_generation_dirs(paths: &Paths) -> Result<(PathBuf, PathBuf, PathBuf), GenerationError> {
    let root = paths.generations_dir();
    fs::create_dir_all(&root).map_err(|e| io_err(&root, e))?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // `create_dir` (sans `_all`) échoue si le dossier existe : on ajoute un suffixe jusqu'à
    // trouver un nom libre, donc on n'écrase jamais une génération précédente.
    let mut n = 0u32;
    let dir = loop {
        let name = if n == 0 {
            millis.to_string()
        } else {
            format!("{millis}-{n}")
        };
        let candidate = root.join(name);
        match fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && n < 1000 => n += 1,
            Err(e) => return Err(io_err(&candidate, e)),
        }
    };
    let dir = make_absolute(&dir)?;
    let players = dir.join("players");
    let output = dir.join("output");
    for d in [&players, &output] {
        fs::create_dir(d).map_err(|e| io_err(d, e))?;
    }
    Ok((dir, players, output))
}

/// Copie chaque YAML en `P01_<nom>.yaml` après contrôles.
fn copy_player_files(files: &[PathBuf], players_dir: &Path) -> Result<(), GenerationError> {
    for (i, file) in files.iter().enumerate() {
        let refuse = |reason: &str| GenerationError::InvalidPlayerFile {
            path: file.clone(),
            reason: reason.to_string(),
        };
        let file_name = file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| refuse("nom de fichier illisible"))?;
        if FORBIDDEN_PLAYER_FILES
            .iter()
            .any(|f| file_name.eq_ignore_ascii_case(f))
        {
            return Err(refuse("ce n'est pas un fichier joueur (réglage global)"));
        }
        let meta = fs::metadata(file).map_err(|e| io_err(file, e))?;
        if !meta.is_file() {
            return Err(refuse("ce n'est pas un fichier"));
        }
        if meta.len() > MAX_PLAYER_FILE_BYTES {
            return Err(refuse("fichier trop gros (plus de 1 Mio)"));
        }
        let bytes = fs::read(file).map_err(|e| io_err(file, e))?;
        // Le fichier a pu grandir entre le contrôle et la lecture.
        if bytes.len() as u64 > MAX_PLAYER_FILE_BYTES {
            return Err(refuse("fichier trop gros (plus de 1 Mio)"));
        }
        let stem = file
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("joueur");
        // Seulement des caractères sûrs dans le nom copié.
        let stem: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .take(60)
            .collect();
        let target = players_dir.join(format!("P{:02}_{stem}.yaml", i + 1));
        fs::write(&target, bytes).map_err(|e| io_err(&target, e))?;
    }
    Ok(())
}

/// Le dossier d'Archipelago est-il sous les données de Raftup (et pas ce dossier lui-même) ?
/// Comparaison sur les chemins réels : un lien ou un `..` ne peut pas tromper le contrôle.
fn is_managed(paths: &Paths, archipelago_dir: &Path) -> bool {
    let (Ok(ap), Ok(data)) = (
        fs::canonicalize(archipelago_dir),
        fs::canonicalize(&paths.data_dir),
    ) else {
        return false;
    };
    ap != data && ap.starts_with(data)
}

/// Vide `custom_worlds` puis y copie les apworlds, après contrôle de compatibilité.
///
/// Le vidage n'a lieu que si Archipelago est sous les données de Raftup. Sinon : rien n'est
/// touché sans apworld demandé, et `UnmanagedArchipelago` avec des apworlds.
fn install_apworlds(
    paths: &Paths,
    archipelago_dir: &Path,
    apworlds: &[PathBuf],
) -> Result<(), GenerationError> {
    if !is_managed(paths, archipelago_dir) {
        return if apworlds.is_empty() {
            Ok(())
        } else {
            Err(GenerationError::UnmanagedArchipelago(
                archipelago_dir.to_path_buf(),
            ))
        };
    }

    // Contrôles avant tout effacement : un apworld refusé ne vide rien.
    let ap_version = read_ap_version(archipelago_dir);
    // Deux apworlds de même nom de fichier s'écraseraient dans `custom_worlds`
    // (comparaison insensible à la casse, comme Windows).
    let mut names: BTreeSet<String> = BTreeSet::new();
    for apworld in apworlds {
        check_apworld(apworld, ap_version.as_deref())?;
        let name = apworld
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !names.insert(name) {
            return Err(GenerationError::IncompatibleApworld {
                path: apworld.clone(),
                reason: "un autre apworld porte le même nom de fichier".to_string(),
            });
        }
    }

    let custom = archipelago_dir.join("custom_worlds");
    match fs::symlink_metadata(&custom) {
        Ok(meta) => {
            // Jamais de lien suivi : on ne vide que le vrai dossier d'Archipelago.
            if !meta.is_dir() {
                return Err(io_err(
                    &custom,
                    std::io::Error::other("custom_worlds n'est pas un vrai dossier"),
                ));
            }
            fs::remove_dir_all(&custom).map_err(|e| io_err(&custom, e))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(io_err(&custom, e)),
    }
    fs::create_dir_all(&custom).map_err(|e| io_err(&custom, e))?;
    for apworld in apworlds {
        let name = apworld
            .file_name()
            .ok_or_else(|| io_err(apworld, std::io::Error::other("nom illisible")))?;
        let target = custom.join(name);
        fs::copy(apworld, &target).map_err(|e| io_err(&target, e))?;
    }
    Ok(())
}

fn check_apworld(apworld: &Path, ap_version: Option<&str>) -> Result<(), GenerationError> {
    let refuse = |reason: String| GenerationError::IncompatibleApworld {
        path: apworld.to_path_buf(),
        reason,
    };
    let is_apworld = apworld
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("apworld"));
    if !is_apworld {
        return Err(refuse("l'extension doit être .apworld".to_string()));
    }
    let file = fs::File::open(apworld).map_err(|e| io_err(apworld, e))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| refuse(format!("archive illisible : {e}")))?;
    // Le manifeste est `archipelago.json` à la racine ou dans le dossier du monde.
    let mut manifest_index = None;
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index(i) else { continue };
        let Ok(name) = entry.name() else { continue };
        let name = name.replace('\\', "/");
        if name == "archipelago.json"
            || (name.ends_with("/archipelago.json") && name.matches('/').count() == 1)
        {
            manifest_index = Some(i);
            break;
        }
    }
    let Some(index) = manifest_index else {
        return Ok(()); // pas de manifeste : rien à contrôler
    };
    let entry = zip
        .by_index(index)
        .map_err(|e| refuse(format!("manifeste illisible : {e}")))?;
    let mut text = String::new();
    entry
        .take(MAX_MANIFEST_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| refuse(format!("manifeste illisible : {e}")))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| refuse(format!("manifeste invalide : {e}")))?;
    // Version illisible (manifeste ou installée) : on ne bloque pas à tort.
    let Some(current) = ap_version.and_then(parse_version) else {
        return Ok(());
    };
    let installed = ap_version.unwrap_or("?");
    if let Some(minimum) = json.get("minimum_ap_version").and_then(|v| v.as_str()) {
        if parse_version(minimum).is_some_and(|min| min > current) {
            return Err(refuse(format!(
                "demande Archipelago {minimum} au minimum, version installée : {installed}"
            )));
        }
    }
    if let Some(maximum) = json.get("maximum_ap_version").and_then(|v| v.as_str()) {
        if parse_version(maximum).is_some_and(|max| max < current) {
            return Err(refuse(format!(
                "demande Archipelago {maximum} au maximum, version installée : {installed}"
            )));
        }
    }
    Ok(())
}

/// Version d'Archipelago, lue dans la ligne `__version__ = "x.y.z"` de `Utils.py`.
fn read_ap_version(archipelago_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(archipelago_dir.join("Utils.py")).ok()?;
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("__version__")?;
        let rest = rest.trim_start().strip_prefix('=')?.trim();
        let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let inner = &rest[1..];
        Some(inner[..inner.find(quote)?].to_string())
    })
}

/// `"0.6.8"` -> `(0, 6, 8)` ; ce qui suit les chiffres d'un composant est ignoré.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.').map(|p| {
        let digits: String = p.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u32>().ok()
    });
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

/// Traduit les erreurs du worker en erreurs lisibles.
fn map_worker_error(error: GenerationError) -> GenerationError {
    match error {
        GenerationError::Bridge(BridgeError::Cancelled) => GenerationError::Cancelled,
        GenerationError::Bridge(BridgeError::Worker(rpc))
            if rpc.code == ERR_GENERATION_FAILED || rpc.code == ERR_IMPORT_FAILED =>
        {
            let data_text = |key: &str| {
                rpc.data
                    .as_ref()
                    .and_then(|d| d.get(key))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let default_kind = if rpc.code == ERR_IMPORT_FAILED {
                "ImportError"
            } else {
                "GenerationError"
            };
            GenerationError::GeneratorFailed {
                kind: data_text("type").unwrap_or_else(|| default_kind.to_string()),
                message: rpc.message.clone(),
                traceback: data_text("traceback").unwrap_or_default(),
            }
        }
        other => other,
    }
}

/// Le zip attendu est `output_dir/AP_<seed_name>.zip`, reconstruit ici : le chemin renvoyé
/// par le worker n'est jamais utilisé. Le nom de seed doit être `[A-Za-z0-9_-]+`, et le
/// fichier un fichier ordinaire (pas un lien).
fn expected_zip(seed_name: &str, output_dir: &Path) -> Result<PathBuf, GenerationError> {
    let valid_name = !seed_name.is_empty()
        && seed_name.len() <= 100
        && seed_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid_name {
        return Err(GenerationError::UnsafeOutput(PathBuf::from(seed_name)));
    }
    let zip = output_dir.join(format!("AP_{seed_name}.zip"));
    match fs::symlink_metadata(&zip) {
        Ok(meta) if meta.is_file() => Ok(zip),
        Ok(_) => Err(GenerationError::UnsafeOutput(zip)),
        Err(e) => Err(io_err(&zip, e)),
    }
}

/// Extrait le zip sous `output/extracted` et rend le `.archipelago` trouvé.
fn extract_archipelago(zip: &Path, output_dir: &Path) -> Result<PathBuf, GenerationError> {
    let target = output_dir.join("extracted");
    let limits = ArchiveLimits {
        max_entries: 1_000,
        max_total_bytes: 1024 * 1024 * 1024,
    };
    let mut found: Vec<PathBuf> = Vec::new();
    extract_zip(zip, "", &target, &limits, &mut |relative, _hash| {
        let is_seed = relative
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("archipelago"));
        if is_seed {
            found.push(relative.to_path_buf());
        }
    })
    .map_err(|e| match e {
        ArchiveError::UnsafePath(_) | ArchiveError::Link(_) | ArchiveError::SpecialFile(_) => {
            GenerationError::UnsafeOutput(zip.to_path_buf())
        }
        other => GenerationError::Archive(other),
    })?;
    // Ordre déterministe : le premier par nom.
    found.sort();
    let first = found.into_iter().next().ok_or(GenerationError::NoOutput)?;
    Ok(target.join(first))
}

/// Taille maximale du JSON écrit par un script d'inspection ([`inspect_worlds`]).
pub const MAX_INSPECTION_BYTES: u64 = 1024 * 1024;

/// Construit les mondes d'Archipelago jusqu'au placement, comme le moteur « export »
/// (ADR 0028), puis lance `script` sur le `MultiWorld` et rend le JSON qu'il a écrit.
/// Sert à la CI du registre (ADR 0042) pour lister les options d'un monde.
///
/// `script` est du Python qui définit `export(mw, path, max_bytes)` : il écrit son JSON
/// dans `path` (au plus `max_bytes` octets) et rend `{"bytes": n, "opaque_rules": 0}`.
/// Mêmes protections que `generate` : verrou, apworlds posées dans `custom_worlds`, bac à
/// sable, réseau bloqué selon `backend`. Le dossier de travail est toujours effacé.
pub fn inspect_worlds(
    paths: &Paths,
    backend: &PythonBackend,
    req: &GenerateRequest,
    script: &str,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<serde_json::Value, GenerationError> {
    if req.player_files.is_empty() {
        return Err(GenerationError::NoPlayers);
    }
    let _guard = LockGuard::acquire()?;
    let archipelago_dir = make_absolute(backend.archipelago_dir())?;
    let (generation_dir, players_dir, output_dir) = make_generation_dirs(paths)?;
    let result = inspect_in(
        paths,
        archipelago_dir,
        &generation_dir,
        players_dir,
        output_dir,
        backend,
        req,
        script,
        cancel,
        log,
    );
    // Rien à garder : le JSON est déjà lu (meilleur effort, l'erreur d'origine compte plus).
    let _ = remove_generation_dir(paths, &generation_dir);
    result
}

/// Le travail d'[`inspect_worlds`] une fois les dossiers créés.
#[allow(clippy::too_many_arguments)]
fn inspect_in(
    paths: &Paths,
    archipelago_dir: PathBuf,
    generation_dir: &Path,
    players_dir: PathBuf,
    output_dir: PathBuf,
    backend: &PythonBackend,
    req: &GenerateRequest,
    script: &str,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<serde_json::Value, GenerationError> {
    copy_player_files(&req.player_files, &players_dir)?;
    if let Some(meta) = &req.meta_yaml {
        let meta_path = players_dir.join("meta.yaml");
        fs::write(&meta_path, meta).map_err(|e| io_err(&meta_path, e))?;
    }
    install_apworlds(paths, &archipelago_dir, &req.apworlds)?;
    let params = GenerateParams {
        archipelago_dir,
        site_packages: None,
        player_files_dir: players_dir,
        output_dir,
        seed: req.seed,
        spoiler: 0,
        race: false,
    };
    let mut bridge = backend.start_bridge(&params, log)?;
    // Dans le dossier de la génération : le seul où le worker a le droit d'écrire.
    let export_path = generation_dir.join("inspection.json");
    let export = py_bridge::ExportParams {
        generate: backend.complete(&params),
        export_path: export_path.clone(),
        exporter: script.to_string(),
    };
    let result = bridge.export_worlds_cancellable(&export, req.timeout, cancel);
    // Le worker attend la suite du placement : on le ferme dans tous les cas.
    let _ = bridge.shutdown();
    result.map_err(|e| map_worker_error(GenerationError::Bridge(e)))?;
    let bytes = multidata::read_file_bounded(&export_path, MAX_INSPECTION_BYTES)
        .map_err(|e| io_err(&export_path, e))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| GenerationError::Seed(format!("JSON d'inspection illisible : {e}")))
}

/// Efface le dossier `generations/<horodatage>/` d'une génération dont on n'a plus besoin
/// (CI du registre : on ne garde que le verdict). Refuse tout autre dossier.
pub fn discard_generation(paths: &Paths, output: &GenerateOutput) -> Result<(), GenerationError> {
    // `zip_file` = `generations/<horodatage>/output/AP_<seed>.zip`.
    let dir = output
        .zip_file
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| GenerationError::UnsafeOutput(output.zip_file.clone()))?;
    remove_generation_dir(paths, dir)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Faux démarrage : échoue avec `first` au premier appel, réussit au second ; note
    /// chaque fois si le bac à sable était demandé.
    fn fake_start(
        first: BridgeError,
        calls: &mut Vec<bool>,
    ) -> impl FnMut(BridgeOptions) -> Result<&'static str, BridgeError> + '_ {
        let mut first = Some(first);
        move |options: BridgeOptions| {
            calls.push(options.sandbox.is_some());
            match first.take() {
                Some(e) => Err(e),
                None => Ok("démarré"),
            }
        }
    }

    fn with_sandbox() -> BridgeOptions {
        let mut options = BridgeOptions::new("python.exe");
        options.sandbox = Some(Sandbox::new("generations/1"));
        options
    }

    #[test]
    fn bac_a_sable_impossible_relance_sans_lui() {
        let (log, lines) = std::sync::mpsc::channel();
        let mut calls = Vec::new();
        let started = start_with_sandbox_fallback(
            with_sandbox(),
            &log,
            fake_start(
                BridgeError::SandboxSetup("icacls : FAT32".into()),
                &mut calls,
            ),
        );
        assert_eq!(started.unwrap(), "démarré");
        assert_eq!(calls, [true, false], "relancé une fois, sans bac à sable");
        let line = lines.try_recv().unwrap();
        assert!(
            line.contains("Bac à sable impossible") && line.contains("FAT32"),
            "{line}"
        );
    }

    #[test]
    fn autre_erreur_rendue_sans_relance() {
        let (log, lines) = std::sync::mpsc::channel();
        let mut calls = Vec::new();
        let started = start_with_sandbox_fallback(
            with_sandbox(),
            &log,
            fake_start(
                BridgeError::SandboxNotApplied { integrity: None },
                &mut calls,
            ),
        );
        // Un worker qui démarre hors de son bac à sable reste une erreur : pas de repli.
        assert!(matches!(
            started,
            Err(BridgeError::SandboxNotApplied { .. })
        ));
        assert_eq!(calls, [true]);
        assert!(lines.try_recv().is_err());
    }

    #[test]
    fn remove_generation_dir_refuse_un_dossier_hors_de_generations() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::under(tmp.path());
        let root = paths.generations_dir();
        let intrus = tmp.path().join("important");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&intrus).unwrap();
        fs::write(intrus.join("a.txt"), b"x").unwrap();
        // Hors de `generations/`, le dossier même `generations/`, un petit-enfant : refusés.
        let petit_enfant = root.join("g1").join("output");
        fs::create_dir_all(&petit_enfant).unwrap();
        for dir in [&intrus, &root, &petit_enfant] {
            assert!(remove_generation_dir(&paths, dir).is_err(), "{dir:?}");
        }
        assert!(intrus.join("a.txt").is_file() && petit_enfant.is_dir());
        // Un enfant direct est supprimé.
        remove_generation_dir(&paths, &root.join("g1")).unwrap();
        assert!(!root.join("g1").exists());
    }
}
