# Profils d'installation

Un fichier TOML par jeu, nommé d'après `game.id` (exemple : `a-short-hike.toml`).
Le format est versionné par le champ `format_version`.
Exemple commenté : `docs/SPEC.md`, section 5 ; format détaillé : `docs/adr/0015-profils-installation-lancement.md`.

## Règles

- Chaque profil est **validé par la CI** (`cargo test`) : champ inconnu, sha256 absent, domaine hors liste blanche, chemin hors de `{game_dir}` / `{worlds_dir}`, etc. sont refusés.
- Tout nouveau fichier doit aussi être ajouté à la liste `EMBEDDED` de `crates/launcher-core/src/catalog.rs` ; un test échoue sinon.
- Jamais de jeu, de ROM ni d'asset : seulement des mods, des chargeurs de mods et des apworlds.
- `game.seconds_per_location` (facultatif, 1 à 3600) : temps moyen d'un lieu, pour la durée estimée d'une partie (ADR 0030) ; 120 s sans lui.
- Un mod sans licence demande l'autorisation écrite de son auteur (`license = "permission"` et `permission_url`).
