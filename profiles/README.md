# Profils d'installation

Un fichier TOML par jeu, nommé d'après `game.id` (exemple : `a-short-hike.toml`).
Le format est versionné par le champ `format_version`.
Exemple commenté : `docs/SPEC.md`, section 5 ; format détaillé : `docs/adr/0015-profils-installation-lancement.md`.

## Règles

- Chaque profil est **validé par la CI** (`cargo test`) : champ inconnu, sha256 absent, domaine hors liste blanche, chemin hors de `{game_dir}` / `{worlds_dir}`, etc. sont refusés.
- Tout nouveau fichier doit aussi être ajouté à la liste `EMBEDDED` de `crates/launcher-core/src/catalog.rs` ; un test échoue sinon.
- Jamais de jeu, de ROM ni d'asset : seulement des mods, des chargeurs de mods et des apworlds.
- `game.seconds_per_location` (facultatif, 1 à 3600) : temps moyen d'un lieu, pour la durée estimée d'une partie (ADR 0030) ; 120 s sans lui.
- `[art]` (facultatif, ADR 0051) : les images choisies par le créateur du world, téléchargées à l'installation du jeu puis affichées sans réseau. `cover` (jaquette en portrait, environ 2:3) et `hero` (image large, 16:9), chacune `{ url = "https://…", sha256 = "…" }`, en JPEG ou PNG, 8 Mio au plus, sur `github.com`, `objects.githubusercontent.com`, `raw.githubusercontent.com` ou Thunderstore. Seulement des adresses chez le créateur : aucune image dans le dépôt. Sans `[art]`, le launcher prend l'image que Steam garde sur le PC du joueur, sinon il dessine une jaquette. `game.igdb_id` est encore lu mais ignoré.
- Un mod sans licence est accepté (ADR 0021, étendu par l'ADR 0050) : `license = "unlicensed"`, téléchargé chez son auteur sur le PC du joueur, jamais redistribué ; retiré si l'auteur le demande. Une autorisation écrite reste possible (`license = "permission"` et `permission_url`).
- `game.status = "untested"` : profil jamais essayé avec le vrai jeu (ADR 0049) ; il passe en `available` après un essai réel réussi.
