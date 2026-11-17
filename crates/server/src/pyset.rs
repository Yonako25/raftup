//! Imitation d'un `set` de CPython 3.12, pour parcourir les éléments dans le
//! même ordre que le serveur officiel (ADR 0018).
//!
//! Le serveur officiel range les indices de chaque joueur dans un `set` Python.
//! Python parcourt un `set` dans l'ordre de sa table de hachage : cet ordre
//! dépend du hachage de chaque élément, de la taille de la table et de
//! l'historique des ajouts et des retraits. On refait ici exactement le même
//! calcul que `Objects/setobject.c` (CPython 3.12) :
//!
//! - table de taille puissance de 2 (8 au départ), sondage linéaire par
//!   paquets de 10 cases, puis saut pseudo-aléatoire (`perturb`) ;
//! - un retrait laisse une case « fantôme » (`dummy`) ;
//! - agrandissement quand 3/5 des cases ont servi.
//!
//! Et le hachage de Python pour les entiers, les textes et les tuples
//! (`Python/pyhash.c`, `Objects/tupleobject.c`, `Objects/longobject.c`).
//!
//! Limite : Python tire au hasard la clé de hachage des textes à chaque
//! lancement. On prend la clé nulle, celle de `PYTHONHASHSEED=0`. L'ordre est
//! donc exact pour les éléments dont les textes sont vides (leur hachage vaut
//! toujours 0), et pour un serveur officiel lancé avec `PYTHONHASHSEED=0`.

/// Taille de départ d'une table (`PySet_MINSIZE`).
const MIN_SIZE: usize = 8;
/// Cases voisines essayées avant de sauter ailleurs (`LINEAR_PROBES`).
const LINEAR_PROBES: usize = 9;
/// Décalage de `perturb` à chaque saut (`PERTURB_SHIFT`).
const PERTURB_SHIFT: u32 = 5;

/// Valeur `hash()` que Python donne à un objet.
pub(crate) trait PyHash {
    fn py_hash(&self) -> i64;
}

impl PyHash for i64 {
    fn py_hash(&self) -> i64 {
        hash_int(*self)
    }
}

/// Numéro de slot ou d'équipe : un entier en Python.
impl PyHash for u32 {
    fn py_hash(&self) -> i64 {
        hash_int(i64::from(*self))
    }
}

/// `(équipe, slot)` : un tuple de deux entiers en Python.
impl PyHash for (u32, u32) {
    fn py_hash(&self) -> i64 {
        hash_tuple(&[self.0.py_hash(), self.1.py_hash()])
    }
}

/// Un indice (`NetUtils.Hint`) : haché sur 5 de ses champs seulement,
/// `(receiving_player, finding_player, location, item, entrance)`, mais
/// comparé sur tous (`==` de Rust, comme l'égalité d'un tuple nommé).
impl PyHash for ap_protocol::Hint {
    fn py_hash(&self) -> i64 {
        hash_tuple(&[
            self.receiving_player.py_hash(),
            self.finding_player.py_hash(),
            hash_int(self.location),
            hash_int(self.item),
            hash_str(&self.entrance),
        ])
    }
}

/// Hachage d'un entier Python : la valeur modulo 2^61 - 1, avec le signe ;
/// -1 est réservé aux erreurs en C, il devient -2.
pub(crate) fn hash_int(value: i64) -> i64 {
    const MODULUS: u64 = (1 << 61) - 1;
    // `unsigned_abs` évite le dépassement de `i64::MIN.abs()`.
    let magnitude = (value.unsigned_abs() % MODULUS) as i64;
    let hash = if value < 0 { -magnitude } else { magnitude };
    if hash == -1 {
        -2
    } else {
        hash
    }
}

/// Hachage d'un texte Python avec la clé nulle (`PYTHONHASHSEED=0`) :
/// SipHash-1-3 sur la représentation interne du texte (1, 2 ou 4 octets par
/// caractère selon le plus grand caractère). Le texte vide vaut 0.
pub(crate) fn hash_str(text: &str) -> i64 {
    if text.is_empty() {
        return 0;
    }
    let widest = text.chars().map(u32::from).max().unwrap_or(0);
    let mut bytes = Vec::with_capacity(text.len() * 4);
    for c in text.chars() {
        let code = u32::from(c);
        if widest < 0x100 {
            bytes.push(code as u8);
        } else if widest < 0x1_0000 {
            bytes.extend_from_slice(&(code as u16).to_le_bytes());
        } else {
            bytes.extend_from_slice(&code.to_le_bytes());
        }
    }
    let hash = siphash13(0, 0, &bytes) as i64;
    if hash == -1 {
        -2
    } else {
        hash
    }
}

/// Hachage d'un tuple Python (variante de xxHash, CPython 3.8 et après), à
/// partir du hachage de chaque élément.
pub(crate) fn hash_tuple(lanes: &[i64]) -> i64 {
    const PRIME_1: u64 = 11_400_714_785_074_694_791;
    const PRIME_2: u64 = 14_029_467_366_897_019_727;
    const PRIME_5: u64 = 2_870_177_450_012_600_261;
    // Calcul en entiers non signés qui « bouclent », comme `Py_uhash_t` en C.
    let mut acc = PRIME_5;
    for &lane in lanes {
        acc = acc.wrapping_add((lane as u64).wrapping_mul(PRIME_2));
        acc = acc.rotate_left(31);
        acc = acc.wrapping_mul(PRIME_1);
    }
    acc = acc.wrapping_add(lanes.len() as u64 ^ (PRIME_5 ^ 3_527_539));
    if acc == u64::MAX {
        1_546_275_796
    } else {
        acc as i64
    }
}

/// SipHash-1-3 tel que l'écrit `Python/pyhash.c`.
fn siphash13(k0: u64, k1: u64, src: &[u8]) -> u64 {
    let mut v0 = k0 ^ 0x736f_6d65_7073_6575;
    let mut v1 = k1 ^ 0x646f_7261_6e64_6f6d;
    let mut v2 = k0 ^ 0x6c79_6765_6e65_7261;
    let mut v3 = k1 ^ 0x7465_6462_7974_6573;

    // Un tour de mélange (`SINGLE_ROUND`).
    fn round(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
        *v0 = v0.wrapping_add(*v1);
        *v2 = v2.wrapping_add(*v3);
        *v1 = v1.rotate_left(13) ^ *v0;
        *v3 = v3.rotate_left(16) ^ *v2;
        *v0 = v0.rotate_left(32);
        *v2 = v2.wrapping_add(*v1);
        *v0 = v0.wrapping_add(*v3);
        *v1 = v1.rotate_left(17) ^ *v2;
        *v3 = v3.rotate_left(21) ^ *v0;
        *v2 = v2.rotate_left(32);
    }

    // Blocs de 8 octets lus en petit-boutiste, puis le reste.
    let (words, rest) = src.as_chunks::<8>();
    for word in words {
        let m = u64::from_le_bytes(*word);
        v3 ^= m;
        round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }
    let mut last = [0u8; 8];
    last[..rest.len()].copy_from_slice(rest);
    let b = ((src.len() as u64) << 56) | u64::from_le_bytes(last);

    v3 ^= b;
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= b;
    v2 ^= 0xff;
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    (v0 ^ v1) ^ (v2 ^ v3)
}

/// Une case de la table.
#[derive(Debug, Clone)]
enum Slot<T> {
    /// Jamais servie (`key == NULL`).
    Empty,
    /// Élément retiré (`dummy`) : la recherche continue au-delà.
    Dummy,
    Active {
        hash: i64,
        key: T,
    },
}

/// Un `set` Python. Les éléments sont comparés avec `==` (comme
/// `PyObject_RichCompareBool`) et rangés selon [`PyHash`].
#[derive(Debug, Clone)]
pub(crate) struct PySet<T> {
    table: Vec<Slot<T>>,
    /// Cases actives ou fantômes (`fill`).
    fill: usize,
    /// Cases actives (`used`).
    used: usize,
}

impl<T> Default for PySet<T> {
    fn default() -> Self {
        PySet::new()
    }
}

/// Saut suivant du sondage : `i = (i * 5 + 1 + perturb) & mask`.
fn next_index(i: usize, perturb: &mut u64, mask: usize) -> usize {
    *perturb >>= PERTURB_SHIFT;
    (i.wrapping_mul(5)
        .wrapping_add(1)
        .wrapping_add(*perturb as usize))
        & mask
}

impl<T> PySet<T> {
    /// `set()` vide.
    pub(crate) fn new() -> Self {
        PySet {
            table: empty_table(MIN_SIZE),
            fill: 0,
            used: 0,
        }
    }

    /// Les éléments dans l'ordre où Python les parcourt.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        self.table.iter().filter_map(|slot| match slot {
            Slot::Active { key, .. } => Some(key),
            _ => None,
        })
    }

    fn mask(&self) -> usize {
        self.table.len() - 1
    }

    /// Range un élément absent dans la première case vide (`set_insert_clean`).
    fn insert_clean(table: &mut [Slot<T>], hash: i64, key: T) {
        let mask = table.len() - 1;
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        loop {
            if matches!(table[i], Slot::Empty) {
                table[i] = Slot::Active { hash, key };
                return;
            }
            if i + LINEAR_PROBES <= mask {
                for j in 1..=LINEAR_PROBES {
                    if matches!(table[i + j], Slot::Empty) {
                        table[i + j] = Slot::Active { hash, key };
                        return;
                    }
                }
            }
            i = next_index(i, &mut perturb, mask);
        }
    }

    /// Reconstruit la table pour plus de `min_used` éléments, sans fantômes
    /// (`set_table_resize`).
    fn resize(&mut self, min_used: usize) {
        let mut new_size = MIN_SIZE;
        while new_size <= min_used {
            new_size <<= 1;
        }
        // Petite table déjà à la bonne taille et sans fantôme : rien à faire.
        if new_size == MIN_SIZE && self.table.len() == MIN_SIZE && self.fill == self.used {
            return;
        }
        // `std::mem::replace` prend l'ancienne table et laisse la nouvelle à
        // sa place : on peut ensuite déplacer les éléments sans les copier.
        let old = std::mem::replace(&mut self.table, empty_table(new_size));
        self.fill = self.used;
        for slot in old {
            if let Slot::Active { hash, key } = slot {
                Self::insert_clean(&mut self.table, hash, key);
            }
        }
    }
}

fn empty_table<T>(size: usize) -> Vec<Slot<T>> {
    (0..size).map(|_| Slot::Empty).collect()
}

impl<T: PyHash + PartialEq + Clone> PySet<T> {
    /// `set(iterable)` : ajoute les éléments un par un, dans l'ordre donné.
    pub(crate) fn from_ordered<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut set = PySet::new();
        for item in items {
            set.add(item);
        }
        set
    }

    /// Case de l'élément, ou `None` s'il est absent (`set_lookkey`).
    fn find(&self, key: &T, hash: i64) -> Option<usize> {
        let mask = self.mask();
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        loop {
            let probes = if i + LINEAR_PROBES <= mask {
                LINEAR_PROBES
            } else {
                0
            };
            for j in 0..=probes {
                match &self.table[i + j] {
                    Slot::Empty => return None,
                    Slot::Active { hash: h, key: k } if *h == hash && k == key => {
                        return Some(i + j);
                    }
                    _ => {}
                }
            }
            i = next_index(i, &mut perturb, mask);
        }
    }

    pub(crate) fn contains(&self, key: &T) -> bool {
        self.find(key, key.py_hash()).is_some()
    }

    /// `set.add` (`set_add_entry`). Renvoie `true` si l'élément était absent.
    pub(crate) fn add(&mut self, key: T) -> bool {
        let hash = key.py_hash();
        self.add_with_hash(key, hash)
    }

    fn add_with_hash(&mut self, key: T, hash: i64) -> bool {
        let mask = self.mask();
        let mut perturb = hash as u64;
        let mut i = (hash as u64 as usize) & mask;
        // Comme en C : la *dernière* case fantôme vue avant une case vide.
        let mut free_slot: Option<usize> = None;
        let empty = loop {
            let probes = if i + LINEAR_PROBES <= mask {
                LINEAR_PROBES
            } else {
                0
            };
            let mut found_empty = None;
            for j in 0..=probes {
                match &self.table[i + j] {
                    Slot::Empty => {
                        found_empty = Some(i + j);
                        break;
                    }
                    Slot::Active { hash: h, key: k } => {
                        if *h == hash && *k == key {
                            return false;
                        }
                    }
                    Slot::Dummy => free_slot = Some(i + j),
                }
            }
            if let Some(index) = found_empty {
                break index;
            }
            i = next_index(i, &mut perturb, mask);
        };
        if let Some(index) = free_slot {
            self.used += 1;
            self.table[index] = Slot::Active { hash, key };
            return true;
        }
        self.fill += 1;
        self.used += 1;
        self.table[empty] = Slot::Active { hash, key };
        if self.fill * 5 >= mask * 3 {
            let min_used = if self.used > 50_000 {
                self.used * 2
            } else {
                self.used * 4
            };
            self.resize(min_used);
        }
        true
    }

    /// `set.discard` (`set_discard_entry`). Renvoie `true` si l'élément était là.
    pub(crate) fn discard(&mut self, key: &T) -> bool {
        match self.find(key, key.py_hash()) {
            Some(index) => {
                self.table[index] = Slot::Dummy;
                self.used -= 1;
                true
            }
            None => false,
        }
    }

    /// `set.update(other)` quand `other` est un `set` (`set_merge`).
    pub(crate) fn update(&mut self, other: &PySet<T>) {
        if other.used == 0 {
            return;
        }
        // Un seul agrandissement au début, comme en C.
        if (self.fill + other.used) * 5 >= self.mask() * 3 {
            self.resize((self.used + other.used) * 2);
        }
        // Table vide de même taille, sans fantôme de l'autre côté : copie case
        // par case.
        if self.fill == 0 && self.table.len() == other.table.len() && other.fill == other.used {
            self.table = other.table.clone();
            self.fill = other.fill;
            self.used = other.used;
            return;
        }
        if self.fill == 0 {
            self.fill = other.used;
            self.used = other.used;
            for slot in &other.table {
                if let Slot::Active { hash, key } = slot {
                    Self::insert_clean(&mut self.table, *hash, key.clone());
                }
            }
            return;
        }
        for slot in &other.table {
            if let Slot::Active { hash, key } = slot {
                self.add_with_hash(key.clone(), *hash);
            }
        }
    }

    /// `set.copy()` ou `set(other_set)` (`make_new_set` puis `set_merge`).
    pub(crate) fn copy(&self) -> PySet<T> {
        let mut set = PySet::new();
        set.update(self);
        set
    }

    /// `self - other` entre deux `set` (`set_difference`).
    pub(crate) fn difference(&self, other: &PySet<T>) -> PySet<T> {
        if (self.used >> 2) > other.used {
            // Copie puis retrait des éléments communs
            // (`set_copy_and_difference`).
            let mut result = self.copy();
            // L'ordre des retraits ne change pas la table : seules les cases
            // retirées comptent. Le raccourci de C par l'intersection n'a donc
            // pas d'effet visible.
            for slot in &other.table {
                if let Slot::Active { hash, key } = slot {
                    if let Some(index) = result.find(key, *hash) {
                        result.table[index] = Slot::Dummy;
                        result.used -= 1;
                    }
                }
            }
            if result.fill - result.used > result.mask() / 4 {
                let min_used = if result.used > 50_000 {
                    result.used * 2
                } else {
                    result.used * 4
                };
                result.resize(min_used);
            }
            return result;
        }
        let mut result = PySet::new();
        for slot in &self.table {
            if let Slot::Active { hash, key } = slot {
                if other.find(key, *hash).is_none() {
                    result.add_with_hash(key.clone(), *hash);
                }
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Valeurs relevées avec CPython 3.12 (`PYTHONHASHSEED=0` pour les textes).
    #[test]
    fn int_hash_matches_python() {
        assert_eq!(hash_int(0), 0);
        assert_eq!(hash_int(5), 5);
        assert_eq!(hash_int(-1), -2);
        assert_eq!(hash_int(-5), -5);
        assert_eq!(hash_int((1 << 61) - 1), 0);
        assert_eq!(hash_int(1 << 61), 1);
    }

    #[test]
    fn empty_tuple_hash_matches_python() {
        // hash(()) en Python 3.8 et après.
        assert_eq!(hash_tuple(&[]), 5_740_354_900_026_072_187);
    }

    #[test]
    fn small_ints_iterate_by_value() {
        let set = PySet::from_ordered([3i64, 1, 2]);
        let order: Vec<i64> = set.iter().copied().collect();
        assert_eq!(order, vec![1, 2, 3]);
    }

    #[test]
    fn colliding_ints_keep_insertion_order() {
        // 1 et 9 tombent sur la même case d'une table de 8.
        let a: Vec<i64> = PySet::from_ordered([1i64, 9]).iter().copied().collect();
        let b: Vec<i64> = PySet::from_ordered([9i64, 1]).iter().copied().collect();
        assert_eq!(a, vec![1, 9]);
        assert_eq!(b, vec![9, 1]);
    }

    #[test]
    fn discard_then_add_reuses_dummy_slot() {
        let mut set = PySet::from_ordered([1i64, 9]);
        assert!(set.discard(&1));
        assert!(!set.contains(&1));
        assert!(set.contains(&9));
        set.add(17);
        let order: Vec<i64> = set.iter().copied().collect();
        assert_eq!(order, vec![17, 9]);
        assert_eq!(set.used, 2);
    }

    /// Indice de test : même hachage et même égalité que `Hint` dans
    /// `scripts/pyset_vectors.py`.
    #[derive(Debug, Clone, PartialEq)]
    struct TestHint {
        receiving: i64,
        finding: i64,
        location: i64,
        item: i64,
        found: bool,
        entrance: String,
        flags: i64,
        status: i64,
    }

    impl PyHash for TestHint {
        fn py_hash(&self) -> i64 {
            hash_tuple(&[
                hash_int(self.receiving),
                hash_int(self.finding),
                hash_int(self.location),
                hash_int(self.item),
                hash_str(&self.entrance),
            ])
        }
    }

    fn test_hint(value: &serde_json::Value) -> TestHint {
        let field = |i: usize| value[i].as_i64().expect("entier");
        TestHint {
            receiving: field(0),
            finding: field(1),
            location: field(2),
            item: field(3),
            found: value[4].as_bool().expect("booléen"),
            entrance: value[5].as_str().expect("texte").to_string(),
            flags: field(6),
            status: field(7),
        }
    }

    /// Rejoue les suites d'opérations notées par CPython 3.12
    /// (`tests/data/scripts/pyset_vectors.py`) et compare l'ordre de parcours
    /// après chaque opération.
    #[test]
    fn matches_cpython_vectors() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../tests/data/pyset-vectors.json"))
                .expect("pyset-vectors.json valide");
        let cases = data["cases"].as_array().expect("liste de cas");
        assert!(!cases.is_empty());
        for (case_number, case) in cases.iter().enumerate() {
            let elements: Vec<TestHint> = case["elements"]
                .as_array()
                .expect("éléments")
                .iter()
                .map(test_hint)
                .collect();
            for (element, hash) in elements
                .iter()
                .zip(case["hashes"].as_array().expect("hachages"))
            {
                assert_eq!(
                    element.py_hash(),
                    hash.as_i64().expect("entier"),
                    "hachage de {element:?}"
                );
            }
            let pick =
                |v: &serde_json::Value| elements[v.as_u64().expect("numéro") as usize].clone();
            let list = |v: &serde_json::Value| -> Vec<TestHint> {
                v.as_array().expect("liste").iter().map(pick).collect()
            };
            let mut set: PySet<TestHint> = PySet::new();
            for (step, op) in case["ops"]
                .as_array()
                .expect("opérations")
                .iter()
                .enumerate()
            {
                match op["op"].as_str().expect("nom") {
                    "add" => {
                        set.add(pick(&op["h"]));
                    }
                    "discard" => {
                        set.discard(&pick(&op["h"]));
                    }
                    "rebuild" => set = PySet::from_ordered(set.iter().cloned()),
                    "copy" => set = set.copy(),
                    "update" => set.update(&PySet::from_ordered(list(&op["items"]))),
                    "difference" => set = set.difference(&PySet::from_ordered(list(&op["items"]))),
                    "from_list" => set = PySet::from_ordered(list(&op["items"])),
                    other => panic!("opération inconnue {other}"),
                }
                let expected = list(&op["order"]);
                let actual: Vec<TestHint> = set.iter().cloned().collect();
                assert_eq!(
                    actual, expected,
                    "cas {case_number}, étape {step} ({})",
                    op["op"]
                );
                assert_eq!(set.used, expected.len());
            }
        }
    }

    #[test]
    fn grows_and_keeps_every_element() {
        let set = PySet::from_ordered(0i64..200);
        assert_eq!(set.used, 200);
        assert!((0..200).all(|v| set.contains(&v)));
    }
}
