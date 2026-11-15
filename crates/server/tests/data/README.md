# Données de test du serveur

## `roominfo-seed-mit.json`

- Trame brute `RoomInfo` envoyée par le serveur officiel d'Archipelago 0.6.8 (`MultiServer.py --disable_save`) pour la seed `crates/multidata/tests/data/seed-mit.archipelago`.
- Capturée le 15 novembre 2026 avec un petit client Python `websockets` (script `roominfo.py`, hors dépôt).
- `tests/socle.rs` vérifie que Raftup envoie les mêmes valeurs. Seule l'heure (`time`) change d'un lancement à l'autre ; le test reprend celle de la capture.

## `connexion-seed-mit.json`

- 16 scénarios de connexion rejoués sur le serveur officiel 0.6.8 avec la même seed, le 15 novembre 2026 (script `connect_vectors.py`, hors dépôt).
- Pour chaque étape : le client qui envoie, les paquets envoyés (`envoi`) et les trames brutes reçues par chaque client (`recu`). `<fermé>` signale une connexion fermée par le serveur.
- Couvre : connexion acceptée, inventaire de départ, `slot_data` refusé, `items_handling`, outil `TextOnly`, chaque refus de `ConnectionRefused`, `Connect` incomplet, reconnexion, changement de slot et groupe (le serveur officiel ferme la connexion), `GetDataPackage`.
- `tests/connexion.rs` rejoue les scénarios sur Raftup et compare les trames, sans tenir compte de l'ordre des clés ni de l'ordre des erreurs de `ConnectionRefused` (un ensemble en Python).

## `partie-seed-mit.json`

- Une partie de 20 étapes rejouée sur un serveur officiel 0.6.8 neuf, avec la même seed, le 15 novembre 2026 (script `game_vectors.py`, hors dépôt). Même format que `connexion-seed-mit.json`.
- Cinq connexions : Alice, Bob, Chloé, un outil `TextOnly` sur le slot de Bob, puis une seconde connexion de Bob.
- Couvre : lieux visités (`LocationChecks`, y compris en double ou inconnus), messages `ItemSend`, objets distribués (`ReceivedItems`, avec le groupe `Lien`), `RoomUpdate`, refus pour un outil `TextOnly`, `Sync`, `ConnectUpdate` (`items_handling` et tags), reconnexion.
- `tests/connexion.rs` trie aussi `checked_locations` de `RoomUpdate`, un ensemble en Python.

## `objectif-seed-mit.json`

- Même méthode que `partie-seed-mit.json` (script `game_vectors.py objectif`, serveur officiel neuf), le 15 novembre 2026.
- Couvre : `StatusUpdate` (refus pour un outil `TextOnly`, statut « prêt »), objectif atteint (`Goal`), `collect` et `release` automatiques (options `auto` de la seed), `collect` du groupe `Lien` quand ses deux membres ont fini, objectif définitif, message de fin d'équipe.
