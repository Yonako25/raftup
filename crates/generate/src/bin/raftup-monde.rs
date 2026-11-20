//! `raftup-monde` : conversion assistée entre apworld et monde natif (ADR 0042).
//!
//! ```text
//! raftup-monde importer <export.json> [--joueur N] [--seed <seed.zip|.archipelago>] [--sortie world.toml]
//! raftup-monde apworld <world.toml> [--module nom] [--sortie dossier]
//! ```
//!
//! - `importer` : l'export JSON d'une génération (moteur « export », ADR 0028) devient un
//!   `world.toml`. Avec `--seed` (la seed de la même génération), le `slot_data` du joueur
//!   est recopié. Les points à vérifier sont affichés et écrits en tête du fichier.
//! - `apworld` : un monde natif devient un `.apworld` pour Archipelago.
//!
//! Rien n'est envoyé nulle part ; seuls les fichiers de sortie sont écrits.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

/// Une seed ou un export ne dépasse pas 64 Mio (NF08).
const MAX_INPUT: u64 = 64 * 1024 * 1024;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("importer") => importer(&args[1..]),
        Some("apworld") => apworld(&args[1..]),
        _ => {
            eprintln!(
                "usage :\n  raftup-monde importer <export.json> [--joueur N] [--seed <seed>] [--sortie world.toml]\n  \
                 raftup-monde apworld <world.toml> [--module nom] [--sortie dossier]"
            );
            std::process::exit(2);
        }
    }
}

/// Arguments : un chemin, puis des paires `--nom valeur`.
struct Args {
    input: PathBuf,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(args: &[String], known: &[&str]) -> Result<Args> {
        let mut input = None;
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if let Some(name) = arg.strip_prefix("--") {
                if !known.contains(&name) {
                    bail!("option inconnue : --{name}");
                }
                let value = it.next().with_context(|| format!("--{name} sans valeur"))?;
                flags.push((name.to_string(), value.clone()));
            } else if input.is_none() {
                input = Some(PathBuf::from(arg));
            } else {
                bail!("argument en trop : {arg}");
            }
        }
        Ok(Args {
            input: input.context("fichier d'entrée manquant")?,
            flags,
        })
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    multidata::read_file_bounded(path, MAX_INPUT)
        .with_context(|| format!("lecture de {}", path.display()))
}

fn importer(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["joueur", "seed", "sortie"])?;
    let export = generate::imported::read_export(&read(&args.input)?)?;
    let player: u32 = match args.get("joueur") {
        Some(n) => n.parse().context("--joueur : un numéro de joueur")?,
        None => 1,
    };
    let slot_data = match args.get("seed") {
        Some(path) => Some(slot_data(Path::new(path), player)?),
        None => None,
    };
    let conversion = generate::convert::convert(&export, player, slot_data.as_ref())?;
    let out = PathBuf::from(args.get("sortie").unwrap_or("world.toml"));
    std::fs::write(&out, &conversion.toml)
        .with_context(|| format!("écriture de {}", out.display()))?;
    println!("Monde natif écrit dans {}", out.display());
    if !conversion.notes.is_empty() {
        println!("À vérifier :");
        for note in &conversion.notes {
            println!("- {note}");
        }
    }
    Ok(())
}

/// Le `slot_data` d'un joueur dans une seed (`.archipelago`, ou zip d'Archipelago).
fn slot_data(path: &Path, player: u32) -> Result<serde_json::Value> {
    let bytes = read(path)?;
    let archipelago = if bytes.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        let mut found = None;
        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            if entry.name()?.ends_with(".archipelago") {
                let mut content = Vec::new();
                // `take` borne la lecture : une archive piégée ne remplit pas la mémoire.
                entry.take(MAX_INPUT).read_to_end(&mut content)?;
                found = Some(content);
                break;
            }
        }
        found.context("pas de .archipelago dans le zip")?
    } else {
        bytes
    };
    let md = multidata::load_archipelago(&archipelago, &multidata::ArchiveLimits::default())?;
    md.slot_data
        .get(&player)
        .cloned()
        .with_context(|| format!("pas de slot_data pour le joueur {player}"))
}

fn apworld(args: &[String]) -> Result<()> {
    let args = Args::parse(args, &["module", "sortie"])?;
    let world = worlds_native::load_path(&args.input)?;
    let mut config = worlds_native::apworld::ApworldConfig::for_world(&world);
    if let Some(module) = args.get("module") {
        config.module = module.to_string();
    }
    let files = worlds_native::apworld::to_apworld(&world, &config)?;
    let dir = PathBuf::from(args.get("sortie").unwrap_or("."));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.apworld", config.module));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for f in &files {
        zip.start_file(format!("{}/{}", config.module, f.name), options)?;
        zip.write_all(f.contents.as_bytes())?;
    }
    zip.finish()?;
    println!("apworld écrit dans {}", path.display());
    Ok(())
}
