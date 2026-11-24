# Inventaire de conversion des mondes d'Archipelago 0.6.8

Mesuré le **23 novembre 2026** (puis remesuré le même jour après le lot 0 et après le lot 1, et le **24 novembre 2026** après le lot 2) sur les **81 mondes intégrés** d'Archipelago 0.6.8 (dossier `worlds/` du générateur épinglé, sans le monde caché « Archipelago » ni les apworlds ajoutées), avec les **options par défaut**, un joueur, seed 1. Rien n'a été publié ; les `world.toml` produits sont restés hors du dépôt.

Chaîne essayée pour chaque jeu (test `launcher-core/tests/inventaire_real.rs`) :

1. **export** du worker Python (ADR 0028), arrêté avant le placement ;
2. **import** de l'export et contrôle croisé des règles contre le vrai Archipelago (moteur « export ») ;
3. **conversion** en monde natif (`generate::convert`, ADR 0042), relue par `worlds_native::parse_world` ;
4. **seed complète par le moteur Rust seul** (sans Python), relue par `multidata` ;
5. **aller-retour** : le monde converti, renommé « … Natif », réexporté en apworld, réexporté par le worker et repassé au contrôle croisé ;
6. **seconde seed** (depuis le lot 1, ADR 0054 § 5) : export et conversion refaits avec la seed 2 ; `hasard` si le texte change, `pose au hasard` si ce sont les objets posés d'avance qui changent.

## Totaux

| Résultat | Jeux |
| --- | --- |
| Converti sans point à vérifier | **1** (ChecksFinder) |
| Converti avec des points « À vérifier » | **35** (seed Rust et aller-retour réussis pour les 36 convertis) ; 14 au départ, 16 après le lot 0, 29 après le lot 1 |
| Échec | **45** |

- Les 36 convertis : A Short Hike, APQuest, Adventure, Bumper Stickers, Castlevania - Circle of the Moon, Castlevania 64, Celeste (Open World), Celeste 64, ChecksFinder, Choo-Choo Charles, Civilization VI, DOOM 1993, DOOM II, Dark Souls III, EarthBound, Faxanadu, Heretic, Hylics 2, Inscryption, Landstalker, Mario & Luigi Superstar Saga, Mega Man 2, Mega Man 3, Meritous, Muse Dash, Noita, Risk of Rain 2, Saving Princess, Sonic Adventure 2 Battle, Subnautica, Super Mario 64, The Wind Waker, Timespinner, Undertale, VVVVVV, Wargroove.
- Sur les 49 jeux PC (sans ROM ni ISO), **25 convertis** (13 avant le lot 1, 21 avant le lot 2) ; sur les 32 jeux à ROM ou ISO, 11.
- Lot 1 : 13 des 14 jeux visés se convertissent ; Aquaria bute sur une cause qui était cachée (pool plus grand que les lieux). Celeste 64 et Celeste (Open World) demandaient en plus une correction du convertisseur (objet cité par une règle mais jamais créé).
- Lot 2 (sondeur élargi, puis sommes de compteurs, ADR 0055) : 7 jeux de plus (Castlevania 64, ChecksFinder, EarthBound, Subnautica, The Wind Waker, Timespinner, Wargroove) ; les 29 convertis avant le restent. Les règles opaques baissent aussi chez des jeux qui échouent pour une autre cause (Hollow Knight : 326 → 1 ; Stardew Valley : 545 → 388 ; Subnautica : 135 → 0). Overcooked! 2, Yu-Gi-Oh! 2006, shapez et Super Mario Land 2 n'ont plus de règle opaque, mais butent sur une cause que la règle opaque cachait (voir les causes).
- **Hasard** : 25 convertis changent d'une seed à l'autre (20 avant le lot 2) ; pour 15 d'entre eux, c'est le pool (remplissage, pièges ou objets présents tirés au hasard) ; pour Mega Man 3, une option par défaut tirée au hasard ; pour Landstalker, une règle (`has_all(Lantern, "Axe Magic")` devient `has_all("Axe Magic")`), à regarder. Pour **6** (Dark Souls III, EarthBound, Faxanadu, Noita, The Wind Waker, Timespinner), ce sont les **objets posés d'avance** qui changent : l'instantané fige un tirage, à réécrire à la main avant toute publication.
- Aucun jeu de la liste n'est sur une console Nintendo actuelle (Switch) ; 21 demandent une ROM ou un ISO d'une ancienne console Nintendo.
- Durée du lot : 7 minutes (426 s, puis 442 s) pour les 81 jeux ; 13 minutes (774 s) avec la seconde seed ; 11 minutes (658 s) après le lot 2, dont 99 s pour Kingdom Hearts et 74 s pour Starcraft 2.

### Ce que « converti » veut dire ici

Un monde converti n'est pas encore jouable tel quel :

- c'est un **instantané** des options par défaut (ADR 0042, § 1) : tant que l'auteur n'a pas écrit les `when`, le YAML ne peut régler aucune option du jeu ;
- **`slot_data` absent** : l'inventaire convertit sans seed d'Archipelago. 11 des 14 premiers convertis ont un `fill_slot_data` que leur mod lit ; il faut le reprendre (option `--seed` de `raftup-monde`, en valeurs littérales) puis l'écrire à la main s'il dépend des options ;
- **fichier de sortie** : 5 convertis écrivent un fichier à la génération (`generate_output`) que le moteur Rust ne produit pas : Adventure, Mega Man 2 et Mega Man 3 (patch de ROM), Civilization VI (mod propre à la seed), VVVVVV (fichier en solo seulement). Sans lui, le jeu ne se lance pas (sauf VVVVVV en multi).

## Causes d'échec, par fréquence

Le convertisseur s'arrête à la **première** cause ; l'export, lui, compte toutes les règles opaques et signale le `collect` personnalisé. Chiffres après le lot 2.

| Cause | 1ʳᵉ cause | En tout | Seule cause | Ce qu'il faudrait faire |
| --- | --- | --- | --- | --- |
| `collect` / `remove` redéfinis (compteurs maison, objets qui valent N) | 21 | 21 | 11 | Format : objets qui **ajoutent à un compteur** (`gives = { "Argent" = 50 }`) et règles sur ce compteur ; les cas plus libres restent au portage à la main. À valider (ADR) |
| Règle opaque (sondeur) | 9 | **20** | 8 | Sondeur : voir le détail plus bas (36 jeux avant le lot 2) |
| ROM demandée dès la génération | 8 | 8 | 8 | En mode export, ne pas exiger la ROM (elle ne sert qu'à `generate_output`) ; mais le patch reste écrit par Python : décision 3 |
| Pool plus grand que les lieux | 1 | 1 | 1 | Aquaria (217 objets pour 195 lieux) : Archipelago laisse le surplus non placé, avec un avertissement. Format : dire quel surplus laisser tomber, ou garder Aquaria au repli Python |
| Lieu sans numéro et sans objet d'événement | 1 | 1 | 0 | Pokemon Red and Blue (« Buy Fire Stone ») : à regarder avec ses règles opaques |
| Règle convertie de plus de 16 Kio | 1 | 1 | 1 | Overcooked! 2 : le score d'un niveau est une somme de **conditions** à poids flottants (`has(x)` vaut 0,5…), pas de compteurs ; le sondeur la traduit exactement, mais en « ou » de 250 « et ». Format : somme de conditions, ou limite relevée |
| Groupe qui contient un objet d'événement | 1 | 1 | 1 | Yu-Gi-Oh! 2006 (« Campaign Boss Beaten » : « Tier 1 Beaten »…) : convertisseur, cause cachée jusqu'ici par la règle opaque |
| Événement qui est aussi un lieu (nom en double) | 1 | 1 | 1 | Super Mario Land 2 (« Tree Zone 5 - Boss ») : convertisseur, cause cachée |
| Entrée vers une région inconnue | 1 | 1 | 1 | shapez (« Achievements needing a MAM ») : export ou convertisseur, cause cachée |
| `has_from_list` hors groupe déclaré | 0 | — | — | **Fait au lot 2** (format version 4, ADR 0055) : `count_from_list(…) >= n` (ChecksFinder) |
| Plus de 256 combinaisons (`au moins 6 parmi 23`) | 0 | — | — | **Fait au lot 2** : `count_unique(…) >= n` (Yu-Gi-Oh! 2006) |
| Options par défaut refusées par le monde | 1 | 1 | 1 | Final Fantasy exige des réglages faits sur le site de FFR : à essayer avec un vrai YAML |
| Objet verrouillé dans un lieu à numéro | 0 | — | — | **Fait au lot 1** (format version 3, ADR 0054) : `item = "…"` sur le lieu ; 24 jeux l'avaient en 1ʳᵉ cause |
| Objet d'événement qui a aussi un numéro | 0 | — | — | **Fait au lot 1** : en version 3, l'événement nomme l'objet à numéro |
| Objet cité par une règle, jamais créé | 0 | — | — | **Corrigé au lot 1** : écrit `false`, comme Archipelago le compte (Celeste 64, Celeste (Open World)) |
| `has_all` vide | 0 | — | — | **Corrigé au lot 0** (Mega Man 2 et 3) |

Combinaisons des jeux à plusieurs causes : `collect` + règles opaques (10), règles opaques + lieu sans numéro (Pokemon Red and Blue).

### Règles opaques : d'où elles viennent

Raisons relevées avant le lot 2 (échantillon de 8 raisons distinctes par jeu, 36 jeux) :

| Raison | Jeux | Piste |
| --- | --- | --- |
| Attribut de l'inventaire inconnu, propre au monde (`state._hk_notches`, `state.smbm`, `state._tww_…`, `state.raft_…`, `state.stale`) | ~15 | Ce sont des caches posés par un `collect` maison : se règle avec la cause `collect`, ou à la main |
| `state.multiworld` (lecture des options par la règle) | 6 | Sondeur : donner un `multiworld` en lecture seule (options, noms) |
| `state.prog_items` lu directement | 5 | Sondeur : vue en lecture seule de `prog_items` qui pose les questions `count` |
| Calcul sur un compteur (`count * 1.5`, somme de `count`) | 8 | `logic` : règle « somme pondérée ≥ seuil », puis le sondeur la reconnaît |
| Plus de 4 096 chemins ou de 64 questions | 9 | Relever les plafonds par monde, ou mémoriser les sous-règles déjà sondées |
| Résultat qui n'est pas un booléen (`EntranceRando`, `EnterSandman`…) | 4 | Lire la vérité de l'objet rendu (`bool(x)`) comme Archipelago |
| `count_from_list_unique`, `update_reachable_regions` | 2 | Ajouter ces deux questions au sondeur |
| Règle non monotone, résultat instable (Stardew) | 2 | Rien à faire dans le sondeur : portage à la main |

**Fait au lot 2** : `state.multiworld` et `prog_items` en lecture seule, `reachable_regions` (pour `Region.can_reach`), méthodes de LogicMixin, résultat non booléen, `count_from_list_unique`, calculs sur un compteur, sommes de compteurs en une seule question (ADR 0055), plafonds réglables (`RAFTUP_INVENTAIRE_PLAFONDS`). **Reste**, sur les 20 jeux qui ont encore des règles opaques :

| Raison | Jeux | Piste |
| --- | --- | --- |
| Plus de 4 096 chemins (« et » de nombreux « ou », sommes de conditions) | Kingdom Hearts, Kingdom Hearts 2, Kirby's Dream Land 3, Old School Runescape, Shivers, Starcraft 2, Terraria, Lingo | Mémoriser les sous-règles déjà sondées, ou une règle « somme de conditions » |
| Plus de 64 questions | Pokemon Red and Blue | Plafond relevé pour ce monde |
| Règle non monotone | Lingo, Kingdom Hearts (une règle) | Portage à la main |
| Copie de `prog_items` (`prog_items[...].copy`) | Zillion | Copie en lecture seule dans le sondeur |
| Attributs propres au monde, posés par un `collect` maison | les 10 jeux `collect` + opaques | Lot 3 (compteurs) |

### Points « À vérifier » des 29 convertis (après le lot 1)

| Code | Point | Jeux |
| --- | --- | --- |
| `classif` | objet ni dans le pool, ni au départ, ni posé, écrit `filler` | 24 |
| `pool` | fin du pool écrite en `[filler]` : le monde tire-t-il des pièges ? | 23 |
| `absents` | lieux absents avec ces options, gardés avec `when = 'false'` | 20 |
| `pose` | objets posés d'avance : dépendent-ils des options ou du hasard ? (`pose au hasard` dans le tableau quand la seconde seed le prouve) | 13 |
| `option` | option d'un type que le format ne connaît pas (listes, dictionnaires, poids) | 12 jeux, 84 options |
| `mixte` | exemplaires d'un même objet avec des classifications différentes : l'union de leurs drapeaux est gardée | 7 jeux, 30 objets |
| `item_rule` | règles d'objet non converties | 6 |
| `early` | objets à placer tôt ignorés | 3 |
| `jamais` | objets cités par les règles mais jamais créés, écrits `false` | 2 |
| `drapeaux` | drapeaux de classification inconnus | 2 |

Avant le lot 0, `start_inventory_from_pool` (option déclarée par chaque monde mais appliquée par Archipelago avant l'export) était signalée à tort dans 5 jeux, et le convertisseur gardait la classification du **premier** exemplaire d'un objet mixte, ce qui pouvait faire d'une progression un simple remplissage.

À noter : pour Muse Dash, Sonic Adventure 2 Battle et Saving Princess, l'**import** (moteur « export » de l'ADR 0028) refuse toujours l'objet à classifications mixtes, alors que la conversion garde l'union des drapeaux. Laisser l'import faire pareil changerait le comportement décrit par l'ADR 0028 : à proposer à part.

## Tableau par jeu

Support : « PC » quand le jeu se joue sur sa version PC sans ROM ni ISO. « licence restrictive » : licence « Modified MIT » qui interdit la relicence et la vente (décision 2). « fichier de sortie » : le monde écrit un fichier que le moteur Rust ne produit pas.

| Jeu | Support | Résultat | Points à vérifier (codes) ou causes d'échec | Seed Rust | Aller-retour |
| --- | --- | --- | --- | --- | --- |
| A Hat in Time | PC | Échec | `collect` personnalisé | — | — |
| A Link to the Past | SNES | Échec | ROM demandée à la génération | — | — |
| A Short Hike | PC | Converti (2 à vérifier) | item_rule, pool | oui (131 lieux) | contrôle croisé OK |
| APQuest | PC | Converti (3 à vérifier) | absents, classif, pool | oui (6 lieux) | contrôle croisé OK |
| Adventure | Atari 2600 · fichier de sortie | Converti (3 à vérifier) | classif, item_rule, pool · hasard | oui (24 lieux) | contrôle croisé OK |
| Aquaria | PC | Échec | pool plus grand que les lieux (217 pour 195) | — | — |
| Blasphemous | PC | Échec | `collect` personnalisé, 63 règle(s) opaque(s) | — | — |
| Bomb Rush Cyberfunk | PC | Échec | `collect` personnalisé, 78 règle(s) opaque(s) | — | — |
| Bumper Stickers | PC | Converti (2 à vérifier) | classif, pool · hasard | oui (100 lieux) | contrôle croisé OK |
| Castlevania - Circle of the Moon | GBA | Converti (4 à vérifier) | absents, early, pool, pose · hasard | oui (124 lieux) | contrôle croisé OK |
| Castlevania 64 | N64 | Converti (5 à vérifier) | absents, classif, item_rule, option, pool · hasard | oui (213 lieux) | contrôle croisé OK |
| Celeste (Open World) | PC · licence restrictive | Converti (23 à vérifier) | absents, classif, jamais, mixte, option ×17, pool, pose | oui (233 lieux) | contrôle croisé OK |
| Celeste 64 | PC · licence restrictive | Converti (9 à vérifier) | absents, jamais, mixte, option ×4, pool, pose | oui (40 lieux) | contrôle croisé OK |
| ChecksFinder | PC | Converti (aucun point à vérifier) | — | oui (25 lieux) | contrôle croisé OK |
| Choo-Choo Charles | PC | Converti (1 à vérifier) | classif | oui (691 lieux) | contrôle croisé OK |
| Civilization VI | PC · fichier de sortie | Converti (7 à vérifier) | absents, classif, early, item_rule, option ×2, pool · hasard | oui (148 lieux) | contrôle croisé OK |
| DLCQuest | PC | Échec | `collect` personnalisé, 16 règle(s) opaque(s) | — | — |
| DOOM 1993 | PC | Converti (4 à vérifier) | absents, classif, pool, pose · hasard | oui (348 lieux) | contrôle croisé OK |
| DOOM II | PC | Converti (4 à vérifier) | absents, classif, pool, pose · hasard | oui (453 lieux) | contrôle croisé OK |
| Dark Souls III | PC | Converti (25 à vérifier) | absents, classif, early, item_rule, option ×19, pool, pose · pose au hasard, hasard | oui (1190 lieux) | contrôle croisé OK |
| EarthBound | SNES | Converti (6 à vérifier) | absents, classif, jamais, option, pool, pose · pose au hasard, hasard | oui (251 lieux) | contrôle croisé OK |
| Factorio | PC | Échec | `collect` personnalisé | — | — |
| Faxanadu | NES | Converti (4 à vérifier) | classif, item_rule, mixte, pose · pose au hasard, hasard | oui (110 lieux) | contrôle croisé OK |
| Final Fantasy | NES | Échec | options par défaut refusées (réglages FFR) | — | — |
| Final Fantasy Mystic Quest | SNES | Échec | `collect` personnalisé | — | — |
| Gauntlet Legends | N64 | Échec | `collect` personnalisé | — | — |
| Heretic | PC | Converti (4 à vérifier) | absents, classif, pool, pose · hasard | oui (502 lieux) | contrôle croisé OK |
| Hollow Knight | PC | Échec | `collect` personnalisé, 1 règle opaque | — | — |
| Hylics 2 | PC | Converti (3 à vérifier) | absents, classif, option | oui (133 lieux) | contrôle croisé OK |
| Inscryption | PC | Converti (2 à vérifier) | classif, pool | oui (100 lieux) | contrôle croisé OK |
| Jak and Daxter: The Precursor Legacy | PS2 (ISO, portage OpenGOAL) | Échec | `collect` personnalisé, 15 règle(s) opaque(s) | — | — |
| Kingdom Hearts | PC | Échec | 51 règle(s) opaque(s) | — | — |
| Kingdom Hearts 2 | PC | Échec | 15 règle(s) opaque(s) | — | — |
| Kirby's Dream Land 3 | SNES | Échec | 1 règle(s) opaque(s) | — | — |
| Landstalker - The Treasures of King Nole | Mega Drive | Converti (6 à vérifier) | absents, classif, item_rule, mixte ×2, pose · hasard | oui (291 lieux) | contrôle croisé OK |
| Lingo | PC | Échec | 4 règle(s) opaque(s) (non monotone, chemins) | — | — |
| Links Awakening DX | GBC | Échec | `collect` personnalisé, 97 règle(s) opaque(s) | — | — |
| Lufia II Ancient Cave | SNES | Échec | ROM demandée à la génération | — | — |
| Mario & Luigi Superstar Saga | GBA | Converti (5 à vérifier) | absents, classif, option, pool, pose · hasard | oui (556 lieux) | contrôle croisé OK |
| Mega Man 2 | NES · fichier de sortie | Converti (5 à vérifier) | absents, classif, option ×2, pool · hasard | oui (44 lieux) | contrôle croisé OK |
| Mega Man 3 | NES · fichier de sortie | Converti (4 à vérifier) | absents, option ×2, pool · hasard | oui (64 lieux) | contrôle croisé OK |
| MegaMan Battle Network 3 | GBA | Échec | ROM demandée à la génération | — | — |
| Meritous | PC | Converti (3 à vérifier) | classif, pool, pose · hasard | oui (104 lieux) | contrôle croisé OK |
| Muse Dash | PC | Converti (31 à vérifier) | absents, classif, drapeaux, mixte ×21, option ×7 · hasard | oui (90 lieux) | contrôle croisé OK |
| Noita | PC | Converti (6 à vérifier) | absents, classif, option ×2, pool, pose · pose au hasard, hasard | oui (109 lieux) | contrôle croisé OK |
| Ocarina of Time | N64 | Échec | ROM demandée à la génération | — | — |
| Old School Runescape | PC | Échec | 16 règle(s) opaque(s) | — | — |
| Overcooked! 2 | PC | Échec | règle convertie de plus de 16 Kio (somme de conditions) | — | — |
| Paint | PC | Échec | `collect` personnalisé, 131 règle(s) opaque(s) | — | — |
| Pokemon Emerald | GBA | Échec | `collect` personnalisé | — | — |
| Pokemon Red and Blue | GB | Échec | 139 règle(s) opaque(s), lieu sans numéro ni événement | — | — |
| Raft | PC | Échec | `collect` personnalisé, 142 règle(s) opaque(s) | — | — |
| Risk of Rain 2 | PC | Converti (3 à vérifier) | absents, classif, pool · hasard | oui (187 lieux) | contrôle croisé OK |
| SMZ3 | SNES | Échec | `collect` personnalisé, 357 règle(s) opaque(s) | — | — |
| Satisfactory | PC | Échec | `collect` personnalisé, 38 règle(s) opaque(s) | — | — |
| Saving Princess | PC | Converti (5 à vérifier) | classif, mixte ×3, pool · hasard | oui (36 lieux) | contrôle croisé OK |
| Secret of Evermore | SNES | Échec | ROM demandée à la génération | — | — |
| Shivers | PC | Échec | 1 règle(s) opaque(s) | — | — |
| Sonic Adventure 2 Battle | PC · licence restrictive | Converti (30 à vérifier) | absents, classif, mixte, option ×26, pool · hasard | oui (206 lieux) | contrôle croisé OK |
| Starcraft 2 | PC | Échec | 119 règle(s) opaque(s) | — | — |
| Stardew Valley | PC | Échec | `collect` personnalisé, 388 règle(s) opaque(s) | — | — |
| Subnautica | PC | Converti (6 à vérifier) | absents, classif, early, option, pool, pose · hasard | oui (131 lieux) | contrôle croisé OK |
| Super Mario 64 | N64 (ROM, portage sm64ex) | Converti (5 à vérifier) | absents, classif, drapeaux, option, pose | oui (149 lieux) | contrôle croisé OK |
| Super Mario Land 2 | GB | Échec | événement qui est aussi un lieu (« Tree Zone 5 - Boss ») | — | — |
| Super Mario World | SNES · licence restrictive | Échec | ROM demandée à la génération | — | — |
| Super Metroid | SNES | Échec | `collect` personnalisé, 301 règle(s) opaque(s) | — | — |
| TUNIC | PC | Échec | `collect` personnalisé, 14 règle(s) opaque(s) | — | — |
| Terraria | PC | Échec | 3 règle(s) opaque(s) | — | — |
| The Legend of Zelda | NES | Échec | ROM demandée à la génération | — | — |
| The Messenger | PC | Échec | `collect` personnalisé, 23 règle(s) opaque(s) | — | — |
| The Wind Waker | GameCube (ISO) | Converti (5 à vérifier) | absents, classif, option ×2, pose · pose au hasard, hasard | oui (108 lieux) | contrôle croisé OK |
| The Witness | PC | Échec | `collect` personnalisé, 75 règle(s) opaque(s) | — | — |
| Timespinner | PC | Converti (12 à vérifier) | absents, classif, early, option ×7, pool, pose · pose au hasard, hasard | oui (180 lieux) | contrôle croisé OK |
| Undertale | PC | Converti (3 à vérifier) | absents, classif, pool · hasard | oui (48 lieux) | contrôle croisé OK |
| VVVVVV | PC · fichier de sortie | Converti (1 à vérifier) | pool | oui (20 lieux) | contrôle croisé OK |
| Wargroove | PC | Converti (2 à vérifier) | classif, pool | oui (38 lieux) | contrôle croisé OK |
| Yacht Dice | PC | Échec | `collect` personnalisé, 90 règle(s) opaque(s) | — | — |
| Yoshi's Island | SNES | Échec | ROM demandée à la génération | — | — |
| Yu-Gi-Oh! 2006 | GBA | Échec | groupe qui contient un objet d'événement | — | — |
| Zillion | Master System | Échec | 147 règle(s) opaque(s) (copie de `prog_items`) | — | — |
| shapez | PC | Échec | entrée vers une région inconnue (« Achievements needing a MAM ») | — | — |

Codes des points à vérifier : voir le tableau précédent.

## Licences des mondes

- Le dépôt d'Archipelago est sous **MIT** ; un `world.toml` converti en est une œuvre dérivée (noms, numéros, régions, règles). Le distribuer demande de garder la notice MIT d'Archipelago (`THIRD_PARTY_NOTICES.md`).
- 12 mondes ont **leur propre fichier de licence** dans `worlds/<monde>/`, à recopier en plus : Celeste 64, Celeste (Open World), Civilization VI, Castlevania - Circle of the Moon, Final Fantasy Mystic Quest, Super Mario Land 2, Ocarina of Time, Pokemon Emerald, Pokemon Red and Blue, Sonic Adventure 2 Battle, Super Mario World, SMZ3.
- 4 d'entre eux (PoryGone : Celeste 64, Celeste (Open World), Sonic Adventure 2 Battle, Super Mario World) sont sous une **« Modified MIT »** qui interdit de relicencier ou de vendre « une copie ou une part substantielle » sans l'accord écrit des auteurs. Un monde converti ne pourrait pas passer sous MIT / Apache-2.0 ; il garderait leur licence, ou resterait converti sur le PC seulement (comme Slime Rancher, ADR 0042 § 6).
- Castlevania - Circle of the Moon : les sprites du logo d'Archipelago sont en CC BY-NC 4.0, mais ils sont dans le patch de ROM, pas dans ce que le convertisseur reprend.

## Suite proposée

Ordre des lots, du moins cher au plus cher :

1. **Lot 0, corrections du convertisseur, fait le 23 novembre 2026** : `has_all` / `has_any` vides, `start_inventory_from_pool` plus signalée, union des drapeaux pour les classifications mixtes (tests dans `generate/tests/conversion.rs`). Mesuré : Mega Man 2 et Mega Man 3 convertis (16 au total), 5 points à vérifier en moins. Reste : l'import du moteur « export » refuse toujours les classifications mixtes.
2. **Lot 1, objets posés d'avance, fait le 23 novembre 2026** (format version 3, ADR 0054) : `item = "…"` sur un lieu à numéro, événement qui donne un objet à numéro, verrou respecté par les deux équilibrages (corrige aussi le moteur « export »), objet jamais créé écrit `false`, seconde seed dans l'inventaire. Mesuré : 29 convertis (13 de plus), dont 21 jeux PC. Reste : Aquaria (pool plus grand que les lieux) ; Dark Souls III, Faxanadu et Noita tirent leurs objets posés au hasard.
3. **Lot 2, sondeur et sommes de compteurs, fait le 24 novembre 2026** (format version 4, export version 3, ADR 0055) : sondeur élargi (`state.multiworld`, `prog_items`, `reachable_regions`, LogicMixin, résultat non booléen, `count_from_list_unique`, plafonds réglables), règle `Rule::Sum` dans `logic`, sommes `count(a) + 2 * count(b) >= n`, `count_from_list`, `count_unique`, `count_group` dans le format. Mesuré : 36 convertis (7 de plus), aucun des 29 précédents perdu. Reste : Kingdom Hearts, Kingdom Hearts 2, Starcraft 2, Old School Runescape, Kirby's Dream Land 3, Shivers, Terraria (trop de chemins), Lingo (non monotone), Zillion (copie de `prog_items`), Overcooked! 2 (somme de conditions), et 4 causes du convertisseur révélées (Yu-Gi-Oh! 2006, Super Mario Land 2, shapez, Pokemon Red and Blue).
4. **Lot 3, compteurs (format, ADR à valider)** : objets qui ajoutent à un compteur, pour les 21 `collect` personnalisés (dont Hollow Knight, Stardew Valley, TUNIC, The Witness, Factorio, Satisfactory). Le plus gros lot ; certains (Stardew Valley, 545 règles dont des objets de règle instables) relèveront du portage à la main.
5. **Pour chaque converti, avant le badge « Converti »** : `slot_data`, `when` des options principales, tirage des pièges, puis une vraie partie avec le mod. C'est un travail par jeu, à faire par lots de 3 ou 4 jeux PC du catalogue (`docs/jeux-pc-catalogue.md`).

## Décisions (validées le 23 novembre 2026, ADR 0053)

Les 4 recommandations ci-dessous ont été acceptées telles quelles par l'utilisatrice ; elles sont consignées dans l'ADR 0053.

1. **Périmètre de « tous convertis »**. Aucun des 81 jeux n'est sur une console Nintendo actuelle, mais 32 demandent une ROM ou un ISO, et pour eux le fichier que lit le jeu (patch de ROM) est écrit par le code Python du monde (`generate_output`). Un monde natif peut placer les objets, pas écrire ce patch sans porter ce code. *Recommandation* : viser d'abord les **49 jeux PC** ; pour les jeux à ROM, garder le générateur Python (moteur « export » au mieux) et ne pas les compter dans l'objectif.
2. **Licence et distribution des mondes convertis**. *Recommandation* : chaque monde converti publié comme paquet du registre (ADR 0046) **sous la licence de son monde d'origine** (MIT d'Archipelago, avec la notice, plus le fichier `LICENSE` du monde s'il en a un), et non sous MIT / Apache-2.0 de Raftup ; les 4 mondes « Modified MIT » de PoryGone ne sont pas publiés sans l'accord écrit des auteurs (conversion sur le PC seulement).
3. **Mondes qui ne se convertissent pas automatiquement**. *Recommandation* : le **repli sur le générateur Python reste permanent** (badge « Archipelago »), on étend le format et le sondeur par lots (1 à 4), et le portage à la main est réservé à quelques jeux PC très joués, un par un, sur demande.
4. **Ce qu'il faut pour afficher « Converti »**. Aujourd'hui une conversion est un instantané des options par défaut, sans `slot_data`. *Recommandation* : le badge « Converti » seulement après le point 5 de la suite (`slot_data`, options principales, une vraie partie) ; avant, le jeu garde le badge « Archipelago ».

## Reproduire

Avec un runtime installé par Raftup (ici `D:\raftup-test\runtime`), sous PowerShell :

1. La liste des 81 jeux mesurés est dans `crates/launcher-core/tests/data/jeux-archipelago-0.6.8.txt` (noms tirés de `AutoWorldRegister.world_types`, mondes de `worlds/` non cachés).
2. Lancer l'inventaire (7 minutes environ) :

```powershell
$env:RAFTUP_REAL_RUNTIME = "D:\raftup-test\runtime"
$env:RAFTUP_INVENTAIRE_JEUX = "$PWD\crates\launcher-core\tests\data\jeux-archipelago-0.6.8.txt"
$env:RAFTUP_INVENTAIRE_OUT = "$env:TEMP\inventaire.jsonl"
$env:RAFTUP_INVENTAIRE_NATIFS = "$env:TEMP\natifs"   # facultatif, dossier existant hors du dépôt
$env:RAFTUP_INVENTAIRE_ALLER_RETOUR = ((Get-Content $env:RAFTUP_INVENTAIRE_JEUX -Encoding utf8) -join "|")
cargo test -p launcher-core --test inventaire_real -- --ignored --nocapture --test-threads=1
```

Chaque ligne de `inventaire.jsonl` donne, pour un jeu : l'étape atteinte (`export`, `convert`, `done`…), l'erreur, le nombre de règles sondées et opaques (avec des exemples de raisons), le drapeau `collect` personnalisé, le résultat de l'import, les points à vérifier, la seed Rust (lieux remplis) et l'aller-retour.
