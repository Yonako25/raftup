//! Identifiants des éléments du monde : un index `u32` dans le tableau correspondant.
//!
//! Chaque sorte d'élément a son propre type (`RegionId`, `LocationId`…) : le compilateur
//! refuse ainsi de passer un numéro de lieu là où on attend un numéro de région.
//! Le champ est `pub(crate)` : seule la crate fabrique des identifiants, en ajoutant un
//! élément au [`MultiWorld`](crate::MultiWorld).

/// Écrit un type d'identifiant. Une macro (`macro_rules!`) évite de recopier six fois
/// exactement le même code : elle prend un nom et une phrase de documentation.
macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        // `Copy` : un identifiant n'est qu'un nombre, on le copie au lieu de l'emprunter.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub(crate) u32);

        impl $name {
            /// Position dans le tableau du [`MultiWorld`](crate::MultiWorld).
            pub fn index(self) -> usize {
                self.0 as usize
            }

            /// Valeur brute (pour les messages et les tests).
            pub fn raw(self) -> u32 {
                self.0
            }
        }
    };
}

id_type!(SlotId, "Un joueur (slot) du multiworld.");
id_type!(RegionId, "Une région d'un monde.");
id_type!(
    EntranceId,
    "Une entrée : passage orienté d'une région vers une autre."
);
id_type!(LocationId, "Un lieu, où l'on trouve un objet.");
id_type!(
    ItemKindId,
    "Une sorte d'objet d'un joueur (« Sword » du joueur 1)."
);
id_type!(
    ItemGroupId,
    "Un groupe nommé de sortes d'objets d'un joueur."
);
