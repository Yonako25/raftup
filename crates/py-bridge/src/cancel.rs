//! Jeton d'annulation partagé entre le thread qui appelle le pont et celui qui veut l'interrompre.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Jeton d'annulation. `Clone` copie seulement le pointeur (`Arc`) : toutes les copies
/// partagent le même drapeau, donc annuler depuis un thread est vu par les autres.
/// `AtomicBool` permet de lire et d'écrire le drapeau depuis plusieurs threads sans verrou.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// Crée un jeton non annulé.
    pub fn new() -> Self {
        Self::default()
    }

    /// Demande l'annulation (définitive : un jeton annulé le reste).
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Vrai si `cancel()` a été appelé sur ce jeton ou l'une de ses copies.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
