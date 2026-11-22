# ADR 0051 — Images choisies par le créateur, installées avec le world

- **Statut** : accepté le 22 novembre 2026 par l'utilisatrice
- Remplace l'ADR 0047 (illustrations servies par le hub depuis IGDB) ; complète l'ADR 0015 (profils d'installation).

## Contexte

Avec l'ADR 0047, les jaquettes venaient d'IGDB par le hub. L'utilisatrice n'en veut plus. Elle veut :

- des images internes, qui ne passent par aucune API ;
- des images déjà chargées quand on ouvre un onglet ;
- un affichage qui ne dépend pas d'un service en ligne.

Ranger les vraies jaquettes dans le dépôt est exclu. Elles appartiennent aux éditeurs, et Raftup ne distribue jamais d'asset de jeu (règle du projet). L'utilisatrice a proposé l'idée retenue : à l'installation d'un world, installer aussi les images choisies par son créateur.

## Décision

1. **Section `[art]` du profil** (facultative) :
   - `cover` (jaquette en portrait) et `hero` (image large), chacune `{ url, sha256 }` ;
   - JPEG ou PNG, 8 Mio au plus ;
   - domaines permis : ceux des mods, plus `raw.githubusercontent.com`.

   Le dépôt ne contient que l'adresse chez le créateur, comme pour un apworld. La section est facultative, donc `format_version` reste à 1.
2. **À l'installation**, `launcher_core::art::install_art` télécharge les images par le téléchargeur habituel, avec le hash vérifié et le cache par sha256. Il les range dans `<données>/art/<jeu>/cover.jpg` (ou `.png`). Une image qui ne vient pas ne fait jamais échouer l'installation : un avertissement va au journal. Un jeu déjà à jour récupère ses images à la réinstallation suivante.
3. **À l'affichage**, sans réseau, les images sont cherchées dans cet ordre :
   1. l'image du créateur installée ;
   2. l'image que Steam garde déjà sur le PC du joueur (`<Steam>/appcache/librarycache/<appid>/library_600x900.jpg` et `library_hero.jpg`), qui vient de la copie du joueur et n'est pas redistribuée ;
   3. la jaquette dessinée par Raftup (aplat de la teinte du jeu et titre).
4. **Préchargement** : au démarrage, le launcher demande d'un coup les images de tous les jeux du catalogue. Un thread à part les lit et les décode.
5. **IGDB quitte le launcher** :
   - le client `ArtClient` et son cache disque sont retirés ;
   - la ligne d'attribution « Illustrations : IGDB.com » est retirée ;
   - les `igdb_id` des profils sont retirés. Le champ reste lu mais est ignoré, pour que les anciens profils restent valides.

## Conséquences

- Plus aucun appel réseau ni aucune API pour afficher une image. Tout est sur le disque dès le démarrage.
- La plupart des créateurs d'apworlds ne fournissent pas encore d'images : aucun profil du catalogue n'a de section `[art]` pour l'instant. Les jeux installés via Steam ont quand même leur jaquette grâce au cache de Steam. Les autres gardent la jaquette dessinée.
- Côté hub, les routes `/v1/art/…` et la clé IGDB restent en service mais ne servent plus au launcher. Leur retrait est une étape à part.
- Les mondes natifs (`world.toml`) n'ont pas encore de section `[art]`. Ce sera une étape à part, qui touche la spécification `spec/monde-natif/`.
- Les images restent dans `art/<jeu>/` après une désinstallation : quelques centaines de Ko, relues si le jeu revient.
