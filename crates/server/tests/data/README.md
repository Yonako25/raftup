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
