//! Regroupe le worker et sa descendance pour pouvoir tout tuer d'un coup (ADR 0016).
//!
//! Sous Windows, `Child::kill` ne tue que le process lui-même, pas ceux qu'il a lancés.
//! On met donc le worker dans un « Job Object » configuré `KILL_ON_JOB_CLOSE` : quand le
//! job est fermé (ou que Raftup meurt), Windows tue tous les process du job. La crate
//! `win32job` offre une API sûre : aucun `unsafe` dans notre code.
//!
//! Hors Windows : rien n'est fait (le type existe mais est vide). Les systèmes Unix
//! pourraient utiliser un groupe de process plus tard ; ce n'est pas nécessaire pour l'instant.

use std::process::Child;

/// Détient le job Windows ; le lâcher (`Drop`) ferme le job et tue toute la descendance.
pub(crate) struct KillGuard {
    #[cfg(windows)]
    _job: Option<win32job::Job>,
}

impl KillGuard {
    /// Affecte le process au job juste après son lancement.
    /// En cas d'échec, on continue sans job (le worker seul sera tué par `Child::kill`).
    #[cfg(windows)]
    pub(crate) fn attach(child: &Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        let job = (|| -> Result<win32job::Job, win32job::JobError> {
            let job = win32job::Job::create()?;
            let mut info = job.query_extended_limit_info()?;
            info.limit_kill_on_job_close();
            job.set_extended_limit_info(&info)?;
            // Le handle du process est un pointeur ; la crate attend un `isize`.
            job.assign_process(child.as_raw_handle() as isize)?;
            Ok(job)
        })();
        match job {
            Ok(job) => KillGuard { _job: Some(job) },
            Err(e) => {
                tracing::warn!(
                    "Job Object indisponible, la descendance du worker ne sera pas tuée : {e}"
                );
                KillGuard { _job: None }
            }
        }
    }

    #[cfg(not(windows))]
    pub(crate) fn attach(_child: &Child) -> Self {
        KillGuard {}
    }
}
