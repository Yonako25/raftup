//! Journal du serveur gardé en mémoire, pour le panneau « Journal du serveur ».
//!
//! Le serveur écrit ses messages avec `tracing` (`info!`, `warn!`…). On
//! branche ici une « couche » (`Layer`) qui range chaque message dans un
//! tampon de taille fixe ; l'interface lit les lignes nouvelles.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Lignes gardées au plus : les plus anciennes partent.
const CAPACITY: usize = 2000;

/// Une ligne du journal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogLine {
    /// Numéro croissant : sert à demander « les lignes après la n° X ».
    pub number: u64,
    /// Secondes depuis 1970 ; l'interface l'affiche à l'heure locale.
    pub time: f64,
    /// `INFO`, `WARN`, `ERROR`, `DEBUG`.
    pub level: String,
    /// Le message : `joueur connecté`.
    pub text: String,
    /// Ses champs, pour la case « Détails » : `id=1 slot=1` (vide s'il n'y en a pas).
    pub fields: String,
}

#[derive(Debug, Default)]
struct Inner {
    lines: VecDeque<LogLine>,
    next: u64,
}

/// Tampon partagé entre la couche `tracing` et l'interface.
///
/// `Clone` copie seulement le pointeur (`Arc`) : toutes les copies voient le
/// même tampon. Le `Mutex` protège les écritures venues de plusieurs tâches.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
}

impl LogBuffer {
    pub fn new() -> LogBuffer {
        LogBuffer::default()
    }

    /// La couche à brancher sur `tracing_subscriber::registry()`.
    pub fn layer(&self) -> LogLayer {
        LogLayer {
            buffer: self.clone(),
        }
    }

    /// Ajoute une ligne (utile aussi pour les messages du launcher lui-même).
    pub fn push(&self, level: &str, text: String) {
        self.push_with_fields(level, text, String::new());
    }

    fn push_with_fields(&self, level: &str, text: String, fields: String) {
        // Un `Mutex` « empoisonné » (une tâche a paniqué en le tenant) garde des
        // données valables ici : on les reprend.
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let number = inner.next;
        inner.next += 1;
        inner.lines.push_back(LogLine {
            number,
            time: now(),
            level: level.to_string(),
            text,
            fields,
        });
        while inner.lines.len() > CAPACITY {
            inner.lines.pop_front();
        }
    }

    /// Lignes dont le numéro est au moins `from`.
    pub fn since(&self, from: u64) -> Vec<LogLine> {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner
            .lines
            .iter()
            .filter(|l| l.number >= from)
            .cloned()
            .collect()
    }
}

/// Couche `tracing` qui écrit dans un [`LogBuffer`].
pub struct LogLayer {
    buffer: LogBuffer,
}

// `impl<S> Layer<S> for … where S: Subscriber` : la couche marche avec
// n'importe quel « abonné » tracing, c'est ce que demande la bibliothèque.
impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();
        if level > Level::INFO {
            return; // DEBUG et TRACE : trop bavards pour l'interface.
        }
        // Seulement le serveur et le launcher : pas les messages internes des
        // bibliothèques (carte graphique, fenêtre…).
        let target = event.metadata().target();
        if !(target.starts_with("server") || target.starts_with("launcher")) {
            return;
        }
        let mut visitor = TextVisitor::default();
        event.record(&mut visitor);
        let fields = visitor.fields.trim_start().to_string();
        self.buffer
            .push_with_fields(level.as_str(), visitor.message, fields);
    }
}

/// Écrit le message, puis les champs sous la forme ` clé=valeur`.
#[derive(Default)]
struct TextVisitor {
    message: String,
    fields: String,
}

impl Visit for TextVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn garde_les_messages() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(id = 3, name = "Alice", "joueur connecté");
            tracing::debug!("trop bavard");
            // Message d'une bibliothèque : écarté du journal de l'interface.
            tracing::info!(target: "wgpu_hal::vulkan", "bruit de la carte graphique");
            tracing::warn!("fermeture");
        });
        let lines = buffer.since(0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].level, "INFO");
        assert_eq!(lines[0].text, "joueur connecté");
        assert_eq!(lines[0].fields, "id=3 name=Alice");
        assert_eq!(lines[1].level, "WARN");
        assert_eq!(buffer.since(1).len(), 1);
    }

    #[test]
    fn taille_bornee() {
        let buffer = LogBuffer::new();
        for i in 0..CAPACITY + 10 {
            buffer.push("INFO", format!("ligne {i}"));
        }
        let lines = buffer.since(0);
        assert_eq!(lines.len(), CAPACITY);
        assert_eq!(lines[0].number, 10);
    }
}
