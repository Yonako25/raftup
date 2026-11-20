# 0003 — Licence AGPL-3.0 pour `raftup-hub`

- Statut : accepté
- Date : 2026-11-11

## Contexte

Raftup est en double licence MIT / Apache-2.0. Ces licences autorisent n'importe qui à reprendre le code d'un service en ligne, à le modifier et à l'héberger sans jamais publier ses modifications.
Pour le hub (ADR 0002), Raftup suit le modèle de Modrinth : le serveur est sous AGPL pour empêcher une reprise fermée du service.
La décision doit être prise avant les premières contributions extérieures : changer de licence ensuite demande l'accord de tous les contributeurs.

## Décision

- `raftup-hub` est sous **AGPL-3.0-or-later**, et uniquement lui.
- Tout le reste reste en **MIT / Apache-2.0** : `ap-protocol`, `client-core`, `multidata`, `server`, `py-bridge`, `launcher-core`, `launcher-ui`.
- Le champ `license` du `Cargo.toml` de chaque crate indique sa licence.
- Le fichier `LICENSE-AGPL` est ajouté à la création de la crate du hub.
  C'est le texte exact de la FSF, récupéré le 2026-11-20 sur <https://www.gnu.org/licenses/agpl-3.0.txt> (SHA-256 `0d96a4ff68ad6d4b6f1f30f713b18d5184912ba8dd389f86aa7710db079abcb0`, fins de ligne LF). Le test `crates/hub/tests/licence.rs` vérifie cette empreinte.
- Le hub ne copie pas de code depuis les crates MIT/Apache : il en dépend comme bibliothèques (compatible : du code MIT/Apache peut être utilisé par du code AGPL).
- Les crates MIT/Apache ne dépendent jamais du hub.

## Conséquences

- Une instance modifiée du hub, même seulement hébergée en ligne, doit publier son code source.
- Le launcher et le serveur de partie restent réutilisables librement, y compris par d'autres projets compatibles avec Archipelago.
- Les contributeurs doivent savoir sous quelle licence tombe leur code : à préciser dans `CONTRIBUTING.md`.
