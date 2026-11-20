//! Bac à sable du worker Python (ADR 0034).
//!
//! Le code des apworlds est du Python arbitraire. Sous Windows, le worker est lancé en
//! **intégrité basse** (« Low Integrity », comme les onglets d'un navigateur) : il peut lire
//! les fichiers de l'utilisatrice, mais il ne peut écrire que dans les dossiers étiquetés
//! « bas ». Il ne peut donc ni modifier le runtime, ni les documents, ni la base de registre
//! de l'utilisatrice (sauf `AppDataLow`), ni piloter les fenêtres du launcher.
//!
//! Comment, sans `unsafe` : Windows démarre un programme avec l'intégrité la plus basse
//! entre celle du parent et celle du fichier `.exe`. On copie donc `python.exe` à côté de
//! lui-même (`python-raftup-low.exe`, pour qu'il retrouve sa bibliothèque et `Lib/`), on
//! étiquette la copie « basse » avec l'outil système `icacls.exe`, et on lance la copie.
//! Le dossier de la génération est étiqueté « bas » pendant l'appel, puis remis à
//! « moyen » quand le worker est arrêté.
//!
//! Hors Windows : rien n'est appliqué (repli documenté dans l'ADR 0034) ; le pont le dit
//! dans le journal et `PyBridge::sandbox_applied` renvoie `false`.

use std::path::{Path, PathBuf};

/// Réglages du bac à sable, donnés dans `BridgeOptions::sandbox`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sandbox {
    /// Le seul dossier où le worker peut écrire (le dossier de la génération).
    /// Les variables `TEMP` et `TMP` du worker pointent sur `<work_dir>/tmp`.
    pub work_dir: PathBuf,
}

impl Sandbox {
    pub fn new(work_dir: impl Into<PathBuf>) -> Self {
        Sandbox {
            work_dir: work_dir.into(),
        }
    }

    /// Dossier temporaire du worker.
    pub fn temp_dir(&self) -> PathBuf {
        self.work_dir.join("tmp")
    }
}

/// Le bac à sable existe-t-il sur ce système ?
pub const SANDBOX_SUPPORTED: bool = cfg!(windows);

/// Niveaux d'intégrité que le worker peut annoncer et qui comptent comme « bac à sable ».
pub(crate) fn is_low_integrity(level: Option<&str>) -> bool {
    matches!(level, Some("low") | Some("untrusted"))
}

/// Ce que `prepare` rend : l'exécutable à lancer, et le gardien qui remet les étiquettes
/// en place quand il est lâché (`Drop`).
pub(crate) struct Prepared {
    pub(crate) program: PathBuf,
    pub(crate) guard: LabelGuard,
}

/// Remet le dossier de travail en intégrité « moyenne » (le niveau normal) quand il est
/// lâché, pour qu'un worker suivant ne puisse plus toucher à cette génération.
pub(crate) struct LabelGuard {
    #[cfg(windows)]
    work_dir: Option<PathBuf>,
}

#[cfg(windows)]
impl Drop for LabelGuard {
    fn drop(&mut self) {
        if let Some(dir) = self.work_dir.take() {
            // Meilleur effort : le worker est déjà arrêté, une erreur ici n'a pas de suite.
            if let Err(e) = windows::restore(&dir) {
                tracing::warn!(
                    "bac à sable : étiquette de {} non remise à « moyen » : {e}",
                    dir.display()
                );
            }
        }
    }
}

/// Prépare le lancement : copie étiquetée de `python` et dossier de travail étiqueté.
#[cfg(windows)]
pub(crate) fn prepare(python: &Path, sandbox: &Sandbox) -> Result<Prepared, String> {
    std::fs::create_dir_all(sandbox.temp_dir())
        .map_err(|e| format!("{} : {e}", sandbox.temp_dir().display()))?;
    let program = windows::low_copy(python)?;
    windows::label_dir_low(&sandbox.work_dir)?;
    Ok(Prepared {
        program,
        guard: LabelGuard {
            work_dir: Some(sandbox.work_dir.clone()),
        },
    })
}

/// Hors Windows : rien à préparer, on lance `python` tel quel (repli, ADR 0034).
#[cfg(not(windows))]
pub(crate) fn prepare(python: &Path, sandbox: &Sandbox) -> Result<Prepared, String> {
    // Le dossier temporaire reste utile : il est dans le dossier de la génération.
    std::fs::create_dir_all(sandbox.temp_dir())
        .map_err(|e| format!("{} : {e}", sandbox.temp_dir().display()))?;
    Ok(Prepared {
        program: python.to_path_buf(),
        guard: LabelGuard {},
    })
}

/// Nom de la copie étiquetée : `python.exe` → `python-raftup-low.exe`.
// Servie seulement sous Windows (et par le test, partout) : sans ce `cfg`, la CI
// Linux et macOS refuse une fonction jamais utilisée (`-D warnings`).
#[cfg(any(windows, test))]
pub(crate) fn low_copy_name(python: &Path) -> Option<PathBuf> {
    let stem = python.file_stem()?.to_str()?;
    let dir = python.parent()?;
    Some(dir.join(format!("{stem}-raftup-low.exe")))
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::low_copy_name;

    /// SID « OWNER RIGHTS » : donner le contrôle total au propriétaire du fichier.
    const OWNER_RIGHTS: &str = "*S-1-3-4";

    /// Compteur pour des noms de fichiers temporaires uniques dans ce process.
    static NEXT_TMP: AtomicU32 = AtomicU32::new(0);

    /// `icacls.exe` du système, par chemin absolu (jamais cherché dans le `PATH`).
    fn icacls_path() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("icacls.exe")
    }

    /// Lance `icacls <path> <args>` ; erreur avec la sortie de l'outil si le code n'est pas 0.
    fn icacls(path: &Path, args: &[&str]) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        let output = Command::new(icacls_path())
            .arg(path)
            .args(args)
            // CREATE_NO_WINDOW : pas de console qui clignote.
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|e| format!("icacls introuvable ou impossible à lancer : {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        // La sortie d'icacls est dans la page de code de la console : lue « avec perte ».
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!(
            "icacls {args:?} a échoué ({}) : {text} {err}",
            output.status
        ))
    }

    /// Pose une étiquette d'intégrité. Poser une étiquette demande le droit « changer le
    /// propriétaire » (WRITE_OWNER) : sur un disque où l'on n'a que « modifier » (souvent
    /// le cas hors de `C:\Users`), on se donne d'abord ce droit sur CE fichier ou dossier,
    /// qui appartient à Raftup, puis on réessaie.
    fn set_label(path: &Path, level: &str, owner_grant: &str) -> Result<(), String> {
        if icacls(path, &["/setintegritylevel", level]).is_ok() {
            return Ok(());
        }
        let grant = format!("{OWNER_RIGHTS}:{owner_grant}");
        icacls(path, &["/grant", &grant])?;
        icacls(path, &["/setintegritylevel", level])
    }

    /// Étiquette le dossier « bas », hérité par tout ce qu'il contient et contiendra.
    pub(super) fn label_dir_low(dir: &Path) -> Result<(), String> {
        set_label(dir, "(OI)(CI)low", "(OI)(CI)(F)")
            .map_err(|e| format!("{} : {e}", dir.display()))?;
        // Les fichiers et dossiers déjà présents reçoivent aussi l'étiquette (`/T`).
        icacls(
            dir,
            &["/setintegritylevel", "(OI)(CI)low", "/T", "/C", "/Q"],
        )
        .map_err(|e| format!("{} : {e}", dir.display()))
    }

    /// Remet le dossier et tout son contenu en intégrité « moyenne ».
    pub(super) fn restore(dir: &Path) -> Result<(), String> {
        if !dir.exists() {
            // Dossier déjà supprimé (génération échouée) : rien à faire.
            return Ok(());
        }
        icacls(dir, &["/setintegritylevel", "medium", "/T", "/C", "/Q"])
    }

    /// Copie étiquetée « basse » de `python`, à côté de lui. Réutilisée si elle existe déjà
    /// avec le même contenu. Écrite d'abord sous un nom temporaire, étiquetée, puis renommée :
    /// le nom final ne désigne jamais une copie sans étiquette.
    pub(super) fn low_copy(python: &Path) -> Result<PathBuf, String> {
        let fail = |what: String| format!("copie de {} : {what}", python.display());
        let target = low_copy_name(python).ok_or_else(|| fail("nom de fichier invalide".into()))?;
        let original = std::fs::read(python).map_err(|e| fail(e.to_string()))?;
        if same_content(&target, &original) {
            return Ok(target);
        }
        let n = NEXT_TMP.fetch_add(1, Ordering::Relaxed);
        let tmp = target.with_extension(format!("{}-{n}.tmp", std::process::id()));
        std::fs::write(&tmp, &original).map_err(|e| fail(format!("{} : {e}", tmp.display())))?;
        let labelled = set_label(&tmp, "low", "(F)");
        let renamed = labelled.and_then(|()| {
            std::fs::rename(&tmp, &target).map_err(|e| format!("{} : {e}", target.display()))
        });
        match renamed {
            Ok(()) => Ok(target),
            Err(e) => {
                // On ne garde jamais de copie temporaire.
                let _ = std::fs::remove_file(&tmp);
                // Un autre lancement a pu poser la même copie entre-temps (fichier en usage).
                if same_content(&target, &original) {
                    Ok(target)
                } else {
                    Err(fail(e))
                }
            }
        }
    }

    fn same_content(path: &Path, expected: &[u8]) -> bool {
        std::fs::read(path).is_ok_and(|bytes| bytes == expected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_de_la_copie() {
        assert_eq!(
            low_copy_name(Path::new("C:/r/python/3.12/python.exe")),
            Some(PathBuf::from("C:/r/python/3.12/python-raftup-low.exe"))
        );
    }

    #[test]
    fn niveaux_acceptes() {
        assert!(is_low_integrity(Some("low")));
        assert!(is_low_integrity(Some("untrusted")));
        assert!(!is_low_integrity(Some("medium")));
        assert!(!is_low_integrity(None));
    }
}
