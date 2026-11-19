//! Écrit une seed APQuest générée en Rust : `cargo run -p generate --example apquest -- <dossier> [seed]`.
//!
//! Sert aux essais à la main (serveur officiel, client APQuest).

use generate::worlds::apquest::ApQuestOptions;
use generate::{generate, GameOptions, GenerateRequest, PlayerRequest};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("dossier de sortie manquant"))?;
    let seed = args.next().map(|s| s.parse()).transpose()?.unwrap_or(1);
    let generated = generate(&GenerateRequest {
        seed,
        players: vec![PlayerRequest::new(
            "Alice",
            GameOptions::ApQuest(ApQuestOptions::default()),
        )],
    })?;
    let path = std::path::Path::new(&dir).join(format!("AP_{}.archipelago", generated.seed_name));
    std::fs::write(&path, &generated.archipelago)?;
    println!("{}", path.display());
    Ok(())
}
