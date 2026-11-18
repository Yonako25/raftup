//! Enregistrement du schéma `raftup://` dans le registre Windows (ADR 0020, décision 4).
//!
//! Tout est écrit sous `HKCU\Software\Classes`, donc sans droits administrateur.

use std::path::Path;

/// Nom du schéma d'URL (`raftup://...`).
pub const SCHEME: &str = "raftup";

/// Erreurs d'enregistrement du schéma d'URL.
#[derive(Debug, thiserror::Error)]
pub enum UrlSchemeError {
    /// Lecture ou écriture du registre impossible.
    #[error("accès au registre Windows impossible pour le schéma d'URL : {0}")]
    Registry(#[from] std::io::Error),
}

/// Commande de lancement associée au schéma : `"<exe>" "%1"`.
///
/// Le chemin est entouré de guillemets pour supporter les espaces. Un chemin
/// contenant `"` n'existe pas sous Windows (caractère interdit), donc aucun
/// échappement n'est nécessaire et la fonction ne refuse rien.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Enregistre le schéma `raftup` pour l'utilisateur courant.
///
/// Rend `Ok(false)` si le registre était déjà à jour (rien n'est écrit),
/// `Ok(true)` si des valeurs ont été écrites.
#[cfg(windows)]
pub fn ensure_registered(exe: &Path) -> Result<bool, UrlSchemeError> {
    ensure_registered_at("Software\\Classes", SCHEME, exe)
}

/// Version paramétrable (clé parente et schéma) pour tester sans toucher à la vraie clé.
#[cfg(windows)]
fn ensure_registered_at(
    classes_subkey: &str,
    scheme: &str,
    exe: &Path,
) -> Result<bool, UrlSchemeError> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let exe_str = exe.display().to_string();
    let base = format!("{classes_subkey}\\{scheme}");
    // (sous-clé relative, nom de valeur, contenu attendu). Le nom "" est la valeur par défaut.
    let wanted: [(String, &str, String); 4] = [
        (base.clone(), "", "URL:Raftup".to_string()),
        (base.clone(), "URL Protocol", String::new()),
        (
            format!("{base}\\DefaultIcon"),
            "",
            format!("\"{exe_str}\",0"),
        ),
        (
            format!("{base}\\shell\\open\\command"),
            "",
            command_line(exe),
        ),
    ];

    let root = RegKey::predef(HKEY_CURRENT_USER);
    // `ok()` : une clé ou une valeur absente n'est pas une erreur, juste « pas à jour ».
    let up_to_date = wanted.iter().all(|(path, name, value)| {
        root.open_subkey(path)
            .and_then(|k| k.get_value::<String, _>(name))
            .ok()
            .as_deref()
            == Some(value.as_str())
    });
    if up_to_date {
        return Ok(false);
    }

    for (path, name, value) in &wanted {
        let (key, _) = root.create_subkey(path)?;
        key.set_value(name, value)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_path_with_spaces_and_accents() {
        let exe = Path::new("C:\\Users\\Élodie\\Mes Jeux\\raftup.exe");
        assert_eq!(
            command_line(exe),
            "\"C:\\Users\\Élodie\\Mes Jeux\\raftup.exe\" \"%1\""
        );
    }

    #[test]
    fn command_line_simple_path() {
        assert_eq!(
            command_line(Path::new("C:\\raftup.exe")),
            "\"C:\\raftup.exe\" \"%1\""
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "écrit dans le registre Windows (clé de test supprimée à la fin)"]
    fn registers_then_is_idempotent() {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;

        let exe = Path::new("C:\\Test Élodie\\raftup.exe");
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let test_key = "Software\\Classes\\raftup-test";
        // Nettoyage préalable au cas où un essai précédent aurait échoué.
        let _ = root.delete_subkey_all(test_key);

        let first = ensure_registered_at("Software\\Classes", "raftup-test", exe);
        let second = ensure_registered_at("Software\\Classes", "raftup-test", exe);

        let read = |path: &str, name: &str| {
            root.open_subkey(path)
                .and_then(|k| k.get_value::<String, _>(name))
        };
        let default = read(test_key, "");
        let proto = read(test_key, "URL Protocol");
        let icon = read(&format!("{test_key}\\DefaultIcon"), "");
        let cmd = read(&format!("{test_key}\\shell\\open\\command"), "");

        // Suppression de la seule clé de test avant les assertions.
        let _ = root.delete_subkey_all(test_key);

        assert!(first.expect("premier appel"));
        assert!(!second.expect("second appel"));
        assert_eq!(default.expect("défaut"), "URL:Raftup");
        assert_eq!(proto.expect("URL Protocol"), "");
        assert_eq!(icon.expect("icône"), "\"C:\\Test Élodie\\raftup.exe\",0");
        assert_eq!(cmd.expect("commande"), command_line(exe));
    }
}
