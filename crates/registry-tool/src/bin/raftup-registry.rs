//! Outil du registre Raftup (ADR 0032, 0045). **N'envoie rien** : il écrit des fichiers
//! dans un dossier local, que la mainteneuse met en ligne à la main.
//!
//! ```text
//! raftup-registry keygen <dossier>                          crée la paire de clés (hors du dépôt)
//! raftup-registry check <source.toml> <dossier-du-rapport> [options]
//!                                                           calcule le niveau de chaque paquet
//! raftup-registry publish <source.toml> <clé> <dossier> [--rapport <rapport.json>]
//!                                                           écrit index.json + index.json.minisig
//! raftup-registry verify <index.json> <index.json.minisig> <clé-publique-base64>
//! ```
//!
//! La clé secrète n'est jamais affichée.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use launcher_core::generation::{ensure_runtime, pins, SetupStep};
use launcher_core::{Downloader, HttpFetcher, Paths};
use registry::publish::{keygen, publish};
use registry::verify_index;
use registry_tool::check::{CheckSettings, Generator};
use registry_tool::official::OfficialGenerator;
use registry_tool::report::Report;
use registry_tool::run::{run_check, write_report, CheckRequest, PackageSource};

const USAGE: &str = "usage :
  raftup-registry keygen <dossier>
  raftup-registry check <source.toml> <dossier-du-rapport> [--paquet <nom>] [--telecharger]
      [--graines N] [--fuzz K] [--graine-fuzz G] [--limite <s>] [--delai-cas <s>]
      [--donnees <dossier>] [--travail <dossier>] [--structure-seule]
  raftup-registry publish <source.toml> <clé-secrète> <dossier-de-sortie> [--rapport <rapport.json>]
  raftup-registry verify <index.json> <index.json.minisig> <clé-publique>";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `as_str` : on compare des `&str` pour pouvoir filtrer avec des motifs de tranche.
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["keygen", dir] => {
            refuse_inside_repo(Path::new(dir))?;
            let public = keygen(Path::new(dir))?;
            println!("Paire de clés créée dans {dir}");
            println!("Clé publique (pour REGISTRY_PUBLIC_KEY) : {public}");
        }
        ["check", source, out, options @ ..] => check(source, out, options)?,
        ["publish", source, key, out, options @ ..] => {
            let claims = match options {
                [] => Vec::new(),
                ["--rapport", report] => {
                    let bytes =
                        std::fs::read(report).with_context(|| format!("lecture de {report}"))?;
                    Report::parse(&bytes)
                        .map_err(|e| anyhow!("{report} : {e}"))?
                        .claims()
                }
                _ => usage(),
            };
            let report = publish(Path::new(source), Path::new(key), Path::new(out), &claims)?;
            println!(
                "{} paquet(s) ; index : {} ; signature : {}",
                report.packages,
                report.index_path.display(),
                report.signature_path.display()
            );
            for (package, level) in &report.levels {
                println!("  {package} : {}", level.label());
            }
            for warning in &report.warnings {
                println!("  attention : {warning}");
            }
            println!("Clé publique de la signature : {}", report.public_key);
            println!("Rien n'a été envoyé : mettre ces deux fichiers en ligne à la main.");
        }
        ["verify", index, signature, public_key] => {
            let bytes = std::fs::read(index).with_context(|| format!("lecture de {index}"))?;
            let signature = std::fs::read_to_string(signature)
                .with_context(|| format!("lecture de {signature}"))?;
            let index = verify_index(&bytes, &signature, public_key)?;
            println!(
                "Index « {} » valide : {} paquet(s)",
                index.registry,
                index.packages.len()
            );
            for package in &index.packages {
                println!(
                    "  {} {} {} : {}",
                    package.kind.as_str(),
                    package.name,
                    package.version,
                    package.trust.level.label()
                );
            }
        }
        _ => usage(),
    }
    Ok(())
}

/// Affiche l'aide et quitte avec le code 2.
fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

/// Options de `check`.
struct CheckOptions {
    only: Option<String>,
    download: bool,
    data_dir: Option<PathBuf>,
    work_dir: PathBuf,
    structure_only: bool,
    settings: CheckSettings,
}

fn parse_check_options(options: &[&str]) -> Result<CheckOptions> {
    let mut parsed = CheckOptions {
        only: None,
        download: false,
        data_dir: None,
        work_dir: std::env::temp_dir().join("raftup-registry-check"),
        structure_only: false,
        settings: CheckSettings::default(),
    };
    let mut rest = options.iter();
    // `while let` : on consomme les options une par une ; celles qui ont une valeur
    // prennent aussi l'élément suivant.
    while let Some(option) = rest.next() {
        let mut value = || {
            rest.next()
                .copied()
                .ok_or_else(|| anyhow!("{option} demande une valeur"))
        };
        match *option {
            "--paquet" => parsed.only = Some(value()?.to_string()),
            "--telecharger" => parsed.download = true,
            "--structure-seule" => parsed.structure_only = true,
            "--donnees" => parsed.data_dir = Some(PathBuf::from(value()?)),
            "--travail" => parsed.work_dir = PathBuf::from(value()?),
            "--graines" => parsed.settings.seeds = number(option, value()?)?,
            "--fuzz" => parsed.settings.fuzz_cases = number(option, value()?)?,
            "--graine-fuzz" => parsed.settings.fuzz_seed = number(option, value()?)?,
            "--limite" => {
                parsed.settings.time_limit = Duration::from_secs(number(option, value()?)?)
            }
            "--delai-cas" => {
                parsed.settings.case_timeout = Duration::from_secs(number(option, value()?)?)
            }
            other => bail!("option inconnue : {other}\n{USAGE}"),
        }
    }
    Ok(parsed)
}

/// Lit un nombre (`u32` ou `u64` selon l'option : `FromStr` est un trait que les deux ont).
fn number<T: std::str::FromStr>(option: &str, text: &str) -> Result<T> {
    text.parse()
        .map_err(|_| anyhow!("{option} : « {text} » n'est pas un nombre"))
}

fn check(source: &str, out: &str, options: &[&str]) -> Result<()> {
    let options = parse_check_options(options)?;
    let fetcher = HttpFetcher::new();
    let packages = if options.download {
        PackageSource::Download(&fetcher)
    } else {
        PackageSource::LocalFiles
    };
    let request = CheckRequest {
        source: Path::new(source),
        only: options.only.as_deref(),
        packages,
        work_dir: &options.work_dir,
        settings: options.settings.clone(),
    };
    // Le générateur, s'il peut servir ici ; sinon la raison, écrite dans le rapport.
    let mut official = if options.structure_only {
        Err("vérification de la structure seule (--structure-seule)".to_string())
    } else if !cfg!(windows) {
        Err("le runtime Python du générateur n'existe que sous Windows (ADR 0017)".to_string())
    } else {
        Ok(official_generator(&options)?)
    };
    let generator: Result<&mut dyn Generator, String> = match &mut official {
        Ok(g) => Ok(g),
        Err(reason) => Err(reason.clone()),
    };
    let report = run_check(&request, generator).map_err(|e| anyhow!(e))?;
    let (json, markdown) = write_report(&report, Path::new(out)).map_err(|e| anyhow!(e))?;
    print!("{}", report.summary());
    println!("Rapport : {} et {}", json.display(), markdown.display());
    Ok(())
}

/// Installe (ou retrouve) le runtime épinglé, puis prépare le générateur officiel.
fn official_generator(options: &CheckOptions) -> Result<OfficialGenerator> {
    let paths = match &options.data_dir {
        Some(dir) => Paths::under(dir),
        None => Paths::from_os()?,
    };
    let pins = pins()?;
    let downloader = Downloader::new(paths.downloads_dir(), Box::new(HttpFetcher::new()));
    let mut last_step = None;
    let runtime = ensure_runtime(&paths, &pins, &downloader, &mut |step: SetupStep, _| {
        if last_step.as_ref() != Some(&step) {
            println!("Runtime du générateur : {step:?}");
            last_step = Some(step);
        }
    })?;
    Ok(OfficialGenerator::new(
        paths,
        runtime,
        options.work_dir.join("yaml"),
    ))
}

/// La clé secrète n'a pas sa place dans le dépôt (celui où l'outil a été compilé).
fn refuse_inside_repo(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("création de {}", dir.display()))?;
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if let (Ok(repo), Ok(dir)) = (repo.canonicalize(), dir.canonicalize()) {
        if dir.starts_with(&repo) {
            bail!("le dossier des clés doit être hors du dépôt");
        }
    }
    Ok(())
}
