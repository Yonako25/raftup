//! Tirages au hasard reproductibles.
//!
//! On n'utilise pas la crate `rand` pour mélanger : son algorithme peut changer d'une version
//! à l'autre, et la même graine donnerait alors un autre placement. Ici, tout ne dépend que
//! des nombres renvoyés par `next_u64`.

use rand_core::Rng;

/// Entier tiré uniformément dans `0..n` (renvoie 0 si `n` vaut 0).
///
/// Un simple `x % n` favoriserait les petites valeurs quand 2⁶⁴ n'est pas un multiple de `n`.
/// On rejette donc les tirages sous `seuil = 2⁶⁴ mod n` : il reste un nombre de valeurs
/// multiple de `n`, et chaque reste a la même chance.
pub fn below<R: Rng + ?Sized>(rng: &mut R, n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    // `wrapping_neg` donne 2⁶⁴ − n ; modulo n, c'est 2⁶⁴ mod n.
    let threshold = n.wrapping_neg() % n;
    loop {
        let x = rng.next_u64();
        if x >= threshold {
            return x % n;
        }
    }
}

/// Mélange de Fisher-Yates : chaque ordre a la même chance.
///
/// `?Sized` permet aussi de passer un `&mut dyn Rng`.
pub fn shuffle<T, R: Rng + ?Sized>(rng: &mut R, items: &mut [T]) {
    for i in (1..items.len()).rev() {
        // `i + 1` tient dans un u64 et le tirage est ≤ i : les conversions sont sans perte.
        let j = below(rng, i as u64 + 1) as usize;
        items.swap(i, j);
    }
}
