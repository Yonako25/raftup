//! Démarrage du launcher : lien reçu de Windows, instance unique, schéma `raftup://`
//! (ADR 0020, décisions 4 et 5).
//!
//! Quand on clique un lien `raftup://…`, Windows lance `raftup.exe "<lien>"`. Si Raftup
//! est déjà ouvert, ce deuxième processus dépose le lien dans la « boîte aux lettres »
//! de la première fenêtre (`launcher_core::instance`) puis s'arrête aussitôt.

use std::path::PathBuf;

use launcher_core::instance::{self, Guard, Instance};

/// Message déposé par une deuxième instance lancée SANS lien (double-clic sur Raftup
/// déjà ouvert) : la première fenêtre se remet simplement au premier plan.
pub const FOCUS: &str = "focus";

/// Ce que `main` doit faire.
pub enum Start {
    /// Ouvrir la fenêtre. `guard` garde le verrou de l'instance unique : il doit vivre
    /// jusqu'à la fin du programme (`None` si le verrou n'a pas pu être pris : Raftup
    /// démarre quand même, sans instance unique).
    Run {
        guard: Option<Guard>,
        /// Dossier de la boîte aux lettres, relu par la fenêtre.
        instance_dir: Option<PathBuf>,
        /// Lien reçu en argument, à ouvrir dans l'écran « Rejoindre ».
        link: Option<String>,
    },
    /// Une autre fenêtre Raftup est ouverte et a reçu le lien : on s'arrête là.
    Forwarded,
}

/// Le lien `raftup:` passé par Windows, s'il y en a un.
///
/// Dès qu'un argument commence par `raftup:`, c'est lui et lui seul qui compte : un
/// lien mal encodé qui se découperait en plusieurs arguments ne peut pas en ajouter
/// d'autres (ADR 0020, risques). Le premier argument (le chemin du programme) est sauté.
pub fn link_from_args(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter().skip(1).find_map(|arg| {
        // `get(..7)` rend `None` au lieu de paniquer si l'argument est court ou si le
        // 7e octet tombe au milieu d'un caractère accentué.
        let rest = arg.get(7..)?;
        arg.get(..7)
            .filter(|start| start.eq_ignore_ascii_case("raftup:"))
            // Schéma remis en minuscules : `parse_invite` n'accepte que `raftup://`.
            .map(|_| format!("raftup:{rest}"))
    })
}

/// Prépare le démarrage : lit le lien, prend le verrou ou passe la main, et enregistre
/// le schéma `raftup://`. Ne bloque pas plus que quelques accès disque.
pub fn prepare(link: Option<String>) -> Start {
    let instance_dir = match crate::screens::library::load_paths() {
        Ok(paths) => paths.instance_dir(),
        Err(e) => {
            tracing::warn!("instance unique désactivée : {e}");
            return Start::Run {
                guard: None,
                instance_dir: None,
                link,
            };
        }
    };
    match instance::acquire(&instance_dir) {
        Ok(Instance::First(guard)) => {
            register_scheme();
            Start::Run {
                guard: Some(guard),
                instance_dir: Some(instance_dir),
                link,
            }
        }
        Ok(Instance::Busy) => {
            let message = link.as_deref().unwrap_or(FOCUS);
            match instance::forward(&instance_dir, message) {
                Ok(()) => Start::Forwarded,
                // Impossible de prévenir l'autre fenêtre : on en ouvre une quand même,
                // plutôt que de ne rien montrer au joueur.
                Err(e) => {
                    tracing::warn!("lien non transmis à la fenêtre déjà ouverte : {e}");
                    Start::Run {
                        guard: None,
                        instance_dir: None,
                        link,
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("verrou de l'instance unique impossible : {e}");
            Start::Run {
                guard: None,
                instance_dir: None,
                link,
            }
        }
    }
}

/// Adopte l'adresse de `rafthub.toml` posé à côté de l'exécutable (ADR 0036), si aucun
/// hub n'est encore réglé. À appeler avant d'ouvrir la fenêtre : l'écran « Amis » lit
/// ensuite l'adresse dans la configuration, comme si le joueur l'avait tapée.
pub fn adopt_rafthub_file() {
    use launcher_core::config;
    use launcher_core::rafthub::{self, Adoption};

    let exe_dir = match std::env::current_exe() {
        Ok(exe) => match exe.parent() {
            Some(dir) => dir.to_path_buf(),
            None => return,
        },
        Err(e) => {
            tracing::warn!(
                "{} non lu : chemin du programme introuvable : {e}",
                rafthub::BUNDLED_FILE
            );
            return;
        }
    };
    let file = match crate::screens::library::load_paths() {
        Ok(paths) => paths.config_file(),
        Err(e) => {
            tracing::warn!("{} non lu : {e}", rafthub::BUNDLED_FILE);
            return;
        }
    };
    let mut loaded = match config::load(&file) {
        Ok(loaded) => loaded,
        Err(e) => {
            tracing::warn!(
                "{} non lu : configuration illisible : {e}",
                rafthub::BUNDLED_FILE
            );
            return;
        }
    };
    match rafthub::adopt_bundled(&mut loaded.config, &exe_dir) {
        Adoption::AlreadySet => {}
        Adoption::Absent => {
            tracing::info!("pas de {} : Rafthub par défaut", rafthub::BUNDLED_FILE);
        }
        Adoption::Invalid(e) => tracing::warn!("{e} ; Rafthub par défaut"),
        Adoption::Adopted(url) => match loaded.config.save(&file) {
            Ok(()) => tracing::info!(
                "Rafthub {} adopté depuis {}",
                url.host(),
                rafthub::BUNDLED_FILE
            ),
            Err(e) => tracing::warn!("Rafthub {} non enregistré : {e}", url.host()),
        },
    }
}

/// Enregistre `raftup://` pour ce programme (Windows, binaire livré seulement).
///
/// Pas en debug : un `cargo run` détournerait les liens vers un exécutable de
/// développement, qui disparaît au prochain `cargo clean`.
fn register_scheme() {
    #[cfg(windows)]
    if !cfg!(debug_assertions) {
        match std::env::current_exe() {
            Ok(exe) => match launcher_core::url_scheme::ensure_registered(&exe) {
                Ok(true) => tracing::info!("schéma raftup:// enregistré pour {}", exe.display()),
                Ok(false) => {}
                Err(e) => tracing::warn!("schéma raftup:// non enregistré : {e}"),
            },
            Err(e) => tracing::warn!("chemin du programme introuvable : {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lien_dans_les_arguments() {
        assert_eq!(link_from_args(args(&["raftup.exe"])), None);
        assert_eq!(
            link_from_args(args(&["raftup.exe", "raftup://join?v=1"])),
            Some("raftup://join?v=1".to_string())
        );
        // Majuscules acceptées pour le schéma, remis en minuscules ; le reste est gardé.
        assert_eq!(
            link_from_args(args(&["raftup.exe", "RAFTUP://join?slot=Bob"])),
            Some("raftup://join?slot=Bob".to_string())
        );
        // Le chemin du programme n'est jamais pris pour un lien.
        assert_eq!(link_from_args(args(&["raftup:évité"])), None);
        // Argument court ou sans rapport : ignoré, sans plantage sur l'accent.
        assert_eq!(link_from_args(args(&["raftup.exe", "é", "--x"])), None);
    }

    #[test]
    fn seul_le_lien_compte() {
        // Un lien découpé en plusieurs arguments n'en ajoute pas d'autres.
        assert_eq!(
            link_from_args(args(&["raftup.exe", "--debug", "raftup://join?v=1", "--x"])),
            Some("raftup://join?v=1".to_string())
        );
    }
}
