//! Équipage d'un salon local (ADR 0044) : les joueurs saisis par l'hôte, et le fichier
//! YAML que Raftup écrit pour chacun avant la génération.
//!
//! Tout est pur ici, sauf [`write_yamls`] qui écrit les fichiers.

use std::path::{Path, PathBuf};

/// Longueur maximale d'un nom de joueur pour Archipelago.
pub const MAX_NAME_CHARS: usize = 16;

/// Un joueur du salon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrewMember {
    /// Pseudo, utilisé comme nom de slot.
    pub name: String,
    /// Nom du jeu côté Archipelago (`ap_game` du profil).
    pub ap_game: String,
}

/// Un pseudo refusé, avec la raison.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("Indique un pseudo.")]
    Empty,
    #[error("Le pseudo fait au plus {MAX_NAME_CHARS} caractères.")]
    TooLong,
    #[error("Le pseudo ne peut pas contenir d'accolade ni de caractère invisible.")]
    BadCharacter,
    #[error("« {0} » est déjà dans l'équipage.")]
    Taken(String),
}

/// Vérifie un pseudo et le rend nettoyé (espaces autour retirés).
///
/// `others` : les pseudos déjà dans l'équipage. La comparaison ignore la casse.
pub fn check_name(name: &str, others: &[CrewMember]) -> Result<String, NameError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    // `chars().count()` compte des caractères, pas des octets (« é » = 2 octets).
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(NameError::TooLong);
    }
    if name.chars().any(|c| c == '{' || c == '}' || c.is_control()) {
        return Err(NameError::BadCharacter);
    }
    let lower = name.to_lowercase();
    if others.iter().any(|m| m.name.to_lowercase() == lower) {
        return Err(NameError::Taken(name.to_string()));
    }
    Ok(name.to_string())
}

/// Chaîne YAML entre guillemets doubles : seuls `\` et `"` sont à échapper, les
/// caractères de contrôle étant refusés plus haut (et remplacés ici par prudence).
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Le YAML d'un joueur : nom, jeu et section vide (options par défaut de l'apworld).
pub fn player_yaml(member: &CrewMember) -> String {
    let game = quoted(&member.ap_game);
    format!(
        "# Écrit par Raftup (salon local, ADR 0044).\nname: {}\ngame: {game}\n{game}: {{}}\n",
        quoted(&member.name)
    )
}

/// Écrit un YAML par joueur dans `dir` (`P01.yaml`, `P02.yaml`…), après avoir retiré
/// les anciens `.yaml` du dossier. Rend les chemins écrits, dans l'ordre de l'équipage.
pub fn write_yamls(dir: &Path, crew: &[CrewMember]) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "yaml") {
            std::fs::remove_file(&path)?;
        }
    }
    let mut written = Vec::with_capacity(crew.len());
    for (i, member) in crew.iter().enumerate() {
        let path = dir.join(format!("P{:02}.yaml", i + 1));
        std::fs::write(&path, player_yaml(member))?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str) -> CrewMember {
        CrewMember {
            name: name.into(),
            ap_game: "Slime Rancher".into(),
        }
    }

    #[test]
    fn pseudo_nettoye_et_verifie() {
        assert_eq!(check_name("  Léa ", &[]), Ok("Léa".into()));
        assert_eq!(check_name("   ", &[]), Err(NameError::Empty));
        assert_eq!(
            check_name("ééééééééééééééééé", &[]),
            Err(NameError::TooLong)
        );
        assert_eq!(
            check_name("éééééééééééééééé", &[]).map(|n| n.chars().count()),
            Ok(16)
        );
        assert_eq!(check_name("{player}", &[]), Err(NameError::BadCharacter));
        assert_eq!(check_name("a\tb", &[]), Err(NameError::BadCharacter));
        assert_eq!(
            check_name("LEA", &[member("léa"), member("Lea")]),
            Err(NameError::Taken("LEA".into()))
        );
    }

    #[test]
    fn yaml_d_un_joueur() {
        let yaml = player_yaml(&CrewMember {
            name: "Tom \"le\" \\ Grand".into(),
            ap_game: "Slime Rancher".into(),
        });
        assert_eq!(
            yaml,
            "# Écrit par Raftup (salon local, ADR 0044).\n\
             name: \"Tom \\\"le\\\" \\\\ Grand\"\n\
             game: \"Slime Rancher\"\n\
             \"Slime Rancher\": {}\n"
        );
    }

    #[test]
    fn ecrit_un_yaml_par_joueur_et_retire_les_anciens() -> std::io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("salon");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("vieux.yaml"), "x")?;
        std::fs::write(dir.join("garde.txt"), "x")?;
        let paths = write_yamls(&dir, &[member("A"), member("B")])?;
        assert_eq!(paths, vec![dir.join("P01.yaml"), dir.join("P02.yaml")]);
        assert!(!dir.join("vieux.yaml").exists());
        assert!(dir.join("garde.txt").exists());
        assert!(std::fs::read_to_string(&paths[1])?.contains("name: \"B\""));
        Ok(())
    }
}
