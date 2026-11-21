//! Rapport des vérifications de la CI (ADR 0045) : `rapport.json` (relu par `publish`)
//! et `rapport.md` (lu par la mainteneuse).

use std::fmt::Write as _;

use registry::publish::TrustClaim;
use registry::{PackageKind, Trust, TrustLevel};
use serde::{Deserialize, Serialize};

/// Version du format de `rapport.json`.
pub const REPORT_FORMAT_VERSION: u32 = 1;

/// Taille maximale d'un `rapport.json` relu par `publish` (4 Mio).
pub const REPORT_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Issue d'une étape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Passed,
    Failed,
    /// Pas lancée (étape précédente échouée, générateur absent, profil…).
    Skipped,
}

impl StepStatus {
    fn mark(self) -> &'static str {
        match self {
            StepStatus::Passed => "✅",
            StepStatus::Failed => "❌",
            StepStatus::Skipped => "⏭️",
        }
    }
}

/// Une étape de la vérification (structure, génération, fuzzing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub status: StepStatus,
    pub detail: String,
    /// Remarques qui ne font pas échouer l'étape (refus propres du générateur…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Vérification d'un paquet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageReport {
    pub kind: PackageKind,
    pub name: String,
    pub version: String,
    /// Empreinte du fichier vérifié : `publish` ne reprend le niveau que pour ce fichier.
    pub sha256: String,
    pub size: u64,
    pub level: TrustLevel,
    /// Court détail écrit dans l'index signé (`trust.detail`).
    pub detail: String,
    pub steps: Vec<Step>,
}

/// Réglages utilisés, recopiés dans le rapport pour pouvoir le reproduire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSettings {
    pub seeds: u32,
    pub fuzz_cases: u32,
    pub fuzz_seed: u64,
    pub case_timeout_secs: u64,
    pub time_limit_secs: u64,
}

/// Le rapport complet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format_version: u32,
    /// Outil et version (`raftup-registry 0.1.0`).
    pub tool: String,
    /// Générateur utilisé (`Archipelago 0.6.8`), ou `None` s'il n'y en avait pas.
    pub generator: Option<String>,
    pub settings: ReportSettings,
    pub packages: Vec<PackageReport>,
}

impl Report {
    /// Lit un `rapport.json` (format vérifié en premier).
    pub fn parse(bytes: &[u8]) -> Result<Report, String> {
        if bytes.len() as u64 > REPORT_MAX_BYTES {
            return Err(format!("rapport de plus de {REPORT_MAX_BYTES} octets"));
        }
        #[derive(Deserialize)]
        struct Header {
            format_version: u32,
        }
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| format!("rapport illisible : {e}"))?;
        if header.format_version != REPORT_FORMAT_VERSION {
            return Err(format!(
                "rapport au format {} (ce programme lit le format {REPORT_FORMAT_VERSION})",
                header.format_version
            ));
        }
        serde_json::from_slice(bytes).map_err(|e| format!("rapport illisible : {e}"))
    }

    /// JSON du rapport (indenté, terminé par un saut de ligne).
    pub fn to_json(&self) -> Vec<u8> {
        // Ces structures ne contiennent que des types sérialisables : pas d'échec possible.
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    /// Niveaux à reprendre par `publish` : un par paquet, lié à son sha256.
    pub fn claims(&self) -> Vec<TrustClaim> {
        self.packages
            .iter()
            .map(|p| TrustClaim {
                kind: p.kind,
                name: p.name.clone(),
                version: p.version.clone(),
                sha256: p.sha256.clone(),
                trust: Trust {
                    level: p.level,
                    detail: (!p.detail.is_empty()).then(|| p.detail.clone()),
                },
            })
            .collect()
    }

    /// Le rapport lisible, en Markdown.
    pub fn to_markdown(&self) -> String {
        let mut md = String::from("# Rapport du registre Raftup\n\n");
        // `writeln!` dans une `String` ne peut pas échouer : on ignore le résultat.
        let _ = writeln!(md, "- Outil : {}", self.tool);
        let _ = writeln!(
            md,
            "- Générateur : {}",
            self.generator.as_deref().unwrap_or("aucun")
        );
        let s = &self.settings;
        let _ = writeln!(
            md,
            "- Réglages : {} graine(s), {} cas de fuzzing (graine {:#x}), {} s par cas, {} s au total",
            s.seeds, s.fuzz_cases, s.fuzz_seed, s.case_timeout_secs, s.time_limit_secs
        );
        md.push_str("\n| Paquet | Version | Niveau |\n| --- | --- | --- |\n");
        for p in &self.packages {
            let _ = writeln!(
                md,
                "| {} {} | {} | **{}** |",
                p.kind.as_str(),
                p.name,
                p.version,
                p.level.label()
            );
        }
        for p in &self.packages {
            let _ = writeln!(md, "\n## {} {} {}\n", p.kind.as_str(), p.name, p.version);
            let _ = writeln!(md, "- sha256 : `{}` ({} octets)", p.sha256, p.size);
            let _ = writeln!(
                md,
                "- Niveau : **{}** — {}",
                p.level.label(),
                p.level.meaning()
            );
            md.push('\n');
            for step in &p.steps {
                let _ = writeln!(
                    md,
                    "- {} {} : {}",
                    step.status.mark(),
                    step.name,
                    step.detail
                );
                for note in &step.notes {
                    let _ = writeln!(md, "  - {note}");
                }
            }
        }
        md
    }

    /// Résumé en quelques lignes pour le terminal.
    pub fn summary(&self) -> String {
        let mut text = String::new();
        for p in &self.packages {
            let _ = writeln!(
                text,
                "{} {} {} : {}",
                p.kind.as_str(),
                p.name,
                p.version,
                p.level.label()
            );
            if let Some(step) = p.steps.iter().find(|s| s.status == StepStatus::Failed) {
                let _ = writeln!(text, "  arrêt à « {} » : {}", step.name, step.detail);
            }
        }
        text
    }
}
