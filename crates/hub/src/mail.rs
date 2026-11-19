//! Envoi des e-mails du hub : vérification d'adresse et mot de passe oublié (ADR 0033).
//!
//! Le hub ne connaît que le trait [`Mailer`]. Trois implémentations :
//! - [`SmtpMailer`] : vrai envoi par SMTP (crate `lettre`), pour la production ;
//! - [`MemoryMailer`] : garde les messages en mémoire, pour les tests ;
//! - [`DisabledMailer`] : n'envoie rien (hub lancé sans SMTP sur le PC).
//!
//! Un message contient un jeton secret : ni son corps ni l'adresse du destinataire
//! ne vont dans le journal.

use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use lettre::message::{header::ContentType, Mailbox};
use lettre::{Message, SmtpTransport, Transport};

/// Un e-mail prêt à partir, en texte brut.
#[derive(Clone, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// `Debug` écrit à la main : seulement le sujet. L'adresse est une donnée
/// personnelle et le corps contient un jeton secret.
impl fmt::Debug for Email {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Email")
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("adresse d'e-mail invalide")]
    Address,
    #[error("message impossible à construire")]
    Build,
    #[error("réglage SMTP invalide")]
    Config,
    #[error("envoi SMTP raté")]
    Send,
}

/// Ce que le hub demande à un service d'envoi.
///
/// L'envoi est **synchrone** (il bloque le thread) : le hub l'appelle toujours sur
/// un thread à part (`spawn_blocking`), jamais pendant le traitement d'une requête.
/// `Send + Sync` : l'objet est partagé entre les threads du serveur.
pub trait Mailer: Send + Sync {
    fn send(&self, email: &Email) -> Result<(), MailError>;

    /// Vrai si ce service envoie vraiment les messages (tableau de bord de la
    /// console d'admin, ADR 0036). Méthode par défaut : les services la gardent,
    /// sauf [`DisabledMailer`].
    fn is_configured(&self) -> bool {
        true
    }
}

/// Envoi réel par SMTP.
pub struct SmtpMailer {
    transport: SmtpTransport,
    from: Mailbox,
}

impl SmtpMailer {
    /// `url` : `smtps://utilisateur:motdepasse@hote:465` (TLS direct) ou
    /// `smtp://utilisateur:motdepasse@hote:587?tls=required` (STARTTLS).
    /// `from` : expéditeur, par exemple `Raftup <no-reply@exemple.org>`.
    pub fn from_url(url: &str, from: &str) -> Result<SmtpMailer, MailError> {
        // Les erreurs de `lettre` peuvent recopier l'URL, donc le mot de passe :
        // on les remplace par une erreur sans détail.
        let transport = SmtpTransport::from_url(url)
            .map_err(|_| MailError::Config)?
            .timeout(Some(Duration::from_secs(20)))
            .build();
        let from = from.parse().map_err(|_| MailError::Address)?;
        Ok(SmtpMailer { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send(&self, email: &Email) -> Result<(), MailError> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(email.to.parse().map_err(|_| MailError::Address)?)
            .subject(email.subject.clone())
            .header(ContentType::TEXT_PLAIN)
            .body(email.body.clone())
            .map_err(|_| MailError::Build)?;
        self.transport
            .send(&message)
            .map(|_| ())
            .map_err(|_| MailError::Send)
    }
}

/// L'adresse SMTP est-elle lisible par `lettre` ? (réglages de la console)
pub fn valid_smtp_url(url: &str) -> bool {
    SmtpTransport::from_url(url).is_ok()
}

/// L'expéditeur est-il une adresse valable (`Nom <adresse@exemple.org>`) ?
pub fn valid_mailbox(from: &str) -> bool {
    from.parse::<Mailbox>().is_ok()
}

/// N'envoie rien : le hub tourne sans SMTP (développement sur le PC).
pub struct DisabledMailer;

impl Mailer for DisabledMailer {
    fn send(&self, email: &Email) -> Result<(), MailError> {
        tracing::warn!(subject = %email.subject, "envoi d'e-mail désactivé : message abandonné");
        Ok(())
    }

    fn is_configured(&self) -> bool {
        false
    }
}

/// Garde les messages en mémoire, pour que les tests lisent les jetons envoyés.
#[derive(Default)]
pub struct MemoryMailer {
    sent: Mutex<Vec<Email>>,
}

impl MemoryMailer {
    pub fn new() -> MemoryMailer {
        MemoryMailer::default()
    }

    /// Copie des messages envoyés, du plus ancien au plus récent.
    pub fn sent(&self) -> Vec<Email> {
        self.sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Attend (2 secondes au plus) qu'au moins `count` messages soient partis :
    /// l'envoi se fait sur un autre thread, après la réponse HTTP.
    pub async fn wait_for(&self, count: usize) -> Vec<Email> {
        for _ in 0..200 {
            let sent = self.sent();
            if sent.len() >= count {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.sent()
    }
}

impl Mailer for MemoryMailer {
    fn send(&self, email: &Email) -> Result<(), MailError> {
        self.sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(email.clone());
        Ok(())
    }
}

/// Texte du message de vérification d'adresse.
pub fn verification_email(to: &str, username: &str, link: &str, hours: u64) -> Email {
    Email {
        to: to.to_string(),
        subject: "Raftup : confirmez votre adresse e-mail".to_string(),
        body: format!(
            "Bonjour {username},\n\n\
             Pour confirmer l'adresse de votre compte Raftup, ouvrez ce lien \
             dans les {hours} heures :\n\n{link}\n\n\
             Si vous n'avez pas créé de compte Raftup, ignorez ce message.\n"
        ),
    }
}

/// Texte du message « mot de passe oublié ».
pub fn password_reset_email(to: &str, username: &str, token: &str, minutes: u64) -> Email {
    Email {
        to: to.to_string(),
        subject: "Raftup : nouveau mot de passe".to_string(),
        body: format!(
            "Bonjour {username},\n\n\
             Quelqu'un (vous, sans doute) a demandé à changer le mot de passe de votre \
             compte Raftup. Ouvrez ce lien depuis l'ordinateur où le launcher est installé, \
             dans les {minutes} minutes :\n\n\
             raftup://password-reset?token={token}\n\n\
             ou collez ce code dans le launcher :\n\n{token}\n\n\
             Si vous n'avez rien demandé, ignorez ce message : votre mot de passe ne change pas.\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_ne_montre_ni_adresse_ni_corps() {
        let email = password_reset_email("a@example.org", "Alice", "jeton-secret", 60);
        let shown = format!("{email:?}");
        assert!(!shown.contains("a@example.org"));
        assert!(!shown.contains("jeton-secret"));
        assert!(email.body.contains("jeton-secret"));
    }

    #[test]
    fn smtp_mal_regle_sans_mot_de_passe_dans_l_erreur() {
        let error = SmtpMailer::from_url("pas-une-url:motdepasse", "Raftup <a@b.c>")
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(!error.contains("motdepasse"));
        assert!(SmtpMailer::from_url("smtps://u:p@localhost:465", "pas une adresse").is_err());
        assert!(SmtpMailer::from_url("smtps://u:p@localhost:465", "Raftup <a@b.c>").is_ok());
    }

    /// Faux serveur SMTP minimal (sans TLS) sur un port local : il répond « OK » à
    /// tout et renvoie ce que le client lui a écrit.
    fn fake_smtp_server() -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let mut seen = String::new();
            writer.write_all(b"220 faux serveur\r\n").unwrap();
            let mut in_data = false;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                seen.push_str(&line);
                let reply: &[u8] = if in_data {
                    if line == ".\r\n" {
                        in_data = false;
                        b"250 recu\r\n"
                    } else {
                        continue;
                    }
                } else if line.starts_with("DATA") {
                    in_data = true;
                    b"354 suite\r\n"
                } else if line.starts_with("QUIT") {
                    writer.write_all(b"221 au revoir\r\n").unwrap();
                    break;
                } else {
                    b"250 ok\r\n"
                };
                writer.write_all(reply).unwrap();
            }
            seen
        });
        (port, handle)
    }

    #[test]
    fn envoi_smtp_reel_vers_un_faux_serveur() {
        let (port, server) = fake_smtp_server();
        let mailer = SmtpMailer::from_url(
            &format!("smtp://127.0.0.1:{port}"),
            "Raftup <no-reply@example.org>",
        )
        .unwrap();
        let email = verification_email("alice@example.org", "Alice", "http://lien", 24);
        mailer.send(&email).unwrap();
        drop(mailer);
        let seen = server.join().unwrap();
        assert!(seen.contains("MAIL FROM:<no-reply@example.org>"), "{seen}");
        assert!(seen.contains("RCPT TO:<alice@example.org>"), "{seen}");
        assert!(seen.contains("http://lien"), "{seen}");
    }
}
