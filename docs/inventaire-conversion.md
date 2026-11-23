# Inventaire de conversion des mondes d'Archipelago 0.6.8

Mesuré le **23 novembre 2026** sur les **81 mondes intégrés** d'Archipelago 0.6.8 (dossier `worlds/` du générateur épinglé, sans le monde caché « Archipelago » ni les apworlds ajoutées), avec les **options par défaut**, un joueur, seed 1. Rien n'a été publié ; les `world.toml` produits sont restés hors du dépôt.

Chaîne essayée pour chaque jeu (test `launcher-core/tests/inventaire_real.rs`) :

1. **export** du worker Python (ADR 0028), arrêté avant le placement ;
2. **import** de l'export et contrôle croisé des règles contre le vrai Archipelago (moteur « export ») ;
3. **conversion** en monde natif (`generate::convert`, ADR 0042), relue par `worlds_native::parse_world` ;
4. **seed complète par le moteur Rust seul** (sans Python), relue par `multidata` ;
5. **aller-retour** : le monde converti, renommé « … Natif », réexporté en apworld, réexporté par le worker et repassé au contrôle croisé.

## Totaux

| Résultat | Jeux |
| --- | --- |
| Converti sans point à vérifier | **0** |
| Converti avec des points « À vérifier » | **14** (seed Rust et aller-retour réussis pour les 14) |
| Échec | **67** |

- Les 14 convertis : A Short Hike, APQuest, Adventure, Bumper Stickers, Choo-Choo Charles, Civilization VI, Hylics 2, Inscryption, Muse Dash, Risk of Rain 2, Saving Princess, Sonic Adventure 2 Battle, Undertale, VVVVVV.
- Sur les 49 jeux PC (sans ROM ni ISO), **13 convertis** ; sur les 32 jeux à ROM ou ISO, 1 (Adventure).
- Aucun jeu de la liste n'est sur une console Nintendo actuelle (Switch) ; 21 demandent une ROM ou un ISO d'une ancienne console Nintendo.
- Durée du lot : 7 minutes (426 s) pour les 81 jeux.

### Ce que « converti » veut dire ici

Un monde converti n'est pas encore jouable tel quel :

- c'est un **instantané** des options par défaut (ADR 0042, § 1) : tant que l'auteur n'a pas écrit les `when`, le YAML ne peut régler aucune option du jeu ;
- **`slot_data` absent** : l'inventaire convertit sans seed d'Archipelago. 11 des 14 mondes ont un `fill_slot_data` que leur mod lit ; il faut le reprendre (option `--seed` de `raftup-monde`, en valeurs littérales) puis l'écrire à la main s'il dépend des options ;
- **fichier de sortie** : 3 convertis écrivent un fichier à la génération (`generate_output`) que le moteur Rust ne produit pas : Adventure (patch de ROM), Civilization VI (mod propre à la seed), VVVVVV (fichier en solo seulement). Sans lui, le jeu ne se lance pas (sauf VVVVVV en multi).

## Causes d'échec, par fréquence

Le convertisseur s'arrête à la **première** cause ; l'export, lui, compte toutes les règles opaques et signale le `collect` personnalisé. Les chiffres de la colonne « en tout » sont donc des minimums pour les objets verrouillés.

| Cause | 1ʳᵉ cause | En tout | Seule cause | Ce qu'il faudrait faire |
| --- | --- | --- | --- | --- |
| Objet verrouillé dans un lieu à numéro (objet vanilla, boutique, sortie de niveau) | 24 | ≥ 24 | 14 | **Format version 3** : `item = "…"` sur un lieu à numéro (objet posé d'avance, gardé dans le datapackage) ; à valider (ADR) |
| `collect` / `remove` redéfinis (compteurs maison, objets qui valent N) | 21 | 21 | 5 | Format : objets qui **ajoutent à un compteur** (`gives = { "Argent" = 50 }`) et règles sur ce compteur ; les cas plus libres restent au portage à la main. À valider (ADR) |
| Règle opaque (sondeur) | 8 | **36** | 8 | Sondeur : voir le détail plus bas |
| ROM demandée dès la génération | 8 | 8 | 8 | En mode export, ne pas exiger la ROM (elle ne sert qu'à `generate_output`) ; mais le patch reste écrit par Python : décision 3 |
| `has_all` vide | 2 | 2 | 2 | **Bogue du convertisseur** : `has_all` d'une liste vide vaut `true` (Mega Man 2, Mega Man 3) |
| `has_from_list` hors groupe déclaré | 1 | 1 | 1 | Format : règle « somme d'exemplaires sur une liste » (prévue par l'ADR 0042, version 3) |
| Plus de 256 combinaisons (`au moins 6 parmi 23`) | 1 | 1 | 0 | Format : règle `at_least(n, liste)` au lieu du dépliage |
| Objet d'événement qui a aussi un numéro | 1 | 1 | 0 | Même chose que l'objet verrouillé (version 3) |
| Options par défaut refusées par le monde | 1 | 1 | 1 | Final Fantasy exige des réglages faits sur le site de FFR : à essayer avec un vrai YAML |

Combinaisons des 28 jeux à plusieurs causes : `collect` + règles opaques (16), objet verrouillé + règles opaques (10), règles opaques + autre (2).

### Règles opaques : d'où elles viennent

Raisons relevées (échantillon de 8 raisons distinctes par jeu, 36 jeux) :

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

### Points « À vérifier » des 14 convertis

| Code | Point | Jeux |
| --- | --- | --- |
| `classif` | objet ni dans le pool ni au départ, écrit `filler` | 12 |
| `pool` | fin du pool écrite en `[filler]` : le monde tire-t-il des pièges ? | 11 |
| `option` | option d'un type que le format ne connaît pas (listes, dictionnaires, poids) ; `start_inventory_from_pool`, option commune, est signalée à tort (**bruit à retirer du convertisseur**) | 8 jeux, 40 options |
| `absents` | lieux absents avec ces options, gardés avec `when = 'false'` | 7 |
| `mixte` | exemplaires d'un même objet avec des classifications différentes | 3 jeux, 25 objets |
| `item_rule` | règles d'objet non converties | 3 |
| `early`, `drapeaux` | objets à placer tôt ignorés ; drapeaux de classification inconnus | 1 chacun |

À noter : pour Muse Dash, Sonic Adventure 2 Battle et Saving Princess, l'**import** (moteur « export » de l'ADR 0028) refuse l'objet à classifications mixtes alors que la conversion le garde avec un point à vérifier. Les deux chemins devraient se comporter pareil.

## Tableau par jeu

Support : « PC » quand le jeu se joue sur sa version PC sans ROM ni ISO. « licence restrictive » : licence « Modified MIT » qui interdit la relicence et la vente (décision 2). « fichier de sortie » : le monde écrit un fichier que le moteur Rust ne produit pas.

| Jeu | Support | Résultat | Points à vérifier (codes) ou causes d'échec | Seed Rust | Aller-retour |
| --- | --- | --- | --- | --- | --- |
| A Hat in Time | PC | Échec | `collect` personnalisé | — | — |
| A Link to the Past | SNES | Échec | ROM demandée à la génération | — | — |
| A Short Hike | PC | Converti (3 à vérifier) | item_rule, option, pool | oui (131 lieux) | contrôle croisé OK |
| APQuest | PC | Converti (3 à vérifier) | absents, classif, pool | oui (6 lieux) | contrôle croisé OK |
| Adventure | Atari 2600 · fichier de sortie | Converti (3 à vérifier) | classif, item_rule, pool | oui (24 lieux) | contrôle croisé OK |
| Aquaria | PC | Échec | objet verrouillé hors événement | — | — |
| Blasphemous | PC | Échec | `collect` personnalisé, 87 règle(s) opaque(s) | — | — |
| Bomb Rush Cyberfunk | PC | Échec | `collect` personnalisé, 78 règle(s) opaque(s) | — | — |
| Bumper Stickers | PC | Converti (2 à vérifier) | classif, pool | oui (100 lieux) | contrôle croisé OK |
| Castlevania - Circle of the Moon | GBA | Échec | objet verrouillé hors événement | — | — |
| Castlevania 64 | N64 | Échec | 5 règle(s) opaque(s) | — | — |
| Celeste (Open World) | PC · licence restrictive | Échec | objet verrouillé hors événement | — | — |
| Celeste 64 | PC · licence restrictive | Échec | objet verrouillé hors événement | — | — |
| ChecksFinder | PC | Échec | `has_from_list` hors groupe | — | — |
| Choo-Choo Charles | PC | Converti (2 à vérifier) | classif, option | oui (691 lieux) | contrôle croisé OK |
| Civilization VI | PC · fichier de sortie | Converti (8 à vérifier) | absents, classif, early, item_rule, option ×3, pool | oui (148 lieux) | contrôle croisé OK |
| DLCQuest | PC | Échec | `collect` personnalisé, 16 règle(s) opaque(s) | — | — |
| DOOM 1993 | PC | Échec | objet verrouillé hors événement | — | — |
| DOOM II | PC | Échec | objet verrouillé hors événement | — | — |
| Dark Souls III | PC | Échec | objet verrouillé hors événement | — | — |
| EarthBound | SNES | Échec | 10 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Factorio | PC | Échec | `collect` personnalisé | — | — |
| Faxanadu | NES | Échec | objet verrouillé hors événement | — | — |
| Final Fantasy | NES | Échec | options par défaut refusées (réglages FFR) | — | — |
| Final Fantasy Mystic Quest | SNES | Échec | `collect` personnalisé | — | — |
| Gauntlet Legends | N64 | Échec | `collect` personnalisé | — | — |
| Heretic | PC | Échec | objet verrouillé hors événement | — | — |
| Hollow Knight | PC | Échec | `collect` personnalisé, 326 règle(s) opaque(s) | — | — |
| Hylics 2 | PC | Converti (3 à vérifier) | absents, classif, option | oui (133 lieux) | contrôle croisé OK |
| Inscryption | PC | Converti (3 à vérifier) | classif, option, pool | oui (100 lieux) | contrôle croisé OK |
| Jak and Daxter: The Precursor Legacy | PS2 (ISO, portage OpenGOAL) | Échec | `collect` personnalisé, 15 règle(s) opaque(s) | — | — |
| Kingdom Hearts | PC | Échec | 63 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Kingdom Hearts 2 | PC | Échec | 30 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Kirby's Dream Land 3 | SNES | Échec | 1 règle(s) opaque(s) | — | — |
| Landstalker - The Treasures of King Nole | Mega Drive | Échec | objet verrouillé hors événement | — | — |
| Lingo | PC | Échec | 4 règle(s) opaque(s) | — | — |
| Links Awakening DX | GBC | Échec | `collect` personnalisé, 97 règle(s) opaque(s) | — | — |
| Lufia II Ancient Cave | SNES | Échec | ROM demandée à la génération | — | — |
| Mario & Luigi Superstar Saga | GBA | Échec | objet verrouillé hors événement | — | — |
| Mega Man 2 | NES | Échec | `has_all` vide (bogue du convertisseur) | — | — |
| Mega Man 3 | NES | Échec | `has_all` vide (bogue du convertisseur) | — | — |
| MegaMan Battle Network 3 | GBA | Échec | ROM demandée à la génération | — | — |
| Meritous | PC | Échec | objet verrouillé hors événement | — | — |
| Muse Dash | PC | Converti (31 à vérifier) | absents, classif, drapeaux, mixte ×21, option ×7 | oui (90 lieux) | contrôle croisé OK |
| Noita | PC | Échec | objet verrouillé hors événement | — | — |
| Ocarina of Time | N64 | Échec | ROM demandée à la génération | — | — |
| Old School Runescape | PC | Échec | 35 règle(s) opaque(s) | — | — |
| Overcooked! 2 | PC | Échec | 126 règle(s) opaque(s) | — | — |
| Paint | PC | Échec | `collect` personnalisé, 131 règle(s) opaque(s) | — | — |
| Pokemon Emerald | GBA | Échec | `collect` personnalisé | — | — |
| Pokemon Red and Blue | GB | Échec | 143 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Raft | PC | Échec | `collect` personnalisé, 142 règle(s) opaque(s) | — | — |
| Risk of Rain 2 | PC | Converti (3 à vérifier) | absents, classif, pool | oui (187 lieux) | contrôle croisé OK |
| SMZ3 | SNES | Échec | `collect` personnalisé, 357 règle(s) opaque(s) | — | — |
| Satisfactory | PC | Échec | `collect` personnalisé, 38 règle(s) opaque(s) | — | — |
| Saving Princess | PC | Converti (6 à vérifier) | classif, mixte ×3, option, pool | oui (36 lieux) | contrôle croisé OK |
| Secret of Evermore | SNES | Échec | ROM demandée à la génération | — | — |
| Shivers | PC | Échec | 1 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Sonic Adventure 2 Battle | PC · licence restrictive | Converti (30 à vérifier) | absents, classif, mixte, option ×26, pool | oui (206 lieux) | contrôle croisé OK |
| Starcraft 2 | PC | Échec | 162 règle(s) opaque(s) | — | — |
| Stardew Valley | PC | Échec | `collect` personnalisé, 545 règle(s) opaque(s) | — | — |
| Subnautica | PC | Échec | 135 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Super Mario 64 | N64 (ROM, portage sm64ex) | Échec | objet verrouillé hors événement | — | — |
| Super Mario Land 2 | GB | Échec | 30 règle(s) opaque(s), événement avec numéro | — | — |
| Super Mario World | SNES · licence restrictive | Échec | ROM demandée à la génération | — | — |
| Super Metroid | SNES | Échec | `collect` personnalisé, 301 règle(s) opaque(s) | — | — |
| TUNIC | PC | Échec | `collect` personnalisé, 14 règle(s) opaque(s) | — | — |
| Terraria | PC | Échec | 3 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| The Legend of Zelda | NES | Échec | ROM demandée à la génération | — | — |
| The Messenger | PC | Échec | `collect` personnalisé, 23 règle(s) opaque(s) | — | — |
| The Wind Waker | GameCube (ISO) | Échec | 40 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| The Witness | PC | Échec | `collect` personnalisé, 75 règle(s) opaque(s) | — | — |
| Timespinner | PC | Échec | 3 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| Undertale | PC | Converti (3 à vérifier) | absents, classif, pool | oui (48 lieux) | contrôle croisé OK |
| VVVVVV | PC · fichier de sortie | Converti (1 à vérifier) | pool | oui (20 lieux) | contrôle croisé OK |
| Wargroove | PC | Échec | 40 règle(s) opaque(s) | — | — |
| Yacht Dice | PC | Échec | `collect` personnalisé, 90 règle(s) opaque(s) | — | — |
| Yoshi's Island | SNES | Échec | ROM demandée à la génération | — | — |
| Yu-Gi-Oh! 2006 | GBA | Échec | 4 règle(s) opaque(s), plus de 256 combinaisons | — | — |
| Zillion | Master System | Échec | 147 règle(s) opaque(s), objet verrouillé hors événement | — | — |
| shapez | PC | Échec | 1 règle(s) opaque(s) | — | — |

cumul [('objet verrouillé hors événement', 24), ('`collect` personnalisé', 21), ('ROM demandée à la génération', 8), ('1', 3), ('30', 2), ('4', 2), ('`has_all` vide (bogue du convertisseur)', 2), ('3', 2), ('40', 2), ('87', 1), ('78', 1), ('5', 1), ('`has_from_list` hors groupe', 1), ('16', 1), ('10', 1), ('options par défaut refusées (réglages FFR)', 1), ('326', 1), ('15', 1), ('63', 1), ('97', 1), ('35', 1), ('126', 1), ('131', 1), ('143', 1), ('142', 1), ('357', 1), ('38', 1), ('162', 1), ('545', 1), ('135', 1), ('événement avec numéro', 1), ('301', 1), ('14', 1), ('23', 1), ('75', 1), ('90', 1), ('plus de 256 combinaisons', 1), ('147', 1)]
seul [('objet verrouillé hors événement', 14), ('ROM demandée à la génération', 8), ('`collect` personnalisé', 5), ('1', 2), ('`has_all` vide (bogue du convertisseur)', 2), ('5', 1), ('`has_from_list` hors groupe', 1), ('options par défaut refusées (réglages FFR)', 1), ('4', 1), ('35', 1), ('126', 1), ('162', 1), ('40', 1)]
PC 49 convertis PC 13

Codes des points à vérifier : voir le tableau précédent.

## Licences des mondes

- Le dépôt d'Archipelago est sous **MIT** ; un `world.toml` converti en est une œuvre dérivée (noms, numéros, régions, règles). Le distribuer demande de garder la notice MIT d'Archipelago (`THIRD_PARTY_NOTICES.md`).
- 12 mondes ont **leur propre fichier de licence** dans `worlds/<monde>/`, à recopier en plus : Celeste 64, Celeste (Open World), Civilization VI, Castlevania - Circle of the Moon, Final Fantasy Mystic Quest, Super Mario Land 2, Ocarina of Time, Pokemon Emerald, Pokemon Red and Blue, Sonic Adventure 2 Battle, Super Mario World, SMZ3.
- 4 d'entre eux (PoryGone : Celeste 64, Celeste (Open World), Sonic Adventure 2 Battle, Super Mario World) sont sous une **« Modified MIT »** qui interdit de relicencier ou de vendre « une copie ou une part substantielle » sans l'accord écrit des auteurs. Un monde converti ne pourrait pas passer sous MIT / Apache-2.0 ; il garderait leur licence, ou resterait converti sur le PC seulement (comme Slime Rancher, ADR 0042 § 6).
- Castlevania - Circle of the Moon : les sprites du logo d'Archipelago sont en CC BY-NC 4.0, mais ils sont dans le patch de ROM, pas dans ce que le convertisseur reprend.

## Suite proposée (rien n'est codé)

Ordre des lots, du moins cher au plus cher :

1. **Lot 0, corrections du convertisseur, sans décision** : `has_all` vide ⇒ `true` (bogue, avec un test), ne plus signaler les options communes (`start_inventory_from_pool`…), même traitement des classifications mixtes à l'import et à la conversion. Gain attendu : Mega Man 2 et Mega Man 3 (16 convertis).
2. **Lot 1, objets posés d'avance (format version 3, ADR à valider)** : un lieu à numéro peut porter un objet fixé. Débloque jusqu'à 14 jeux qui n'ont que cette cause : Aquaria, DOOM 1993, DOOM II, Dark Souls III, Heretic, Meritous, Noita (PC), Celeste 64 et Celeste (Open World) (licence restrictive), Castlevania - Circle of the Moon, Faxanadu, Landstalker, Mario & Luigi Superstar Saga, Super Mario 64 (ROM). Il peut rester des causes cachées derrière la première : à remesurer.
3. **Lot 2, sondeur** : `state.multiworld` et `prog_items` en lecture seule, résultat non booléen, `count_from_list_unique`, plafonds réglables ; puis la règle « somme pondérée » dans `logic` et le format (avec `has_from_list` et `at_least`). Vise les 8 jeux qui n'ont que des règles opaques (Castlevania 64, Kirby's Dream Land 3, Lingo, Old School Runescape, Overcooked! 2, Starcraft 2, Wargroove, shapez), ChecksFinder et Yu-Gi-Oh! 2006, et une part des 10 jeux « verrouillé + opaque ».
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
