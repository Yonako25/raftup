//! Outil du registre de Raftup (binaire `raftup-registry`) : publication signée locale
//! (ADR 0032) et vérifications de la CI qui donnent à chaque paquet son niveau de
//! confiance (ADR 0042).
//!
//! - [`structure`] : niveau « structure valide » (zip sûr, dossier du monde, profil) ;
//! - [`check`] : montée des niveaux, derrière le trait [`check::Generator`] ;
//! - [`fuzz`] : cas de fuzzing déterministes (graine fixe) ;
//! - [`options`] : options d'un monde et YAML de test ;
//! - [`official`] : le vrai générateur, le générateur officiel épinglé de `launcher-core` ;
//! - [`report`] : `rapport.json` (relu par `publish`) et `rapport.md` ;
//! - [`run`] : `check` sur tout un fichier source.
//!
//! Cette crate ne signe rien à la place de la mainteneuse et n'envoie rien : `check` écrit
//! un rapport, `publish` écrit un index signé dans un dossier local.

pub mod check;
pub mod fuzz;
pub mod official;
pub mod options;
pub mod report;
pub mod run;
pub mod structure;
