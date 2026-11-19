//! Outil de publication du registre Raftup (ADR 0032). **N'envoie rien** : il écrit des
//! fichiers dans un dossier local, que la mainteneuse met en ligne à la main.
//!
//! ```text
//! raftup-registry keygen <dossier>                          crée la paire de clés (hors du dépôt)
//! raftup-registry publish <source.toml> <clé> <dossier>     écrit index.json + index.json.minisig
//! raftup-registry verify <index.json> <index.json.minisig> <clé-publique-base64>
//! ```
//!
//! La clé secrète n'est jamais affichée.

use std::path::Path;

use anyhow::{bail, Context, Result};
use registry::publish::{keygen, publish};
use registry::verify_index;

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
        ["publish", source, key, out] => {
            let report = publish(Path::new(source), Path::new(key), Path::new(out))?;
            println!(
                "{} paquet(s) ; index : {} ; signature : {}",
                report.packages,
                report.index_path.display(),
                report.signature_path.display()
            );
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
        }
        _ => {
            eprintln!(
                "usage :\n  raftup-registry keygen <dossier>\n  \
                 raftup-registry publish <source.toml> <clé-secrète> <dossier-de-sortie>\n  \
                 raftup-registry verify <index.json> <index.json.minisig> <clé-publique>"
            );
            std::process::exit(2);
        }
    }
    Ok(())
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
