//! Héberger une partie à partir de fichiers YAML : générer, puis démarrer l'hôte (H1).
//!
//! Ce module est à côté de `generation` et `host` plutôt que dans l'un des deux : il
//! dépend des deux, et aucun ne doit dépendre de l'autre.
//!
//! Deux étapes, exposées séparément pour pouvoir les tester (et les appeler) chacune :
//! - [`generate_blocking`] : la génération, synchrone et bloquante ;
//! - [`host_generated`] : le démarrage de l'hôte sur la seed produite, `async`.
//!
//! [`generate_and_host`] enchaîne les deux.
//!
//! Si le démarrage de l'hôte échoue (port pris, certificat refusé…), le dossier
//! `generations/<horodatage>/` est CONSERVÉ : la seed y est, le joueur peut corriger
//! le réglage et réessayer avec [`host_generated`] sans régénérer.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use py_bridge::CancelToken;
use tokio::runtime::{Handle, RuntimeFlavor};
use tracing::warn;

use crate::error::LauncherError;
use crate::generation::{
    generate, GenerateOutput, GenerateRequest, GenerationError, GeneratorBackend,
};
use crate::host::{Encryption, Host, HostOptions, HostStatus};
use crate::paths::Paths;

/// Réglages de l'hébergement : ceux de [`HostOptions`], sans la seed (elle vient de la génération).
#[derive(Clone, PartialEq, Eq)]
pub struct PartyHostOptions {
    pub port: u16,
    pub password: Option<String>,
    pub encryption: Encryption,
    pub public_host: String,
    pub disable_save: bool,
}

impl PartyHostOptions {
    /// Vérifie les réglages sans rien démarrer : appelé AVANT de générer, pour ne pas
    /// perdre des minutes de génération sur un réglage invalide.
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.public_host.trim().is_empty() {
            return Err(PartyError::EmptyPublicHost);
        }
        Ok(())
    }

    /// Complète les réglages avec la seed pour obtenir les [`HostOptions`] de l'hôte.
    /// `self` est consommé (déplacé) : le mot de passe n'est pas copié.
    pub fn into_host_options(self, seed: PathBuf) -> Result<HostOptions, PartyError> {
        self.validate()?;
        Ok(HostOptions {
            seed,
            port: self.port,
            password: self.password,
            encryption: self.encryption,
            public_host: self.public_host,
            disable_save: self.disable_save,
            webhook: None,
        })
    }
}

impl std::fmt::Debug for PartyHostOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Comme `HostOptions` : le mot de passe n'apparaît jamais dans un journal.
        let password = self.password.as_ref().map(|_| "***");
        f.debug_struct("PartyHostOptions")
            .field("port", &self.port)
            .field("password", &password)
            .field("encryption", &self.encryption)
            .field("public_host", &self.public_host)
            .field("disable_save", &self.disable_save)
            .finish()
    }
}

/// Une partie générée et hébergée.
#[derive(Debug, Clone)]
pub struct PartyStarted {
    pub generated: GenerateOutput,
    pub status: HostStatus,
}

/// Ce qui peut échouer entre le YAML et la partie en ligne.
#[derive(Debug, thiserror::Error)]
pub enum PartyError {
    /// Une partie est déjà hébergée : refusé AVANT de générer (une génération dure des minutes).
    #[error("une partie est déjà hébergée")]
    AlreadyRunning,

    #[error(transparent)]
    Generation(#[from] GenerationError),

    /// Le nom public de l'hôte est vide : refusé avant toute génération.
    #[error("le nom public de l'hôte est vide")]
    EmptyPublicHost,

    /// Le démarrage de l'hôte a échoué ; la seed générée reste sur le disque.
    ///
    /// L'erreur rend aussi la sortie de la génération : on peut corriger le réglage
    /// (autre port…) et rappeler [`host_generated`] sans régénérer. `Box` : la sortie
    /// est grosse, la mettre sur le tas garde l'enum `PartyError` petite.
    #[error("démarrage de l'hôte : {source}")]
    Host {
        #[source]
        source: LauncherError,
        generated: Box<GenerateOutput>,
    },
}

/// Lance la génération en bloquant le thread courant. C'est `generation::generate`, sans plus.
///
/// À appeler depuis un thread de fond, pas depuis le thread d'une interface ni
/// directement depuis une tâche async (voir [`generate_and_host`]).
///
/// `+ Send` : le générateur peut être déplacé vers un autre thread (indispensable
/// pour que le `Future` de [`generate_and_host`] puisse passer dans `tokio::spawn`).
pub fn generate_blocking(
    paths: &Paths,
    backend: &mut (dyn GeneratorBackend + Send),
    req: &GenerateRequest,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<GenerateOutput, GenerationError> {
    generate(paths, backend, req, cancel, log)
}

/// Démarre l'hôte sur une seed déjà générée.
pub async fn host_generated(
    host: &mut Host,
    output: &GenerateOutput,
    options: PartyHostOptions,
) -> Result<HostStatus, PartyError> {
    if host.is_running() {
        return Err(PartyError::AlreadyRunning);
    }
    let host_options = options.into_host_options(output.archipelago_file.clone())?;
    match host.start(host_options).await {
        Ok(status) => Ok(status),
        // On rend la sortie dans l'erreur pour permettre de réessayer sans régénérer.
        Err(source) => Err(PartyError::Host {
            source,
            generated: Box::new(output.clone()),
        }),
    }
}

/// Génère une seed à partir des YAML puis l'héberge.
///
/// Refuse tout de suite (`AlreadyRunning`) si une partie est déjà hébergée.
///
/// Pourquoi ni `spawn_blocking` ni un simple appel : `generate` est synchrone et dure
/// des minutes. Dans une fonction `async`, un appel direct gèlerait le thread du
/// runtime tokio (le serveur de la partie en cours, les événements…). La solution
/// habituelle, `tokio::task::spawn_blocking`, déplace le travail sur un autre thread,
/// mais exige que la closure soit `'static` (n'emprunte rien) : or `backend` est un
/// `&mut dyn GeneratorBackend` emprunté à l'appelant. On utilise donc
/// `tokio::task::block_in_place`, qui accepte les emprunts : il prévient tokio que ce
/// thread va bloquer et lui fait confier ses autres tâches à un autre thread.
/// Limite : cela ne marche que sur un runtime multi-thread. Sur un runtime
/// `current_thread` (cas de `#[tokio::test]` par défaut) `block_in_place` paniquerait ;
/// hors de tout runtime tokio, il n'y a rien à protéger. Dans ces deux cas on appelle
/// `generate` directement (un avertissement est journalisé pour `current_thread`) : le
/// runtime est bloqué le temps de la génération.
///
/// À retenir : `block_in_place` ne libère que les AUTRES tâches. Un `join!` ou un
/// `select!` écrit dans la même tâche que cet appel est suspendu lui aussi (ses autres
/// branches n'avancent pas). Le plus sûr pour une interface : lancer
/// [`generate_blocking`] dans un thread `std::thread`, puis appeler [`host_generated`]
/// avec le résultat.
///
/// `Send` : le `Future` retourné peut être envoyé à `tokio::spawn`, parce que `backend`
/// est un `dyn GeneratorBackend + Send` (un objet qui promet d'être déplaçable entre threads).
pub async fn generate_and_host(
    host: &mut Host,
    paths: &Paths,
    backend: &mut (dyn GeneratorBackend + Send),
    req: &GenerateRequest,
    host_options: PartyHostOptions,
    cancel: &CancelToken,
    log: Sender<String>,
) -> Result<PartyStarted, PartyError> {
    if host.is_running() {
        return Err(PartyError::AlreadyRunning);
    }
    // Avant de générer : un réglage invalide ne doit pas coûter une génération.
    host_options.validate()?;
    // `Handle` = la poignée du runtime tokio courant ; `try_current` rend une erreur
    // (au lieu de paniquer) quand on n'est dans aucun runtime.
    let generated = match Handle::try_current().map(|handle| handle.runtime_flavor()) {
        Ok(RuntimeFlavor::CurrentThread) => {
            warn!(
                "génération bloquante sur un runtime current_thread : le runtime est gelé \
                 pendant la génération (préférer generate_blocking dans un thread std)"
            );
            generate_blocking(paths, backend, req, cancel, log)?
        }
        Ok(_) => {
            tokio::task::block_in_place(|| generate_blocking(paths, backend, req, cancel, log))?
        }
        // Hors runtime : appel direct, il n'y a aucune tâche à protéger.
        Err(_) => generate_blocking(paths, backend, req, cancel, log)?,
    };
    // Si ceci échoue, `generated.archipelago_file` reste sur le disque (voir l'en-tête),
    // et l'erreur `PartyError::Host` rend `generated` pour réessayer.
    let status = host_generated(host, &generated, host_options).await?;
    Ok(PartyStarted { generated, status })
}
