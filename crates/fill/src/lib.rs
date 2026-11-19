//! Placement des objets d'un multiworld (ADR 0027, étape 3).
//!
//! Ce que fait la crate :
//!
//! - **faisabilité avant placement** ([`check_feasible`]) : avec tous les objets du pool en
//!   main, chaque lieu et chaque objectif doivent être atteignables ; sinon les options sont
//!   impossibles et l'erreur dit quelle règle bloque ;
//! - **placement** ([`fill`]) : les objets de progression d'abord, par **placement supposé**
//!   (on retire l'objet du pool, on suppose qu'on a tous les autres, on le pose dans un lieu
//!   libre encore atteignable), puis les objets utiles, puis le remplissage ; si l'ordre tiré
//!   mène à une impasse, on recommence ([`MAX_ATTEMPTS`] essais au plus) ;
//! - **faisabilité après placement** : un balayage depuis l'inventaire de départ doit tout
//!   atteindre (`accessibility: full`) ; sinon erreur, jamais de seed injouable ;
//! - **explication** ([`Blocked`]) : pour chaque lieu inatteignable, la clause de règle qui
//!   échoue (ou l'entrée qui bloque sa région).
//! - **équilibrage de progression** ([`balance`]) : après le placement, avance des objets
//!   de progression des joueurs en retard (`progression_balancing`, ADR 0027 étape 5) ;
//!
//! Le hasard vient d'un générateur passé par l'appelant (trait `Rng` de `rand_core`) ; le
//! mélange est écrit ici ([`shuffle`]) pour que le résultat ne dépende que de la suite de
//! nombres tirés : même générateur, même graine ⇒ même placement.

mod balance;
mod explain;
mod place;
mod shuffle;

pub use balance::{balance, BalanceReport, DEFAULT_BALANCING, MAX_BALANCING};
pub use explain::{explain, Blocked, Target};
pub use place::{check_feasible, fill, FillError, FillReport, MAX_ATTEMPTS};
pub use shuffle::{below, shuffle};
// Équilibrage par temps de jeu (ADR 0030), désactivé par défaut.
mod time_balance;
pub use time_balance::{
    simulate, time_balance, PlayerTime, TimeBalance, TimeBalanceReport, Timeline,
    DEFAULT_MAX_SWAPS, DEFAULT_SECONDS_PER_LOCATION,
};
