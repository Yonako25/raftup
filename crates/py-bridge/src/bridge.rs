//! Pilotage du process Python : démarrage, appels, arrêt (ADR 0016).
//!
//! Tout est synchrone : un appel à la fois. Deux threads de lecture (stdout et stderr)
//! évitent qu'un pipe plein bloque le worker. Depuis du code async, appeler le pont
//! via `spawn_blocking`.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use crate::cancel::CancelToken;
use crate::error::BridgeError;
use crate::job::KillGuard;
use crate::line::{read_bounded_bytes, read_bounded_line, LineError};
use crate::protocol::{decode_response, encode_request};
use crate::{DEFAULT_MAX_LINE_BYTES, WORKER_PROTOCOL_VERSION, WORKER_SCRIPT};

/// Nombre maximal de lignes de stderr gardées, et taille maximale cumulée.
const STDERR_TAIL_LINES: usize = 50;
const STDERR_TAIL_BYTES: usize = 8 * 1024;
/// Délai laissé à Drop pour que le worker quitte seul après la fermeture de stdin.
const DROP_GRACE: Duration = Duration::from_millis(200);
/// Temps laissé, après une anomalie, pour récupérer le statut du process et la fin de stderr.
const EXIT_WINDOW: Duration = Duration::from_secs(2);
/// Pas d'attente quand on surveille la fin d'un process.
const POLL_STEP: Duration = Duration::from_millis(10);
/// Tranche maximale d'une attente de réponse : l'annulation est vérifiée à chaque tranche.
const CANCEL_SLICE: Duration = Duration::from_millis(100);

/// Réglages du pont.
#[derive(Debug, Clone)]
pub struct BridgeOptions {
    /// Exécutable Python à lancer.
    pub python: PathBuf,
    /// Script exécuté par `python -c` (le worker embarqué par défaut).
    pub script: String,
    pub working_dir: Option<PathBuf>,
    /// Variables ajoutées à l'environnement hérité. Attention : toutes les variables
    /// `PYTHON*` (`PYTHONPATH`, `PYTHONHOME`...), héritées ou données ici, sont RETIRÉES ;
    /// seule `PYTHONHASHSEED=0` est posée par le pont. Les autres variables (par exemple
    /// `PATH`) passent normalement.
    pub env: BTreeMap<String, String>,
    pub startup_timeout: Duration,
    pub call_timeout: Duration,
    pub shutdown_grace: Duration,
    pub max_line_bytes: usize,
    /// Si présent, chaque ligne de stderr y est aussi envoyée (en plus du tail et de `tracing`).
    /// Un récepteur disparu est ignoré : cela ne bloque jamais la lecture.
    pub stderr_lines: Option<Sender<String>>,
}

impl BridgeOptions {
    pub fn new(python: impl Into<PathBuf>) -> Self {
        BridgeOptions {
            python: python.into(),
            script: WORKER_SCRIPT.to_string(),
            working_dir: None,
            env: BTreeMap::new(),
            startup_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(10),
            shutdown_grace: Duration::from_secs(2),
            max_line_bytes: DEFAULT_MAX_LINE_BYTES,
            stderr_lines: None,
        }
    }
}

/// Réponse de la méthode `version`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WorkerVersion {
    pub python: String,
    pub implementation: String,
    pub protocol: u32,
    /// `sys.flags.hash_randomization` : 0 quand `PYTHONHASHSEED=0` a bien été pris en compte.
    /// `None` pour un worker plus ancien qui ne le dit pas.
    #[serde(default)]
    pub hash_randomization: Option<u32>,
}

/// Ce que le thread de lecture de stdout transmet au pont.
enum Event {
    Line(String),
    Eof,
    Error(LineError),
}

/// Dernières lignes de stderr, partagées entre le thread de lecture et le pont.
#[derive(Default)]
struct StderrTail {
    lines: VecDeque<String>,
    bytes: usize,
}

impl StderrTail {
    fn push(&mut self, line: String) {
        // Une ligne de 8 Kio exactement (avec son `\n`) dépasserait la borne et serait retirée
        // aussitôt : on la TRONQUE plutôt, pour toujours garder la dernière ligne reçue.
        let line = truncate_utf8(line, STDERR_TAIL_BYTES - 1);
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.lines.len() > STDERR_TAIL_LINES || self.bytes > STDERR_TAIL_BYTES {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.len() + 1,
                None => break,
            }
        }
    }

    fn text(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

/// Tronque à `max` octets au plus, sans couper un caractère UTF-8 en deux.
fn truncate_utf8(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        // `is_char_boundary` : vrai si `cut` tombe entre deux caractères.
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

/// Pont vers un process Python.
///
/// `Send` automatiquement : `Child`, `ChildStdin` et `Receiver` le sont. Pas `Sync`
/// (le `Receiver`), donc un seul thread à la fois, ce qui convient à un appel à la fois.
pub struct PyBridge {
    child: Child,
    // Un thread dédié possède `ChildStdin` et écrit les requêtes : un worker qui ne lit pas
    // stdin ne peut donc pas bloquer l'appelant (qui attend l'accusé avec un délai).
    // `Option` pour fermer stdin : lâcher l'émetteur arrête le thread, qui lâche `ChildStdin`.
    writer: Option<Sender<Vec<u8>>>,
    // Accusé du thread d'écriture : `Ok(())` une fois la requête écrite, ou l'erreur d'E/S.
    write_acks: Receiver<io::Result<()>>,
    events: Receiver<Event>,
    // `Arc<Mutex<..>>` : la même donnée est écrite par le thread stderr et lue ici.
    stderr_tail: Arc<Mutex<StderrTail>>,
    stderr_done: Arc<AtomicBool>,
    next_id: u64,
    closed: bool,
    options: BridgeOptions,
    // Job Object Windows : le fermer tue le worker et sa descendance (voir `job.rs`).
    job: Option<KillGuard>,
}

impl PyBridge {
    /// Lance le process, puis vérifie la version du protocole avec l'appel `version`.
    pub fn start(options: BridgeOptions) -> Result<Self, BridgeError> {
        let mut bridge = Self::spawn(options)?;
        let timeout = bridge.options.startup_timeout;
        let value = bridge.call_with_timeout("version", Value::Null, timeout)?;
        let version: WorkerVersion = serde_json::from_value(value).map_err(|e| {
            bridge.close();
            BridgeError::InvalidResponse(format!("réponse `version` invalide : {e}"))
        })?;
        if version.protocol != WORKER_PROTOCOL_VERSION {
            bridge.close();
            return Err(BridgeError::IncompatibleWorker {
                expected: WORKER_PROTOCOL_VERSION,
                got: version.protocol,
            });
        }
        Ok(bridge)
    }

    /// Lance le process sans vérifier la version (la poignée de main est faite par `start`).
    fn spawn(mut options: BridgeOptions) -> Result<Self, BridgeError> {
        // Sous Windows, un `.bat`/`.cmd` passerait par `cmd.exe` et ses règles de guillemets :
        // on exige un vrai `.exe`.
        #[cfg(windows)]
        if let Some(ext) = options.python.extension().and_then(|e| e.to_str()) {
            if ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("cmd") {
                return Err(BridgeError::UnsupportedPython {
                    path: options.python.clone(),
                });
            }
        }
        let mut command = Command::new(&options.python);
        // Isolation (ADR 0016, décision 4, révisée par l'ADR 0028) : `-s` (pas de dossier
        // `site` de l'utilisateur) et `-P` (pas de dossier courant dans `sys.path`), comme
        // `-I`, mais SANS `-E` : on retire nous-mêmes toutes les variables `PYTHON*`, puis
        // on fixe `PYTHONHASHSEED=0` pour que l'ordre des `set` Python soit reproductible.
        for (key, _) in std::env::vars_os() {
            if is_python_var(&key) {
                command.env_remove(&key);
            }
        }
        let env = options
            .env
            .iter()
            .filter(|(k, _)| !is_python_var(std::ffi::OsStr::new(k.as_str())));
        // Pas de shell : les arguments sont passés tels quels, sans interprétation.
        command
            .args(["-s", "-P", "-X", "utf8", "-c", &options.script])
            .envs(env)
            .env("PYTHONHASHSEED", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &options.working_dir {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW : pas de fenêtre console qui clignote.
            command.creation_flags(0x0800_0000);
        }

        let mut child = command.spawn().map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound && !options.python.is_file() {
                BridgeError::PythonNotFound {
                    path: options.python.clone(),
                }
            } else {
                BridgeError::Spawn {
                    path: options.python.clone(),
                    source,
                }
            }
        })?;

        // Affectation au job tout de suite après le lancement, avant que le worker ait pu
        // lancer des petits-enfants.
        let job = KillGuard::attach(&child);

        let (stdin, stdout, stderr) =
            match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(i), Some(o), Some(e)) => (i, o, e),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(BridgeError::Io {
                        context: "pipes du process",
                        source: io::Error::other("pipes indisponibles"),
                    });
                }
            };

        // Thread d'écriture : il reçoit les requêtes (octets) et renvoie un accusé par requête.
        // Il se termine quand le pont lâche l'émetteur, ou à la première erreur d'écriture.
        let (writer, requests) = mpsc::channel::<Vec<u8>>();
        let (ack_tx, write_acks) = mpsc::channel::<io::Result<()>>();
        thread::spawn(move || {
            let mut stdin = stdin;
            // `for` sur un `Receiver` : boucle jusqu'à ce que tous les émetteurs soient lâchés.
            for bytes in requests {
                let result = stdin.write_all(&bytes).and_then(|_| stdin.flush());
                let failed = result.is_err();
                if ack_tx.send(result).is_err() || failed {
                    break;
                }
            }
        });

        // mpsc = file de messages entre threads : le thread envoie, le pont reçoit.
        let (tx, events) = mpsc::channel::<Event>();
        let max = options.max_line_bytes;
        // Ces threads ne sont jamais `join` : ils se terminent seuls quand le process
        // meurt (pipe fermé). Un `join` pourrait bloquer si un petit-fils garde le pipe.
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let event = match read_bounded_line(&mut reader, max) {
                    Ok(Some(line)) => Event::Line(line),
                    Ok(None) => Event::Eof,
                    Err(e) => Event::Error(e),
                };
                let last = !matches!(event, Event::Line(_));
                // Si le pont a disparu, `send` échoue : on s'arrête.
                if tx.send(event).is_err() || last {
                    break;
                }
            }
        });

        let stderr_tail = Arc::new(Mutex::new(StderrTail::default()));
        let stderr_done = Arc::new(AtomicBool::new(false));
        // `take()` sort l'émetteur des options : seul le thread stderr le garde, donc le canal
        // se ferme (le récepteur voit la fin) quand stderr atteint EOF.
        let stderr_lines = options.stderr_lines.take();
        {
            // `Arc::clone` copie le pointeur, pas les données : les deux côtés partagent le même `Mutex`.
            let tail = Arc::clone(&stderr_tail);
            let done = Arc::clone(&stderr_done);
            thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                loop {
                    // Lecture en octets : un affichage non UTF-8 est converti avec perte
                    // (`from_utf8_lossy`) au lieu d'être jeté.
                    match read_bounded_bytes(&mut reader, STDERR_TAIL_BYTES) {
                        Ok(Some(bytes)) => {
                            let line = String::from_utf8_lossy(&bytes).into_owned();
                            tracing::debug!(target: "py_bridge::stderr", "{line}");
                            if let Some(tx) = &stderr_lines {
                                // Canal non borné : `send` ne bloque jamais ; une erreur veut
                                // dire que le récepteur a disparu, on l'ignore.
                                let _ = tx.send(line.clone());
                            }
                            tail.lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .push(line);
                        }
                        Ok(None) | Err(LineError::Io(_)) => break,
                        // Ligne trop longue : on ignore et on continue.
                        Err(_) => {}
                    }
                }
                done.store(true, Ordering::SeqCst);
            });
        }

        Ok(PyBridge {
            child,
            writer: Some(writer),
            write_acks,
            events,
            stderr_tail,
            stderr_done,
            next_id: 1,
            closed: false,
            options,
            job: Some(job),
        })
    }

    /// Appelle une méthode avec le délai par défaut.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, BridgeError> {
        let timeout = self.options.call_timeout;
        self.call_with_timeout(method, params, timeout)
    }

    /// Appelle une méthode. Toute anomalie de flux ferme le pont ; une erreur du
    /// worker (`BridgeError::Worker`) le laisse utilisable.
    pub fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, BridgeError> {
        self.call_inner(method, params, timeout, None)
    }

    /// Comme `call_with_timeout`, mais annulable : l'attente est découpée en tranches de
    /// 100 ms au plus. Si `cancel` est annulé, le process est tué (avec sa descendance),
    /// le pont est fermé et l'appel renvoie `BridgeError::Cancelled` en moins d'une seconde.
    pub fn call_cancellable(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> Result<Value, BridgeError> {
        self.call_inner(method, params, timeout, Some(cancel))
    }

    /// Code commun aux appels avec ou sans annulation.
    fn call_inner(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: Option<&CancelToken>,
    ) -> Result<Value, BridgeError> {
        if self.closed {
            return Err(BridgeError::Closed);
        }
        let result = self.exchange(method, &params, timeout, cancel);
        // Une requête refusée AVANT l'écriture ne touche pas au flux : le pont reste ouvert.
        if !matches!(
            result,
            Ok(_) | Err(BridgeError::Worker(_)) | Err(BridgeError::RequestTooLarge { .. })
        ) {
            self.close();
        }
        result
    }

    pub fn ping(&mut self) -> Result<(), BridgeError> {
        self.call("ping", Value::Null).map(|_| ())
    }

    pub fn version(&mut self) -> Result<WorkerVersion, BridgeError> {
        let value = self.call("version", Value::Null)?;
        serde_json::from_value(value).map_err(|e| {
            // Réponse incompréhensible : le worker n'est pas fiable, on ferme le pont.
            self.close();
            BridgeError::InvalidResponse(format!("réponse `version` invalide : {e}"))
        })
    }

    /// Vrai si le pont est ouvert et que le process tourne encore.
    pub fn is_alive(&mut self) -> bool {
        !self.closed && matches!(self.child.try_wait(), Ok(None))
    }

    /// Ferme stdin, laisse `shutdown_grace` au worker pour quitter, puis le tue.
    /// Prend `self` par valeur : le pont n'est plus utilisable après.
    pub fn shutdown(mut self) -> Result<ExitStatus, BridgeError> {
        self.closed = true;
        self.writer = None;
        let grace = self.options.shutdown_grace;
        // À la sortie, `Drop` s'exécute mais ne fait rien (process déjà terminé).
        self.wait_or_kill(grace)
    }

    /// Une requête, une réponse.
    fn exchange(
        &mut self,
        method: &str,
        params: &Value,
        timeout: Duration,
        cancel: Option<&CancelToken>,
    ) -> Result<Value, BridgeError> {
        let id = self.next_id;
        self.next_id += 1;
        // Le worker exige un objet JSON pour `params` : `null` devient `{}`.
        let empty = Value::Object(serde_json::Map::new());
        let params = if params.is_null() { &empty } else { params };
        let mut line = encode_request(id, method, params)?;
        // Trop gros : refusé AVANT d'écrire (le worker coupe les lignes trop longues).
        if line.len() > self.options.max_line_bytes {
            return Err(BridgeError::RequestTooLarge {
                max: self.options.max_line_bytes,
                len: line.len(),
            });
        }
        line.push('\n');

        // Un seul délai pour l'écriture ET l'attente de la réponse.
        let deadline = Instant::now() + timeout;
        let timed_out = || BridgeError::Timeout {
            method: method.to_string(),
            after: timeout,
        };
        let Some(writer) = self.writer.as_ref() else {
            return Err(BridgeError::Closed);
        };
        if writer.send(line.into_bytes()).is_err() {
            return Err(self.process_exited());
        }
        match recv_sliced(&self.write_acks, deadline, cancel) {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(Wait::Disconnected) => return Err(self.process_exited()),
            Err(Wait::Timeout) => return Err(timed_out()),
            Err(Wait::Cancelled) => return Err(BridgeError::Cancelled),
        }

        let event = match recv_sliced(&self.events, deadline, cancel) {
            Ok(event) => event,
            Err(Wait::Timeout) => return Err(timed_out()),
            Err(Wait::Cancelled) => return Err(BridgeError::Cancelled),
            Err(Wait::Disconnected) => Event::Eof,
        };
        let text = match event {
            Event::Line(text) => text,
            Event::Eof | Event::Error(LineError::Io(_)) => return Err(self.process_exited()),
            Event::Error(LineError::TooLong { max }) => {
                return Err(BridgeError::LineTooLong { max })
            }
            Event::Error(LineError::InvalidUtf8) => {
                return Err(BridgeError::InvalidResponse(
                    "ligne qui n'est pas de l'UTF-8 valide".into(),
                ))
            }
        };

        let (got, outcome) = decode_response(&text)?;
        // `id: null` avec `error` : le worker n'a pas pu lire la requête ; on garde son message.
        if let (None, Err(rpc)) = (got, &outcome) {
            return Err(BridgeError::Protocol(rpc.clone()));
        }
        if got != Some(id) {
            return Err(BridgeError::UnexpectedId { expected: id, got });
        }
        outcome.map_err(BridgeError::from)
    }

    /// Construit l'erreur `ProcessExited` : statut du process et fin de stderr.
    fn process_exited(&mut self) -> BridgeError {
        let deadline = Instant::now() + EXIT_WINDOW;
        let mut status = None;
        while status.is_none() && Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(s)) => status = Some(s),
                Ok(None) => thread::sleep(POLL_STEP),
                Err(_) => break,
            }
        }
        // Laisse au thread stderr le temps de finir de lire (borné).
        while !self.stderr_done.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(POLL_STEP);
        }
        let stderr_tail = self
            .stderr_tail
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .text();
        BridgeError::ProcessExited {
            status,
            stderr_tail,
        }
    }

    /// Passe le pont à `Closed` et tue le process.
    fn close(&mut self) {
        self.closed = true;
        self.writer = None;
        // Fermer le job (Windows) tue toute la descendance ; puis on tue le worker lui-même.
        self.job = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Attend la fin du process pendant `grace`, puis le tue.
    fn wait_or_kill(&mut self, grace: Duration) -> Result<ExitStatus, BridgeError> {
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL_STEP),
                Ok(None) => break,
                Err(source) => return Err(wait_error(source)),
            }
        }
        let _ = self.child.kill();
        self.child.wait().map_err(wait_error)
    }
}

/// Pourquoi une attente de message s'est arrêtée sans message.
enum Wait {
    Timeout,
    Cancelled,
    Disconnected,
}

/// Attend un message jusqu'à `deadline`, par tranches de `CANCEL_SLICE` au plus, en
/// vérifiant l'annulation entre chaque tranche. Générique (`T`) : sert pour deux types de canaux.
fn recv_sliced<T>(
    rx: &Receiver<T>,
    deadline: Instant,
    cancel: Option<&CancelToken>,
) -> Result<T, Wait> {
    loop {
        if cancel.is_some_and(CancelToken::is_cancelled) {
            return Err(Wait::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(CANCEL_SLICE)) {
            Ok(value) => return Ok(value),
            Err(RecvTimeoutError::Disconnected) => return Err(Wait::Disconnected),
            Err(RecvTimeoutError::Timeout) if remaining <= CANCEL_SLICE => {
                return Err(Wait::Timeout)
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Variable d'environnement de Python (`PYTHONPATH`, `PYTHONHOME`…), quelle que soit la
/// casse : sous Windows, les noms de variables ne distinguent pas majuscules et minuscules.
fn is_python_var(key: &std::ffi::OsStr) -> bool {
    key.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("PYTHON")
}

fn wait_error(source: io::Error) -> BridgeError {
    BridgeError::Io {
        context: "attente de la fin du process",
        source,
    }
}

// Drop : code exécuté quand la valeur sort de portée. On évite de laisser un process
// orphelin, sans jamais attendre indéfiniment ni `join` les threads de lecture.
impl Drop for PyBridge {
    fn drop(&mut self) {
        self.writer = None;
        let deadline = Instant::now() + DROP_GRACE;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(POLL_STEP);
        }
        // Fermer le job tue aussi la descendance (Windows).
        self.job = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_python_reconnues() {
        use std::ffi::OsStr;
        assert!(is_python_var(OsStr::new("PYTHONPATH")));
        assert!(is_python_var(OsStr::new("pythonhome")));
        assert!(is_python_var(OsStr::new("PYTHONHASHSEED")));
        assert!(!is_python_var(OsStr::new("PATH")));
        assert!(!is_python_var(OsStr::new("MY_PYTHONPATH")));
    }

    #[test]
    fn ligne_de_8_kio_exactement_est_tronquee_pas_retiree() {
        let mut tail = StderrTail::default();
        tail.push("a".repeat(STDERR_TAIL_BYTES));
        assert!(!tail.text().is_empty());
        assert!(tail.bytes <= STDERR_TAIL_BYTES);
        assert!(tail.text().starts_with('a'));
    }

    #[test]
    fn la_borne_totale_est_respectee() {
        let mut tail = StderrTail::default();
        for i in 0..1000 {
            tail.push(format!("{i} {}", "x".repeat(500)));
        }
        assert!(tail.bytes <= STDERR_TAIL_BYTES);
        assert!(tail.lines.len() <= STDERR_TAIL_LINES);
        // La dernière ligne est toujours conservée.
        assert!(tail.text().contains("999 "));
    }

    #[test]
    fn troncature_sans_couper_un_caractere() {
        let t = truncate_utf8("ééé".to_string(), 3);
        assert_eq!(t, "é");
    }
}
