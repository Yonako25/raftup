//! Le « worker » : fait le lien entre l'interface (synchrone) et `launcher-core` (async).
//!
//! egui redessine l'écran plusieurs fois par seconde dans un seul thread : il ne peut pas
//! attendre (`.await`) la réponse du serveur. Le worker est donc une tâche `tokio` qui :
//! - reçoit des commandes (démarrer, arrêter, annoncer) par un canal ;
//! - une fois par seconde, lit l'état de la partie et le journal, et les range dans un
//!   état partagé que l'interface lit sans attendre ;
//! - demande ensuite un nouveau dessin (`request_repaint`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use std::path::PathBuf;

use launcher_core::{
    Host, HostOptions, HostStatus, LogBuffer, LogLine, NumberedEvent, Readiness, SeedSummary,
};
use tokio::sync::{mpsc, oneshot};

/// Nombre maximal d'événements et de lignes de journal gardés pour l'affichage.
const KEEP: usize = 2000;

/// Ce que l'interface demande au worker.
pub enum Command {
    Start(HostOptions),
    Stop,
    Announce(String),
    /// La fenêtre se ferme : arrêter proprement la partie, puis le worker.
    Shutdown,
    /// Lire et résumer une seed, hors du fil de l'interface.
    LoadSeed(PathBuf),
    /// Tester « prêt ? » d'un slot par une vraie connexion (F08).
    CheckReady(ReadyCheck),
}

/// Ce qu'il faut pour tester un slot. Le mot de passe n'est pas dans `HostStatus` :
/// l'hôte le retape dans le salon (il n'est jamais enregistré).
pub struct ReadyCheck {
    pub address: String,
    pub slot: String,
    pub game: String,
    pub password: Option<String>,
    /// Empreinte du certificat généré par Raftup (ADR 0043) : le test passe en `wss`
    /// épinglé. `None` : vérification habituelle.
    pub fingerprint: Option<String>,
}

/// Où en est le test d'un slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyState {
    Checking,
    Done(Readiness),
}

/// L'état lu par l'interface.
#[derive(Default)]
pub struct Shared {
    /// Photo de la partie hébergée ; `None` s'il n'y en a pas.
    pub status: Option<HostStatus>,
    /// Événements de la partie, du plus ancien au plus récent.
    pub events: Vec<NumberedEvent>,
    /// Lignes du journal du serveur.
    pub logs: Vec<LogLine>,
    /// Un démarrage est en cours.
    pub starting: bool,
    /// Dernière erreur d'une commande, à montrer une fois.
    pub error: Option<String>,
    /// Dernière seed lue : son chemin et son résumé, ou le message d'erreur.
    pub seed: Option<(PathBuf, Result<SeedSummary, String>)>,
    /// Résultat des tests « prêt ? », par nom de slot (F08).
    pub readiness: BTreeMap<String, ReadyState>,
    /// Heure (secondes depuis 1970) de la fin du dernier test de chaque slot.
    pub readiness_at: BTreeMap<String, f64>,
}

/// Poignée côté interface.
pub struct Worker {
    commands: mpsc::UnboundedSender<Command>,
    // `Arc<Mutex<…>>` : l'état est partagé entre le worker (qui écrit) et l'interface
    // (qui lit) ; le `Mutex` garantit qu'un seul des deux y touche à la fois.
    shared: Arc<Mutex<Shared>>,
}

impl Worker {
    /// Lance la tâche du worker sur le runtime `tokio`.
    ///
    /// `done` est prévenu quand le worker a fini, partie arrêtée : `main`
    /// l'attend avant de détruire le runtime.
    pub fn spawn(
        runtime: &tokio::runtime::Handle,
        ctx: egui::Context,
        logs: LogBuffer,
        done: oneshot::Sender<()>,
    ) -> Worker {
        let (commands, receiver) = mpsc::unbounded_channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        runtime.spawn(run(receiver, Arc::clone(&shared), ctx, logs, done));
        Worker { commands, shared }
    }

    /// Tests de l'interface (`ui_tests`) : un worker sans tâche ni runtime `tokio`.
    /// Le récepteur du canal est jeté tout de suite : les commandes envoyées sont
    /// ignorées (comme après la fermeture de la fenêtre) et rien ne démarre.
    #[cfg(test)]
    pub fn detached() -> Worker {
        let (commands, _receiver) = mpsc::unbounded_channel();
        Worker {
            commands,
            shared: Arc::new(Mutex::new(Shared::default())),
        }
    }

    /// Envoie une commande ; si le worker est arrêté (fermeture), on l'ignore.
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Accès à l'état partagé, le temps d'un dessin.
    pub fn lock(&self) -> MutexGuard<'_, Shared> {
        // Un `Mutex` « empoisonné » (un thread a paniqué en le tenant) garde des
        // données exploitables ici : on les reprend.
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reprend la dernière erreur, s'il y en a une.
    pub fn take_error(&self) -> Option<String> {
        self.lock().error.take()
    }
}

/// Boucle du worker : commandes et rafraîchissement chaque seconde.
async fn run(
    mut commands: mpsc::UnboundedReceiver<Command>,
    shared: Arc<Mutex<Shared>>,
    ctx: egui::Context,
    logs: LogBuffer,
    done: oneshot::Sender<()>,
) {
    let mut host = Host::new();
    let mut next_event = 0u64;
    let mut next_log = 0u64;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // `select!` attend la première des deux : une commande ou le tic de la seconde.
        tokio::select! {
            command = commands.recv() => {
                // Canal fermé (l'interface est détruite) ou fermeture demandée.
                let command = match command {
                    None | Some(Command::Shutdown) => break,
                    Some(command) => command,
                };
                handle(command, &mut host, &shared, &mut next_event, &ctx).await;
            }
            _ = tick.tick() => {}
        }
        refresh(&mut host, &shared, &logs, &mut next_event, &mut next_log).await;
        ctx.request_repaint();
    }
    // Arrêt propre : la salle reçoit `Stop`, compacte sa sauvegarde et ferme
    // les connexions, au lieu d'être coupée net avec le runtime.
    if host.is_running() {
        let _ = host.stop().await;
    }
    // `main` attend peut-être ce signal ; s'il ne l'attend plus, tant pis.
    let _ = done.send(());
}

fn with_shared<R>(shared: &Mutex<Shared>, f: impl FnOnce(&mut Shared) -> R) -> R {
    f(&mut shared.lock().unwrap_or_else(|e| e.into_inner()))
}

async fn handle(
    command: Command,
    host: &mut Host,
    shared: &Arc<Mutex<Shared>>,
    next_event: &mut u64,
    ctx: &egui::Context,
) {
    match command {
        Command::Start(options) => {
            // Partie déjà lancée : ce `Start` vient d'un double clic envoyé
            // avant que l'écran change. On l'ignore au lieu d'afficher
            // « une partie est déjà hébergée ».
            if host.is_running() {
                return;
            }
            with_shared(shared, |s| {
                s.starting = true;
                s.readiness.clear();
                s.readiness_at.clear();
                s.error = None;
            });
            let result = host.start(options).await;
            with_shared(shared, |s| {
                s.starting = false;
                match result {
                    Ok(status) => {
                        s.status = Some(status);
                        s.events.clear();
                        *next_event = 0;
                    }
                    Err(e) => s.error = Some(e.to_string()),
                }
            });
        }
        Command::Stop => {
            let result = host.stop().await;
            with_shared(shared, |s| {
                s.status = None;
                s.events.clear();
                s.readiness.clear();
                s.readiness_at.clear();
                *next_event = 0;
                if let Err(e) = result {
                    s.error = Some(e.to_string());
                }
            });
        }
        Command::Announce(text) => {
            if let Err(e) = host.announce(text).await {
                with_shared(shared, |s| s.error = Some(e.to_string()));
            }
        }
        // Traitée dans la boucle de `run`, qui s'arrête avant d'appeler `handle`.
        Command::Shutdown => {}
        Command::LoadSeed(path) => {
            // Lecture de fichier + décompression + pickle : du travail qui
            // bloque. `spawn_blocking` le fait sur un thread prévu pour ça,
            // sans gêner les tâches async (le serveur, s'il tourne).
            let to_read = path.clone();
            let loaded = tokio::task::spawn_blocking(move || SeedSummary::load(&to_read)).await;
            let result = match loaded {
                Ok(Ok((summary, _data))) => Ok(summary),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(format!("lecture de la seed interrompue : {e}")),
            };
            with_shared(shared, |s| s.seed = Some((path, result)));
        }
        Command::CheckReady(check) => {
            let slot = check.slot.clone();
            // Jamais deux tests en même temps pour un slot (clics répétés, rafraîchissement
            // automatique) : si l'un tourne déjà, on ignore la demande.
            let already = with_shared(shared, |s| {
                let running = matches!(s.readiness.get(&slot), Some(ReadyState::Checking));
                if !running {
                    s.readiness.insert(slot.clone(), ReadyState::Checking);
                }
                running
            });
            if already {
                return;
            }
            // Tâche à part : le test peut durer 8 s et le worker doit continuer à traiter
            // les autres commandes. `Arc::clone` : la tâche garde sa propre part de l'état.
            let shared = Arc::clone(shared);
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let pinned = check
                    .fingerprint
                    .as_deref()
                    .and_then(launcher_core::Fingerprint::parse);
                let result = launcher_core::readiness::check_slot_pinned(
                    &check.address,
                    &check.slot,
                    &check.game,
                    check.password.as_deref(),
                    pinned,
                )
                .await;
                with_shared(&shared, |s| {
                    // Partie arrêtée entre-temps : le résultat n'a plus de sens.
                    if s.status.is_some() {
                        s.readiness_at.insert(slot.clone(), crate::format::now());
                        s.readiness.insert(slot, ReadyState::Done(result));
                    }
                });
                ctx.request_repaint();
            });
        }
    }
}

/// Recopie la photo, les nouveaux événements et les nouvelles lignes de journal.
async fn refresh(
    host: &mut Host,
    shared: &Mutex<Shared>,
    logs: &LogBuffer,
    next_event: &mut u64,
    next_log: &mut u64,
) {
    // On lit d'abord (avec `.await`, sans tenir le verrou), puis on écrit d'un coup.
    let status = if host.is_running() {
        host.status().await
    } else {
        None
    };
    let events = host.events_since(*next_event);
    let new_logs = logs.since(*next_log);
    // Hébergée mais muette : la salle s'est arrêtée seule (erreur d'écriture
    // de la sauvegarde, panique…). On libère le port au lieu d'afficher une
    // partie figée « en ligne ».
    let dead = host.is_running() && status.is_none();
    if dead {
        let _ = host.stop().await;
    }

    with_shared(shared, |s| {
        if dead {
            s.status = None;
            s.error = Some(
                "La partie s'est arrêtée d'elle-même. Le journal du serveur en donne la raison."
                    .to_string(),
            );
        } else if status.is_some() {
            s.status = status;
        }
        if let Some(last) = events.last() {
            *next_event = last.number + 1;
        }
        s.events.extend(events);
        trim(&mut s.events);
        if let Some(last) = new_logs.last() {
            *next_log = last.number + 1;
        }
        s.logs.extend(new_logs);
        trim(&mut s.logs);
    });
}

/// Garde les `KEEP` derniers éléments.
fn trim<T>(items: &mut Vec<T>) {
    if items.len() > KEEP {
        let extra = items.len() - KEEP;
        items.drain(..extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_garde_les_derniers() {
        let mut v: Vec<usize> = (0..KEEP + 5).collect();
        trim(&mut v);
        assert_eq!(v.len(), KEEP);
        assert_eq!(v[0], 5);
    }
}
