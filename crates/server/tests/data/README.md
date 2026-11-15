# Données de test du serveur

## `roominfo-seed-mit.json`

- Trame brute `RoomInfo` envoyée par le serveur officiel d'Archipelago 0.6.8 (`MultiServer.py --disable_save`) pour la seed `crates/multidata/tests/data/seed-mit.archipelago`.
- Capturée le 15 novembre 2026 avec un petit client Python `websockets` (script `roominfo.py`, hors dépôt).
- `tests/socle.rs` vérifie que Raftup envoie les mêmes valeurs. Seule l'heure (`time`) change d'un lancement à l'autre ; le test reprend celle de la capture.
