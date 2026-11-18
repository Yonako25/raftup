//! Webhook Discord : poster les événements clés d'une partie (F10).
//!
//! - [`Event`] : ce qu'on annonce (partie lancée, objet, objectif, partie terminée) ;
//! - [`event_from`] et [`all_goals_reached`] : fonctions pures qui branchent le serveur
//!   ([`GameEvent`], [`ServerStatus`]) sur ces événements ;
//! - [`format_payload`] : le JSON envoyé à Discord (français, noms échappés, 2000 caractères
//!   au plus, aucune mention `@everyone` / `@here`) ;
//! - [`WebhookUrl`] : l'adresse du webhook, validée, et **secrète** (jamais dans les logs) ;
//! - [`WebhookSender`] : file bornée + un fil d'exécution qui envoie au plus 1 message par
//!   seconde et retente après un 429. Un échec d'envoi ne bloque jamais la partie.
//!
//! Le branchement dans `Host` (créer l'`Event::PartyStarted`, appeler `event_from` sur chaque
//! `GameEvent`, `all_goals_reached` sur le statut) reste à faire côté appelant.

use std::fmt;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;
use server::{GameEvent, ServerStatus};

/// Longueur maximale d'un message Discord, en caractères.
const MAX_CONTENT_CHARS: usize = 2000;
/// Messages en attente au plus ; au-delà, les nouveaux sont abandonnés.
const QUEUE_CAPACITY: usize = 64;
/// Tentatives au plus pour un message (1 envoi + retries après 429).
const MAX_ATTEMPTS: u32 = 3;
/// Attente maximale acceptée après un 429, pour ne pas bloquer le fil d'envoi longtemps.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(30);

/// Ce qui empêche d'utiliser ou d'envoyer vers le webhook.
#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    /// L'adresse n'est pas `https://discord.com/api/webhooks/…` (ou `discordapp.com`).
    #[error("adresse de webhook refusée : il faut https://discord.com/api/webhooks/…")]
    InvalidUrl,
    /// Le réseau a échoué (message sans l'adresse, qui est secrète).
    #[error("envoi au webhook impossible : {0}")]
    Network(String),
    /// Discord a répondu autre chose qu'un succès.
    #[error("le webhook a répondu avec le code {0}")]
    Status(u16),
    /// Discord limite le débit encore après plusieurs tentatives.
    #[error("le webhook limite le débit (trop de tentatives)")]
    RateLimited,
}

/// Adresse d'un webhook Discord. C'est un secret : quiconque la connaît peut poster.
///
/// `Debug` est écrit à la main (pas `derive`) pour ne jamais afficher l'adresse ;
/// il n'y a volontairement pas de `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookUrl(String);

impl WebhookUrl {
    /// Valide l'adresse : HTTPS, hôte `discord.com` ou `discordapp.com`, chemin `/api/webhooks/…`.
    pub fn new(url: &str) -> Result<WebhookUrl, WebhookError> {
        let url = url.trim();
        // Le préfixe se termine par « / » : `discord.com.evil.org` ou `discord.com@evil.org`
        // ne le satisfont pas.
        const PREFIXES: [&str; 2] = [
            "https://discord.com/api/webhooks/",
            "https://discordapp.com/api/webhooks/",
        ];
        let rest = PREFIXES.iter().find_map(|p| url.strip_prefix(p));
        match rest {
            Some(rest)
                if !rest.is_empty()
                    && !rest.chars().any(|c| c.is_whitespace() || c.is_control()) =>
            {
                Ok(WebhookUrl(url.to_string()))
            }
            _ => Err(WebhookError::InvalidUrl),
        }
    }

    /// L'adresse en clair, pour la requête HTTP ou la sauvegarde dans la config. Ne pas logger.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for WebhookUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookUrl(<secret>)")
    }
}

/// Un événement à annoncer sur Discord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// La partie vient d'être lancée.
    PartyStarted { players: usize },
    /// Un objet a été trouvé et part vers un joueur.
    ItemSent {
        finder: String,
        receiver: String,
        item: String,
        location: String,
    },
    /// Un joueur a atteint son objectif.
    Goal { player: String },
    /// Tous les joueurs ont atteint leur objectif.
    PartyFinished,
}

/// Traduit un événement du serveur ; `None` pour ce qu'on n'annonce pas (connexions, chat).
///
/// On ne poste que les objets de progression (drapeau 1) : annoncer chaque objet inonderait
/// le salon, et Discord limite le débit.
pub fn event_from(event: &GameEvent) -> Option<Event> {
    match event {
        GameEvent::ItemSent {
            finder,
            receiver,
            item,
            location,
            flags,
            ..
        } if flags & 1 != 0 => Some(Event::ItemSent {
            finder: finder.clone(),
            receiver: receiver.clone(),
            item: item.clone(),
            location: location.clone(),
        }),
        GameEvent::Goal { name, .. } => Some(Event::Goal {
            player: name.clone(),
        }),
        _ => None,
    }
}

/// Vrai quand tous les vrais joueurs (hors groupes) ont atteint leur objectif (statut 30).
pub fn all_goals_reached(status: &ServerStatus) -> bool {
    let mut players = status.slots.iter().filter(|s| !s.is_group).peekable();
    players.peek().is_some() && players.all(|s| s.status == 30)
}

/// Échappe le Markdown de Discord dans un nom, pour qu'un joueur ne puisse pas mettre en forme
/// ni créer de lien dans le message. Les retours à la ligne deviennent des espaces.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '*' | '_' | '~' | '`' | '|' | '>' | '[' | ']' | '(' | ')' | '#' | '@' | ':'
            | '<' | '-' | '!' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Le texte du message (avant le JSON), en français et borné à 2000 caractères.
pub fn format_message(event: &Event) -> String {
    let text = match event {
        Event::PartyStarted { players } => {
            format!("**La partie est lancée** ({players} joueurs).")
        }
        Event::ItemSent {
            finder,
            receiver,
            item,
            location,
        } => format!(
            "**{}** a trouvé **{}** pour **{}** ({}).",
            escape(finder),
            escape(item),
            escape(receiver),
            escape(location)
        ),
        Event::Goal { player } => format!("**{}** a terminé son objectif !", escape(player)),
        Event::PartyFinished => "**Partie terminée** : tous les joueurs ont fini.".to_string(),
    };
    truncate_chars(&text, MAX_CONTENT_CHARS)
}

/// Coupe à `max` caractères (pas octets : on ne coupe jamais au milieu d'un accent) ; ajoute « … ».
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    // Ne pas finir sur un « \ » orphelin qui échapperait le « … ».
    while cut.ends_with('\\') {
        cut.pop();
    }
    cut.push('…');
    cut
}

/// Le corps JSON envoyé à Discord. `allowed_mentions.parse = []` interdit toute mention.
pub fn format_payload(event: &Event) -> String {
    json!({
        "content": format_message(event),
        "allowed_mentions": { "parse": [] },
    })
    .to_string()
}

/// Réponse HTTP utile à l'envoi.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// Secondes demandées par l'en-tête `Retry-After` (cas du 429).
    pub retry_after: Option<f64>,
}

/// Envoie un corps JSON à une adresse. Un trait pour remplacer le réseau dans les tests.
pub trait Transport: Send + Sync {
    fn post_json(&self, url: &WebhookUrl, body: &str) -> Result<Reply, WebhookError>;
}

/// Vrai client HTTP (`ureq`, le même que le téléchargeur), HTTPS seulement.
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new() -> HttpTransport {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .https_only(true)
            // Pas de redirection : on ne suit jamais le webhook ailleurs.
            .max_redirects(0)
            // On lit nous-mêmes le code (429 compris).
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            .build()
            .into();
        HttpTransport { agent }
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport::new()
    }
}

impl Transport for HttpTransport {
    fn post_json(&self, url: &WebhookUrl, body: &str) -> Result<Reply, WebhookError> {
        let response = self
            .agent
            .post(url.expose())
            .header("Content-Type", "application/json")
            .send(body)
            // Le message d'erreur d'`ureq` peut contenir l'adresse : on la remplace.
            .map_err(|e| WebhookError::Network(e.to_string().replace(url.expose(), "<webhook>")))?;
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok());
        Ok(Reply {
            status: response.status().as_u16(),
            retry_after,
        })
    }
}

/// Envoie un message avec les retries 429. Rend `Ok` quand Discord a accepté (2xx).
fn deliver(
    transport: &dyn Transport,
    url: &WebhookUrl,
    body: &str,
    sleep: &dyn Fn(Duration),
) -> Result<(), WebhookError> {
    for _ in 0..MAX_ATTEMPTS {
        let reply = transport.post_json(url, body)?;
        match reply.status {
            200..=299 => return Ok(()),
            429 => {
                // `max(0.0)` écarte un nombre négatif ; `min` borne l'attente.
                let secs = reply.retry_after.unwrap_or(1.0).max(0.0);
                let wait = if secs.is_finite() {
                    Duration::from_secs_f64(secs).min(MAX_RETRY_WAIT)
                } else {
                    MAX_RETRY_WAIT
                };
                sleep(wait);
            }
            other => return Err(WebhookError::Status(other)),
        }
    }
    Err(WebhookError::RateLimited)
}

/// File d'envoi vers un webhook : `send` prend `&self` et ne bloque jamais.
///
/// Le fil d'envoi s'arrête tout seul quand le `WebhookSender` est détruit (le canal se ferme).
pub struct WebhookSender {
    // `SyncSender` = émetteur d'un canal borné ; `try_send` ne bloque jamais l'appelant.
    tx: Option<SyncSender<Event>>,
    worker: Option<JoinHandle<()>>,
}

impl fmt::Debug for WebhookSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookSender(<secret>)")
    }
}

impl WebhookSender {
    /// Démarre l'envoi réel : 1 message par seconde au plus.
    pub fn start(url: WebhookUrl) -> WebhookSender {
        WebhookSender::with_transport(url, Arc::new(HttpTransport::new()), Duration::from_secs(1))
    }

    /// Comme [`WebhookSender::start`], avec un transport et un intervalle au choix (tests).
    pub fn with_transport(
        url: WebhookUrl,
        transport: Arc<dyn Transport>,
        min_interval: Duration,
    ) -> WebhookSender {
        let (tx, rx) = sync_channel::<Event>(QUEUE_CAPACITY);
        // `move` : le fil prend possession de `rx`, `url` et `transport`.
        let worker = std::thread::spawn(move || {
            let mut last_send: Option<Instant> = None;
            // `rx` rend `Err` quand tous les émetteurs sont détruits : la boucle se termine.
            while let Ok(event) = rx.recv() {
                if let Some(last) = last_send {
                    let elapsed = last.elapsed();
                    if elapsed < min_interval {
                        std::thread::sleep(min_interval - elapsed);
                    }
                }
                let body = format_payload(&event);
                let result = deliver(transport.as_ref(), &url, &body, &std::thread::sleep);
                last_send = Some(Instant::now());
                if let Err(error) = result {
                    // Le message d'erreur ne contient pas l'adresse (voir `WebhookError`).
                    tracing::warn!("webhook Discord : {error}");
                }
            }
        });
        WebhookSender {
            tx: Some(tx),
            worker: Some(worker),
        }
    }

    /// Met un événement en file. Ne bloque jamais ; rend `false` si la file est pleine
    /// (l'événement est alors abandonné).
    pub fn send(&self, event: Event) -> bool {
        let Some(tx) = &self.tx else { return false };
        match tx.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                tracing::warn!("webhook Discord : file pleine, événement abandonné");
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// Attend que la file soit vidée puis arrête le fil (utile aux tests et à la fermeture).
    pub fn finish(mut self) {
        self.tx.take(); // ferme le canal : le fil finit les messages restants puis sort
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const GOOD: &str = "https://discord.com/api/webhooks/123/abcSECRET";

    #[test]
    fn url_validation() {
        assert!(WebhookUrl::new(GOOD).is_ok());
        assert!(WebhookUrl::new("https://discordapp.com/api/webhooks/1/x").is_ok());
        for bad in [
            "http://discord.com/api/webhooks/1/x",
            "https://discord.com.evil.org/api/webhooks/1/x",
            "https://discord.com@evil.org/api/webhooks/1/x",
            "https://evil.org/https://discord.com/api/webhooks/1/x",
            "https://discord.com/api/webhooks/",
            "https://discord.com/api/webhooks/1 2",
            "",
        ] {
            assert!(WebhookUrl::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn debug_hides_the_secret() {
        let url = WebhookUrl::new(GOOD).unwrap();
        assert!(!format!("{url:?}").contains("SECRET"));
    }

    #[test]
    fn event_mapping() {
        let item = |flags| GameEvent::ItemSent {
            time: 0.0,
            finder: "A".into(),
            receiver: "B".into(),
            item: "Épée".into(),
            location: "Coffre".into(),
            flags,
        };
        assert!(event_from(&item(1)).is_some());
        assert!(event_from(&item(0)).is_none());
        let goal = GameEvent::Goal {
            time: 0.0,
            slot: 1,
            name: "A".into(),
        };
        assert_eq!(event_from(&goal), Some(Event::Goal { player: "A".into() }));
        let left = GameEvent::Left {
            time: 0.0,
            slot: 1,
            name: "A".into(),
        };
        assert!(event_from(&left).is_none());
    }

    #[test]
    fn message_escapes_and_blocks_mentions() {
        let event = Event::Goal {
            player: "@everyone **gras**\nligne".into(),
        };
        let payload: serde_json::Value = serde_json::from_str(&format_payload(&event)).unwrap();
        let content = payload["content"].as_str().unwrap();
        assert!(content.contains("\\@everyone"));
        assert!(content.contains("\\*\\*gras\\*\\*"));
        assert!(!content.contains('\n'));
        assert_eq!(payload["allowed_mentions"]["parse"], json!([]));
    }

    #[test]
    fn message_is_truncated_on_char_boundary() {
        let event = Event::Goal {
            player: "é".repeat(5000),
        };
        let text = format_message(&event);
        assert!(text.chars().count() <= MAX_CONTENT_CHARS);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn all_goals() {
        use server::SlotStatus;
        let slot = |status, is_group| SlotStatus {
            team: 0,
            slot: 1,
            name: "x".into(),
            game: "g".into(),
            is_group,
            connections: 0,
            tags: vec![],
            checked: 0,
            total: 0,
            status,
            last_activity: None,
        };
        let status = |slots| ServerStatus {
            slots,
            connections: 0,
        };
        assert!(all_goals_reached(&status(vec![
            slot(30, false),
            slot(0, true)
        ])));
        assert!(!all_goals_reached(&status(vec![
            slot(30, false),
            slot(20, false)
        ])));
        assert!(!all_goals_reached(&status(vec![])));
    }

    /// Faux transport : répond selon un script et note les envois.
    struct Fake {
        script: Mutex<Vec<Result<Reply, WebhookError>>>,
        calls: Mutex<Vec<(Instant, String)>>,
    }

    impl Fake {
        fn new(script: Vec<Result<Reply, WebhookError>>) -> Arc<Fake> {
            Arc::new(Fake {
                script: Mutex::new(script),
                calls: Mutex::new(vec![]),
            })
        }
    }

    impl Transport for Fake {
        fn post_json(&self, _url: &WebhookUrl, body: &str) -> Result<Reply, WebhookError> {
            self.calls
                .lock()
                .unwrap()
                .push((Instant::now(), body.to_string()));
            let mut script = self.script.lock().unwrap();
            if script.is_empty() {
                ok()
            } else {
                script.remove(0)
            }
        }
    }

    fn ok() -> Result<Reply, WebhookError> {
        Ok(Reply {
            status: 204,
            retry_after: None,
        })
    }

    #[test]
    fn retries_after_429_with_retry_after() {
        let fake = Fake::new(vec![
            Ok(Reply {
                status: 429,
                retry_after: Some(2.5),
            }),
            ok(),
        ]);
        let slept = Mutex::new(vec![]);
        let url = WebhookUrl::new(GOOD).unwrap();
        let result = deliver(fake.as_ref(), &url, "{}", &|d| {
            slept.lock().unwrap().push(d)
        });
        assert!(result.is_ok());
        assert_eq!(*slept.lock().unwrap(), vec![Duration::from_millis(2500)]);
        assert_eq!(fake.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn gives_up_after_too_many_429_and_on_errors() {
        let limited = || {
            Ok(Reply {
                status: 429,
                retry_after: Some(0.0),
            })
        };
        let fake = Fake::new(vec![limited(), limited(), limited(), ok()]);
        let url = WebhookUrl::new(GOOD).unwrap();
        let r = deliver(fake.as_ref(), &url, "{}", &|_| {});
        assert!(matches!(r, Err(WebhookError::RateLimited)));
        assert_eq!(fake.calls.lock().unwrap().len(), 3);

        let fake = Fake::new(vec![Ok(Reply {
            status: 404,
            retry_after: None,
        })]);
        let r = deliver(fake.as_ref(), &url, "{}", &|_| {});
        assert!(matches!(r, Err(WebhookError::Status(404))));
    }

    #[test]
    fn sender_spaces_messages_and_survives_failures() {
        let fake = Fake::new(vec![Err(WebhookError::Network("coupé".into())), ok(), ok()]);
        let url = WebhookUrl::new(GOOD).unwrap();
        let interval = Duration::from_millis(80);
        let sender = WebhookSender::with_transport(url, fake.clone(), interval);
        for i in 0..3 {
            assert!(sender.send(Event::Goal {
                player: format!("J{i}")
            }));
        }
        sender.finish();
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 3); // l'échec du premier n'empêche pas les suivants
        for pair in calls.windows(2) {
            assert!(pair[1].0.duration_since(pair[0].0) >= interval);
        }
    }

    #[test]
    fn full_queue_does_not_block() {
        struct Slow;
        impl Transport for Slow {
            fn post_json(&self, _: &WebhookUrl, _: &str) -> Result<Reply, WebhookError> {
                std::thread::sleep(Duration::from_millis(300));
                ok()
            }
        }
        let url = WebhookUrl::new(GOOD).unwrap();
        let sender = WebhookSender::with_transport(url, Arc::new(Slow), Duration::ZERO);
        let start = Instant::now();
        let accepted = (0..QUEUE_CAPACITY + 20)
            .filter(|_| sender.send(Event::PartyFinished))
            .count();
        assert!(start.elapsed() < Duration::from_millis(200));
        assert!(accepted < QUEUE_CAPACITY + 20);
        // Pas de `finish()` : on ne veut pas attendre 64 envois lents.
    }
}
