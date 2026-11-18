# ADR 0026 — Paquets Python installés depuis une archive de source

- **Statut** : accepté le 18 novembre 2026 (étend l'ADR 0017)

## Contexte

`zilliandomizer` (monde Zillion) n'existe ni sur PyPI ni en wheel. Archipelago l'épingle sur un commit GitHub.

## Décision proposée

- Section optionnelle `[[source_packages]]` dans `generator-pins.toml` : archive GitHub d'un commit précis, sha256, HTTPS, liste blanche de domaines, seul `src/<nom>` est copié dans `site-packages`.
- Même règle que les wheels : téléchargé sur le PC du joueur au premier usage, jamais redistribué ni lié à une crate Raftup.
- Licences à connaître : `zilliandomizer` en AGPL-3.0 avec exception de liaison, `pyevermizer` en LGPL-3.0.

## Conséquences

- Tout runtime déjà installé passe en « à réinstaller » (les pins ont changé).
- Une archive GitHub d'un commit n'a pas de hash garanti à vie : même risque que l'archive d'Archipelago déjà épinglée.
