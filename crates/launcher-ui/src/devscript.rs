//! Macro de développement : joue l'écran « Créer une partie » tout seul et en prend
//! des captures, pour le regarder sans être devant le PC.
//!
//! Seulement en debug (`main.rs` ne compile ce module qu'avec `debug_assertions`).
//! Elle s'active avec `RAFTUP_DEV_CAPTURE=<dossier>` :
//! - `RAFTUP_DEV_YAML=<dossier>` : les fichiers `.yaml` à ajouter comme joueurs ;
//! - `RAFTUP_DEV_RUST=1` : coche « Moteur Rust (expérimental) » avant de générer ;
//! - `RAFTUP_DEV_TEMPS=1` : coche aussi « Équilibrer par temps de jeu » (ADR 0030) ;
//! - `RAFTUP_DEV_PORT=<port>` (38296 par défaut) : premier port essayé ; s'il est
//!   occupé, « Réessayer » passe au port suivant ;
//! - `RAFTUP_DEV_SCENARIO` : `creer` (défaut, le scénario complet), puis `bibliotheque`,
//!   `heberger`, `en-cours`, `salon`, `rejoindre`, `amis` (`RAFTUP_DEV_HUB_INSCRIPTION=1` : onglet « Créer un compte »), `parametres`, `changer-de-hub`, `generateur-absent` : une seule capture de l'écran,
//!   `1-<scénario>.png`. L'écran affiché au départ se choisit à part, avec
//!   `RAFTUP_DEV_PAGE` (`bibliotheque`, `partie`, `creer`…), et `RAFTUP_DEV_SEED`,
//!   `RAFTUP_DEV_RACINE`, `RAFTUP_DEV_PROFILS` comme d'habitude ; `generateur-absent`
//!   a besoin de `RAFTUP_DEV_YAML` (sinon le formulaire est refusé avant la vérification
//!   du générateur) et d'une `RAFTUP_DEV_RACINE` vide ;
//! - `RAFTUP_DEV_AVANCE=1` : ouvre au départ les blocs repliés (« Options avancées »,
//!   « Se connecter à la main »…) ;
//! - `RAFTUP_DEV_SEEDFILE=<fichier>` : la seed du scénario `heberger`
//!   (`crates/multidata/tests/data/seed-mit.archipelago` par défaut), port 38295.
//!
//! Pas de faux clics envoyés à Windows : la macro appelle directement les fonctions
//! de l'écran, et la capture vient du rendu de la fenêtre elle-même (egui), donc
//! aucune autre fenêtre du bureau n'y apparaît.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::app::Page;
use crate::screens::create::{self, CreateForm};
use crate::screens::running::{self, RunningState};
use crate::screens::setup::{self, SetupForm};
use crate::worker::Worker;

/// Les formulaires que la macro remplit.
pub struct Forms<'a> {
    pub create: &'a mut CreateForm,
    pub setup: &'a mut SetupForm,
    pub running: &'a mut RunningState,
}

/// Le scénario joué.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    /// « Créer une partie », de bout en bout (plusieurs captures).
    Create,
    /// « Héberger une partie » avec une seed choisie.
    Host,
    /// « Créer une partie » avec un générateur absent : le formulaire est rempli, on
    /// clique « Générer la partie » et on capture le panneau de consentement. On ne
    /// clique JAMAIS « Installer » : rien n'est téléchargé.
    NoGenerator,
    /// Un écran, une capture (Bibliothèque, En cours).
    Screen(&'static str),
}

impl Scenario {
    fn from_env() -> Scenario {
        match std::env::var("RAFTUP_DEV_SCENARIO").ok().as_deref() {
            Some("heberger") => Scenario::Host,
            Some("bibliotheque") => Scenario::Screen("1-bibliotheque"),
            Some("en-cours") => Scenario::Screen("1-en-cours"),
            // « Partie en cours », onglet « Salon » (avec `RAFTUP_DEV_SEED`).
            Some("salon") => Scenario::Screen("1-salon"),
            // Une seule capture, quand un lien reçu (argument ou 2e instance) est affiché.
            Some("rejoindre") => Scenario::Screen("1-rejoindre"),
            // Écran « Amis » (avec `RAFTUP_DEV_PAGE=amis`, `RAFTUP_DEV_HUB` et, pour la
            // vue connectée, `RAFTUP_DEV_HUB_LOGIN`).
            Some("amis") => Scenario::Screen("1-amis"),
            // Écran « Paramètres » (avec `RAFTUP_DEV_PAGE=parametres`).
            Some("parametres") => Scenario::Screen("1-parametres"),
            // Boîte « Passer sur le hub … ? » : lancer avec un lien `raftup://hub/<hôte>`
            // en argument (ou transmis par une 2e instance avec `RAFTUP_DEV_DELAY_MS`).
            Some("changer-de-hub") => Scenario::Screen("1-changer-de-hub"),
            Some("generateur-absent") => Scenario::NoGenerator,
            _ => Scenario::Create,
        }
    }

    /// Vrai si le scénario part de l'écran « Créer une partie ».
    fn on_create_screen(self) -> bool {
        matches!(self, Scenario::Create | Scenario::NoGenerator)
    }

    /// Nom de la capture d'un scénario à une seule capture.
    fn file_name(self) -> &'static str {
        match self {
            Scenario::Create => "1-preparation",
            Scenario::Host => "1-heberger",
            Scenario::NoGenerator => "1-generateur-absent",
            Scenario::Screen(name) => name,
        }
    }
}

/// L'étape de la macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// On attend que l'écran soit affiché.
    Wait,
    /// Formulaire rempli ; capture.
    Prepared,
    /// Clic sur « Générer la partie ».
    Clicking,
    /// Génération lancée ; capture pendant qu'elle tourne.
    Generating,
    /// On attend la fin (échec, port occupé ou partie en ligne).
    Result,
    /// Clic sur « Réessayer », sur le port suivant.
    Retrying,
    /// « Réessayer » cliqué sur un autre port ; capture de « Partie en cours ».
    Retried,
    Finished,
}

pub struct DevScript {
    dir: PathBuf,
    scenario: Scenario,
    yamls: Vec<PathBuf>,
    port: u16,
    step: Step,
    /// Début de l'étape en cours (ou du moment où la condition attendue est vue).
    since: Instant,
    /// Vrai quand la condition de l'étape `Result` ou `Retried` est remplie.
    seen: bool,
    frames: u32,
}

impl DevScript {
    /// La macro, si `RAFTUP_DEV_CAPTURE` est posée.
    pub fn from_env() -> Option<DevScript> {
        let dir = PathBuf::from(std::env::var_os("RAFTUP_DEV_CAPTURE")?);
        let yamls = std::env::var_os("RAFTUP_DEV_YAML")
            .map(|d| yaml_files(Path::new(&d)))
            .unwrap_or_default();
        let port = std::env::var("RAFTUP_DEV_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(38296);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!("macro : dossier de captures {} : {e}", dir.display());
        }
        tracing::info!(
            "macro : {} YAML, captures dans {}",
            yamls.len(),
            dir.display()
        );
        Some(DevScript {
            dir,
            scenario: Scenario::from_env(),
            yamls,
            port,
            step: Step::Wait,
            since: Instant::now(),
            seen: false,
            frames: 0,
        })
    }

    /// Avance d'un pas ; appelée à chaque image, après le dessin.
    pub fn step(
        &mut self,
        ctx: &egui::Context,
        page: &Page,
        forms: &mut Forms,
        error: &mut Option<String>,
        worker: &Worker,
    ) {
        let form = &mut *forms.create;
        self.frames += 1;
        // La macro attend des délais : on redessine régulièrement pour la faire avancer.
        ctx.request_repaint_after(Duration::from_millis(100));
        let elapsed = self.since.elapsed();
        match self.step {
            Step::Wait if self.scenario.on_create_screen() && self.frames > 5 => {
                if *page == Page::Create {
                    create::dev_prefill(form, self.yamls.clone(), self.port);
                    self.next(Step::Prepared);
                }
            }
            Step::Wait if self.frames > 5 => {
                if self.scenario == Scenario::Host {
                    let seed = std::env::var_os("RAFTUP_DEV_SEEDFILE")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| {
                            PathBuf::from("crates/multidata/tests/data/seed-mit.archipelago")
                        });
                    setup::dev_prefill(forms.setup, worker, seed, 38295);
                }
                self.next(Step::Prepared);
            }
            Step::Prepared
                if self.scenario == Scenario::NoGenerator
                    && elapsed > Duration::from_millis(500) =>
            {
                create::dev_generate(ctx, form, error);
                self.next(Step::Result);
            }
            Step::Result
                if self.scenario == Scenario::NoGenerator
                    && elapsed > Duration::from_millis(1500) =>
            {
                self.capture(ctx, self.scenario.file_name());
                self.next(Step::Finished);
            }
            // Scénarios à une capture : 2,5 s pour le scan, la lecture de la seed ou le
            // démarrage de la partie (ou `RAFTUP_DEV_DELAY_MS`, par exemple pour laisser
            // une 2e instance transmettre un lien), puis on capture et on s'arrête.
            Step::Prepared
                if !self.scenario.on_create_screen() && elapsed > single_capture_delay() =>
            {
                // Ouvert ici, pas au départ : tant que la partie n'a pas démarré, l'état
                // de « Partie en cours » est remis à zéro à chaque image.
                if self.scenario == Scenario::Screen("1-salon") {
                    running::dev_open_salon(forms.running);
                }
                self.capture(ctx, self.scenario.file_name());
                self.next(Step::Finished);
            }
            // La capture montre l'image SUIVANTE : on capture, puis on agit à l'étape
            // d'après, sinon l'image montrerait déjà le résultat de l'action.
            Step::Prepared
                if self.scenario == Scenario::Create && elapsed > Duration::from_secs(1) =>
            {
                self.capture(ctx, "1-preparation");
                self.next(Step::Clicking);
            }
            Step::Clicking if elapsed > Duration::from_millis(500) => {
                create::dev_generate(ctx, form, error);
                self.next(Step::Generating);
            }
            // Une partie APQuest se génère en une seconde environ : capture rapide.
            Step::Generating if elapsed > Duration::from_millis(300) => {
                self.capture(ctx, "2-generation");
                self.next(Step::Result);
            }
            Step::Result => {
                let state = create::dev_state(form);
                let done = state.generation_failed || state.hosting_failed || *page != Page::Create;
                if done && !self.seen {
                    self.seen = true;
                    self.since = Instant::now();
                } else if self.seen && elapsed > Duration::from_millis(800) {
                    self.capture(ctx, "3-resultat");
                    let next = if state.hosting_failed {
                        Step::Retrying
                    } else {
                        Step::Finished
                    };
                    self.next(next);
                }
            }
            Step::Retrying if elapsed > Duration::from_millis(500) => {
                create::dev_retry(form, error, worker, self.port.saturating_add(1));
                self.next(Step::Retried);
            }
            Step::Retried => {
                if *page == Page::Hosted && !self.seen {
                    self.seen = true;
                    self.since = Instant::now();
                } else if self.seen && elapsed > Duration::from_millis(1500) {
                    self.capture(ctx, "4-partie-en-cours");
                    self.next(Step::Finished);
                }
            }
            _ => {}
        }
    }

    fn next(&mut self, step: Step) {
        tracing::info!("macro : étape {step:?}");
        self.step = step;
        self.since = Instant::now();
        self.seen = false;
    }

    /// Demande une capture de la prochaine image, enregistrée en `<nom>.png`.
    fn capture(&self, ctx: &egui::Context, name: &str) {
        let path = self.dir.join(format!("{name}.png"));
        // `move` : la fonction est appelée plus tard, une fois l'image lue ; elle
        // emporte sa propre copie du chemin.
        ctx.request_screenshot(move |image| {
            let [width, height] = image.size;
            let saved = image::save_buffer(
                &path,
                image.as_raw(),
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            );
            match saved {
                Ok(()) => tracing::info!("macro : capture {}", path.display()),
                Err(e) => tracing::warn!("macro : capture {} : {e}", path.display()),
            }
        });
    }
}

/// Délai avant la capture des scénarios à une seule image (2,5 s par défaut).
fn single_capture_delay() -> Duration {
    let ms = std::env::var("RAFTUP_DEV_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2500);
    Duration::from_millis(ms)
}

/// Les `.yaml` du dossier, triés par nom (ordre stable).
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "yaml" || x == "yml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_tries_par_nom() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["bob.yaml", "alice.yml", "notes.txt"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let names: Vec<String> = yaml_files(dir.path())
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["alice.yml", "bob.yaml"]);
        // Dossier absent : aucun fichier, pas d'erreur.
        assert!(yaml_files(&dir.path().join("absent")).is_empty());
    }
}
