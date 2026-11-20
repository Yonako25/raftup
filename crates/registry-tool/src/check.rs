//! Calcul du niveau de confiance d'un paquet (ADR 0042).
//!
//! Les vérifications montent dans l'ordre des niveaux ; la première qui échoue arrête la
//! montée. Le générateur est caché derrière le trait [`Generator`] : le vrai (générateur
//! officiel épinglé, `official.rs`) ou un faux dans les tests.

use std::path::Path;
use std::time::Duration;

use registry::{Package, PackageKind, TrustLevel};

use crate::fuzz::{make_case, CaseKind, DEFAULT_FUZZ_SEED};
use crate::options::{default_yaml, OptionSpec};
use crate::report::{PackageReport, ReportSettings, Step, StepStatus};
use crate::structure::check_structure;

/// Issue d'une génération.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// La partie a été générée.
    Generated,
    /// Le générateur a refusé proprement (exception d'Archipelago, YAML refusé).
    Refused(String),
    /// Plantage : process Python mort, réponse illisible, erreur interne du worker.
    Crashed(String),
    /// Boucle : délai du cas dépassé (le worker est tué).
    TimedOut(String),
    /// Problème de l'environnement (disque, runtime, bac à sable) : pas la faute du paquet.
    Environment(String),
}

/// Ce que la vérification demande au générateur. Un trait = un contrat, comme une
/// interface en Java : le vrai générateur et le faux des tests le remplissent tous deux.
pub trait Generator {
    /// Nom et version du générateur, écrits dans le rapport (`Archipelago 0.6.8`).
    fn name(&self) -> String;

    /// Génère une partie solo avec ce YAML, l'apworld posée dans les mondes installés.
    fn generate(&mut self, apworld: &Path, yaml: &str, seed: u64, timeout: Duration) -> RunOutcome;

    /// Liste les options du jeu (partie solo par défaut construite jusqu'au placement).
    /// L'erreur est une issue autre que `Generated`.
    fn describe_options(
        &mut self,
        apworld: &Path,
        game: &str,
        timeout: Duration,
    ) -> Result<Vec<OptionSpec>, RunOutcome>;
}

/// Réglages de la vérification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckSettings {
    /// Graines de la génération par défaut (niveau « générable »).
    pub seeds: u32,
    /// Cas de fuzzing (niveau « testé en profondeur ») ; 0 = pas de fuzzing.
    pub fuzz_cases: u32,
    pub fuzz_seed: u64,
    /// Délai d'une génération ; au-delà, le cas est une boucle.
    pub case_timeout: Duration,
    /// Temps total du fuzzing d'un paquet ; au-delà, on s'arrête au niveau « générable ».
    pub time_limit: Duration,
}

impl Default for CheckSettings {
    fn default() -> Self {
        CheckSettings {
            seeds: 5,
            fuzz_cases: 40,
            fuzz_seed: DEFAULT_FUZZ_SEED,
            case_timeout: Duration::from_secs(300),
            time_limit: Duration::from_secs(30 * 60),
        }
    }
}

impl CheckSettings {
    /// Copie des réglages pour le rapport.
    pub fn for_report(&self) -> ReportSettings {
        ReportSettings {
            seeds: self.seeds,
            fuzz_cases: self.fuzz_cases,
            fuzz_seed: self.fuzz_seed,
            case_timeout_secs: self.case_timeout.as_secs(),
            time_limit_secs: self.time_limit.as_secs(),
        }
    }
}

/// Le paquet à vérifier : son entrée (sha256 et taille déjà calculés), ses octets, et le
/// chemin d'une copie `<name>.apworld` pour le générateur.
pub struct Candidate<'a> {
    pub package: &'a Package,
    pub data: &'a [u8],
    pub apworld_path: &'a Path,
}

/// Noms des étapes, dans l'ordre.
pub const STEP_STRUCTURE: &str = "Structure";
pub const STEP_GENERATE: &str = "Génération par défaut";
pub const STEP_FUZZ: &str = "Fuzzing";

/// Vérifie un paquet et rend son rapport.
///
/// `generator` : `Err(raison)` si aucun générateur n'est disponible (le niveau s'arrête
/// alors à « structure valide »). `elapsed` donne le temps écoulé depuis le début du
/// fuzzing (une horloge, remplacée par un faux dans les tests).
///
/// Erreur : un problème de l'environnement, pas du paquet ; aucun niveau n'est attribué.
pub fn check_package(
    candidate: &Candidate<'_>,
    generator: Result<&mut dyn Generator, String>,
    settings: &CheckSettings,
    elapsed: &mut dyn FnMut() -> Duration,
) -> Result<PackageReport, String> {
    let package = candidate.package;
    let mut report = PackageReport {
        kind: package.kind,
        name: package.name.clone(),
        version: package.version.clone(),
        sha256: package.sha256.clone(),
        size: package.size,
        level: TrustLevel::Unverified,
        detail: String::new(),
        steps: Vec::new(),
    };

    // Niveau 1 : structure.
    match check_structure(package, candidate.data) {
        Ok(summary) => {
            report
                .steps
                .push(step(STEP_STRUCTURE, StepStatus::Passed, summary));
            report.level = TrustLevel::Valid;
        }
        Err(reason) => {
            report
                .steps
                .push(step(STEP_STRUCTURE, StepStatus::Failed, reason));
            skip_rest(&mut report, STEP_GENERATE, "structure invalide");
            return Ok(finish(report, None, settings, 0));
        }
    }
    if package.kind == PackageKind::Profile {
        skip_rest(
            &mut report,
            STEP_GENERATE,
            "un profil s'arrête à « structure valide » (son apworld a son propre niveau)",
        );
        return Ok(finish(report, None, settings, 0));
    }
    let generator = match generator {
        Ok(generator) => generator,
        Err(reason) => {
            skip_rest(&mut report, STEP_GENERATE, &reason);
            return Ok(finish(report, None, settings, 0));
        }
    };
    let generator_name = generator.name();
    let Some(game) = package.game.as_deref() else {
        report.steps.push(step(
            STEP_GENERATE,
            StepStatus::Failed,
            "champ « game » absent du registre : impossible d'écrire le YAML".to_string(),
        ));
        skip_rest(&mut report, STEP_FUZZ, "génération non essayée");
        return Ok(finish(report, Some(&generator_name), settings, 0));
    };

    // Niveau 2 : génération par défaut sur N graines.
    let yaml = default_yaml(game);
    for seed in 1..=u64::from(settings.seeds) {
        let outcome =
            generator.generate(candidate.apworld_path, &yaml, seed, settings.case_timeout);
        match outcome {
            RunOutcome::Generated => {}
            RunOutcome::Environment(reason) => return Err(reason),
            other => {
                report.steps.push(step(
                    STEP_GENERATE,
                    StepStatus::Failed,
                    format!("graine {seed} : {}", describe(&other)),
                ));
                skip_rest(&mut report, STEP_FUZZ, "génération par défaut en échec");
                return Ok(finish(report, Some(&generator_name), settings, 0));
            }
        }
    }
    if settings.seeds == 0 {
        report.steps.push(step(
            STEP_GENERATE,
            StepStatus::Skipped,
            "aucune graine demandée".to_string(),
        ));
        skip_rest(&mut report, STEP_FUZZ, "génération non essayée");
        return Ok(finish(report, Some(&generator_name), settings, 0));
    }
    report.steps.push(step(
        STEP_GENERATE,
        StepStatus::Passed,
        format!(
            "{} graine(s) générée(s) avec les options par défaut",
            settings.seeds
        ),
    ));
    report.level = TrustLevel::Generates;

    // Niveau 3 : fuzzing.
    if settings.fuzz_cases == 0 {
        report.steps.push(step(
            STEP_FUZZ,
            StepStatus::Skipped,
            "aucun cas demandé".to_string(),
        ));
        return Ok(finish(report, Some(&generator_name), settings, 0));
    }
    let options =
        match generator.describe_options(candidate.apworld_path, game, settings.case_timeout) {
            Ok(options) => options,
            Err(RunOutcome::Environment(reason)) => return Err(reason),
            Err(other) => {
                report.steps.push(step(
                    STEP_FUZZ,
                    StepStatus::Failed,
                    format!("options du monde illisibles : {}", describe(&other)),
                ));
                return Ok(finish(report, Some(&generator_name), settings, 0));
            }
        };
    let (fuzz_step, passed) = run_fuzz(
        generator,
        candidate.apworld_path,
        game,
        &options,
        settings,
        elapsed,
    )?;
    report.steps.push(fuzz_step);
    if passed {
        report.level = TrustLevel::Fuzzed;
    }
    let cases = if passed { settings.fuzz_cases } else { 0 };
    Ok(finish(report, Some(&generator_name), settings, cases))
}

/// Lance les cas de fuzzing. Rend l'étape et `true` si tous les cas sont passés.
fn run_fuzz(
    generator: &mut dyn Generator,
    apworld: &Path,
    game: &str,
    options: &[OptionSpec],
    settings: &CheckSettings,
    elapsed: &mut dyn FnMut() -> Duration,
) -> Result<(Step, bool), String> {
    let mut notes = Vec::new();
    let (mut generated, mut refused) = (0u32, 0u32);
    for number in 1..=settings.fuzz_cases {
        if elapsed() >= settings.time_limit {
            let detail = format!(
                "limite de temps ({} s) atteinte après {} cas sur {} : niveau non attribué",
                settings.time_limit.as_secs(),
                number - 1,
                settings.fuzz_cases
            );
            return Ok((
                step_with(STEP_FUZZ, StepStatus::Failed, detail, notes),
                false,
            ));
        }
        let case = make_case(settings.fuzz_seed, number, game, options);
        let outcome = generator.generate(apworld, &case.yaml, case.seed, settings.case_timeout);
        match outcome {
            RunOutcome::Generated => generated += 1,
            RunOutcome::Refused(reason) => {
                refused += 1;
                // Un refus sur des options valides mérite d'être lu ; les autres sont attendus.
                if case.kind == CaseKind::ValidOptions && notes.len() < 20 {
                    notes.push(format!(
                        "cas {} ({}, {}) refusé : {}",
                        case.number,
                        case.kind.label(),
                        case.summary,
                        short(&reason)
                    ));
                }
            }
            RunOutcome::Environment(reason) => return Err(reason),
            failure => {
                let detail = format!(
                    "cas {} ({}, {}, graine {}) : {}",
                    case.number,
                    case.kind.label(),
                    case.summary,
                    case.seed,
                    describe(&failure)
                );
                return Ok((
                    step_with(STEP_FUZZ, StepStatus::Failed, detail, notes),
                    false,
                ));
            }
        }
    }
    let detail = format!(
        "{} cas sans plantage ni boucle ({options} option(s) décrite(s)) : {generated} généré(s), {refused} refusé(s) proprement",
        settings.fuzz_cases,
        options = options.len()
    );
    Ok((
        step_with(STEP_FUZZ, StepStatus::Passed, detail, notes),
        true,
    ))
}

/// Remplit le détail écrit dans l'index signé (300 caractères au plus).
fn finish(
    mut report: PackageReport,
    generator: Option<&str>,
    settings: &CheckSettings,
    fuzz_cases: u32,
) -> PackageReport {
    let mut parts = vec![format!("CI Raftup {}", env!("CARGO_PKG_VERSION"))];
    if let Some(generator) = generator {
        parts.push(generator.to_string());
    }
    if report.level >= TrustLevel::Generates {
        parts.push(format!("{} graine(s)", settings.seeds));
    }
    if fuzz_cases > 0 {
        parts.push(format!(
            "{fuzz_cases} cas de fuzzing (graine {:#x})",
            settings.fuzz_seed
        ));
    }
    report.detail = short(&parts.join(" · "));
    report
}

/// Marque « non lancée » l'étape `from` et les suivantes qui ne sont pas encore là.
fn skip_rest(report: &mut PackageReport, from: &str, reason: &str) {
    let order = [STEP_STRUCTURE, STEP_GENERATE, STEP_FUZZ];
    let start = order.iter().position(|s| *s == from).unwrap_or(order.len());
    for name in &order[start..] {
        if !report.steps.iter().any(|s| s.name == *name) {
            report
                .steps
                .push(step(name, StepStatus::Skipped, reason.to_string()));
        }
    }
}

fn step(name: &str, status: StepStatus, detail: String) -> Step {
    step_with(name, status, detail, Vec::new())
}

fn step_with(name: &str, status: StepStatus, detail: String, notes: Vec<String>) -> Step {
    Step {
        name: name.to_string(),
        status,
        detail,
        notes,
    }
}

/// Une issue en mots.
fn describe(outcome: &RunOutcome) -> String {
    match outcome {
        RunOutcome::Generated => "généré".to_string(),
        RunOutcome::Refused(reason) => format!("refusé par le générateur : {}", short(reason)),
        RunOutcome::Crashed(reason) => format!("plantage : {}", short(reason)),
        RunOutcome::TimedOut(reason) => format!("boucle (délai dépassé) : {}", short(reason)),
        RunOutcome::Environment(reason) => format!("environnement : {}", short(reason)),
    }
}

/// Le texte sur une seule ligne (espaces et sauts de ligne réduits à une espace), au plus
/// 300 caractères. Les messages d'Archipelago s'étalent souvent sur plusieurs lignes.
pub fn short(text: &str) -> String {
    // `split_whitespace` coupe sur toute suite d'espaces, tabulations ou sauts de ligne.
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one_line.chars().take(300).collect()
}
