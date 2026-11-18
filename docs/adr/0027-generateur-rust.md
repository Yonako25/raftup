# 0027 — Générateur Rust (V2) : `logic`, `fill`, `generate`

- Statut : **accepté** le 18 novembre 2026 (les 6 décisions du bas, telles que recommandées)
- Date : 2026-11-18

## Contexte

- Au MVP, toute la génération passe par le générateur officiel d'Archipelago 0.6.8, dans un Python embarqué piloté par `py-bridge` (ADR 0016, 0017). Il faut Python (≈ 72 Mo au premier usage), le code d'Archipelago, et chaque apworld est du code arbitraire exécuté sans bac à sable.
- La V2 de la SPEC (§ 4) prévoit un générateur écrit en Rust (`logic`, `fill`, `generate`), une faisabilité vérifiée et des erreurs expliquées, et une génération reproductible (NF12).
- « Se séparer d'Archipelago » veut dire se séparer de son **générateur Python**, pas de son **protocole** : un mod existant doit toujours se connecter au serveur Raftup sans modification (règle de `CLAUDE.md`). Ce que le générateur produit doit donc rester exactement ce que le serveur et les mods attendent : mêmes noms et numéros d'objets et de lieux, même datapackage, même `slot_data`.
- Constats faits en lisant Archipelago 0.6.8 (runtime installé par Raftup) :
  - le monde **APQuest** fait partie du dépôt d'Archipelago (MIT), sans ROM, environ 880 lignes ; il est déjà utilisé par le test réel de génération ;
  - ses règles sont écrites avec le module **`rule_builder`** d'Archipelago (`Has`, `HasAll`, `|`, `&`, `OptionFilter`…), qui sait se sérialiser (`to_dict` / `from_dict`) : c'est le « module de règles déclaratives » dont parle la SPEC ;
  - APQuest a 7 options (`hard_mode`, `hammer`, `extra_starting_chest`, `trap_chance`, `start_with_one_confetti_cannon`, `confetti_explosiveness`, `player_sprite`) et son `slot_data` en recopie 5 ;
  - l'apworld Slime Rancher n'a **pas de licence** (ADR 0021) : on n'a pas le droit d'en recopier le code en Rust.

## Décision

### 1. Trois crates, chacune avec un seul rôle

| Crate | Rôle | Dépend de |
| --- | --- | --- |
| `logic` | Modèle d'un multiworld (régions, entrées, lieux, objets, règles) et calcul de ce qui est atteignable avec un inventaire donné (balayage par sphères) | rien de Raftup |
| `fill` | Placement des objets dans les lieux (progression, puis utiles, puis remplissage), vérification de faisabilité, explication des échecs | `logic` |
| `generate` | Orchestration : options du YAML → mondes Rust → `fill` → `MultiData` → fichier `.archipelago` dans `AP_<seed>.zip` ; mondes écrits en Rust dans `generate::worlds` | `logic`, `fill`, `multidata` |

- `worlds-native` (format déclaratif TOML) reste en V3 : on ne crée pas la crate. En attendant, un monde Rust est un module de `generate::worlds` (par exemple `generate::worlds::apquest`).
- `launcher-core::generation` reçoit un deuxième moteur à côté de `PythonBackend` : `RustBackend` (même `GenerateRequest`, même résultat). L'interface (`launcher-ui`) ne change presque pas.
- Les trois crates sont en MIT / Apache-2.0. Le portage d'APQuest et de `rule_builder` garde la notice MIT d'Archipelago dans `THIRD_PARTY_NOTICES.md`.

### 2. Modèle de monde : arènes à index `u32`, ordre déterministe

- Un seul `MultiWorld` pour tous les joueurs (comme Archipelago), avec des `Vec` : `regions`, `entrances`, `locations`, `items`. On désigne un élément par son index (`RegionId(u32)`, `LocationId(u32)`…), jamais par une référence : pas de durées de vie compliquées.
- Chaque élément porte son joueur (`SlotId`) et son numéro réseau (le numéro du datapackage, celui que voit le mod).
- Une règle est un arbre fermé, qui reprend le vocabulaire de `rule_builder` :
  `True`, `False`, `Has { item, count }`, `HasAll`, `HasAny`, `HasAllCounts`, `HasGroup`, `CanReach(region | location | entrance)`, `And(Vec)`, `Or(Vec)`. Les `OptionFilter` sont résolus à la construction du monde (on connaît les options), ils n'existent plus dans l'arbre.
- L'inventaire (`State`) est un tableau de compteurs indexé par (joueur, objet) : évaluer une règle ne fait aucune allocation.
- Ordre : celui de création (index croissant) partout ; `BTreeMap` pour les tables nommées ; aucune itération sur une `HashMap` ; aucun nombre à virgule dans le placement.

### 3. Format de sortie : un vrai `.archipelago`, dans `AP_<seed>.zip`

- `generate` construit un `multidata::MultiData` puis l'écrit en `.archipelago` (octet de version, zlib, pickle) avec un **écrivain de pickle** à ajouter à `multidata`, limité aux mêmes classes que la liste blanche du lecteur.
- Pourquoi pas le `.raftup` : le `.raftup` est une **sauvegarde** (l'état d'une partie qui avance, ADR 0014), pas une seed. Inventer un format de seed Raftup casserait la promesse de la SPEC (§ 1) : « une partie qui marche sur Raftup doit marcher sur archipelago.gg ». Avec un vrai `.archipelago`, la seed Rust s'héberge sur Raftup, sur le serveur officiel et sur archipelago.gg ; tout le reste du launcher (`generate_and_host`, sauvegarde, invitations) marche sans changement.
- Les mods ne lisent jamais la seed : ils reçoivent `slot_data` et le datapackage par le protocole. Pour qu'ils marchent sans modification, le monde Rust doit produire **le même datapackage** (noms, numéros, groupes, `checksum` calculé comme Archipelago) et **le même `slot_data`** que le monde Python, pour les mêmes options.
- Versions annoncées : `generator_version` et `minimum_server_version` valent ceux d'Archipelago 0.6.8 (la version dont on reproduit les mondes). Le spoiler indique « Raftup <version> » comme générateur.

### 4. Seed reproductible : reproductible **dans Raftup**, pas identique au Python

- Garantie (NF12) : même nombre de seed + même version de Raftup + mêmes YAML ⇒ **mêmes octets** de `.archipelago`, sur Windows, macOS et Linux.
- Hasard : générateur ChaCha (crate `rand_chacha`, version épinglée, MIT / Apache), un flux par monde dérivé de (seed, numéro de slot), plus un flux pour le placement. Le nom de seed reste 20 chiffres, comme Archipelago.
- **Pas** de placement identique à celui du générateur Python pour la même seed : il faudrait reproduire le Mersenne Twister de Python, chaque appel à `random` de chaque monde et l'ordre de chaque `set` et `dict` du code Python. C'est très coûteux et fragile à chaque version d'Archipelago, et aucun joueur n'en a besoin. Une même seed donne donc deux parties différentes selon le moteur ; le moteur choisi est affiché et écrit dans le spoiler.

### 5. Premier monde : APQuest

- Monde officiel, MIT, sans ROM, petit, déjà générateur de référence du test réel, et écrit avec `rule_builder` : il exerce exactement le modèle de règles prévu, et on peut le comparer à l'officiel sur toutes les combinaisons d'options.
- Slime Rancher ne peut pas être porté à la main (apworld sans licence). Il viendra par l'**export des règles** (voir 7) : le code de l'apworld tourne sur le PC du joueur et seul son résultat (régions, lieux, règles) passe à Rust, sans rien recopier dans le dépôt.

### 6. Cohabitation avec le générateur Python

- Le générateur Python reste installé et sert de **chemin de secours**. Règle simple au début : le moteur Rust prend la génération seulement si **tous** les joueurs jouent à un monde pris en charge, avec des options prises en charge ; sinon toute la génération passe en Python (un multiworld est rempli par un seul moteur, pas de mélange).
- Le moteur Rust reste **sur option** (désactivé par défaut) tant que la vérification (section 8) n'est pas complète et que l'équilibrage de progression n'est pas fait. Ensuite il devient le moteur par défaut pour les mondes pris en charge, avec une option « forcer Archipelago ».
- Options non prises en charge au départ (⇒ Python) : plando, `item_links`, `start_location_hints`, `early_items`, `local_items` / `non_local_items`, `accessibility` autre que `full`. Elles s'ajoutent une par une, chacune avec ses tests.
- `py-bridge`, `generator-pins.toml` et l'étape `generate` actuelle (ADR 0016, 0017) ne changent pas.

### 7. Ordre des étapes (chacune testable, chacune commitée)

1. `logic` : modèle, règles, balayage par sphères ; tests sur de petits mondes écrits à la main.
2. `multidata` : écrivain de pickle + `write_archipelago` ; relu par notre lecteur et par le vrai `pickle` de Python (test réel `#[ignore]`).
3. `fill` : placement « supposé » (on retire l'objet, on calcule ce qui reste atteignable sans lui, on le place dans un lieu atteignable libre), puis utiles et remplissage ; faisabilité avant et après ; erreur expliquée (lieu inatteignable et clause de règle qui échoue).
4. `generate` + APQuest en Rust, branché sur `RustBackend` (sur option).
5. Équilibrage de progression (`progression_balancing`), puis passage en moteur par défaut pour APQuest.
6. Export des règles depuis un apworld (nouvelle méthode `export_world` du worker Python : régions, lieux, objets, règles `rule_builder` en `to_dict`) ; une règle Python non déclarative (lambda) devient `Opaque` et le monde reste en Python. C'est cette étape qui ouvrira Slime Rancher et la plupart des mondes ; elle fera l'objet de son propre ADR.

### 8. Comment on vérifie la correction

- **Faisabilité à chaque génération** : après placement, un balayage depuis l'inventaire de départ doit atteindre tous les lieux (`accessibility: full`) et l'objectif de chaque joueur ; sinon la génération échoue avec une erreur expliquée, jamais une seed invalide.
- **Faisabilité avant placement** : avec tous les objets en main, chaque lieu et chaque objectif doivent être atteignables ; sinon les options sont impossibles et on dit quelle règle bloque.
- **Tests de propriétés** (`logic`, `fill`) : des milliers de seeds × toutes les combinaisons d'options d'APQuest : toujours faisable, chaque objet placé une fois et une seule, même résultat deux fois de suite.
- **Comparaison avec l'officiel** (test réel `#[ignore]`, Archipelago 0.6.8 installé par Raftup) pour chaque combinaison d'options : même datapackage (`checksum` compris), même liste de lieux, même nombre de chaque objet dans le pool, même `slot_data`, même inventaire de départ. Le placement lui-même n'est pas comparé (décision 4) ; on relève seulement, pour information, la répartition du nombre de sphères.
- **La seed Rust s'héberge** sur le serveur Raftup et sur le serveur officiel 0.6.8 (banc des tests différentiels, ADR 0008), avec les mêmes réponses à `Connect`.
- **En vrai** : le client APQuest d'Archipelago se connecte à une seed Rust hébergée par Raftup et la joue jusqu'à l'objectif.
- **Reproductibilité** : empreinte sha256 du `.archipelago` comparée entre deux OS dès que la CI multi-OS existe (NF11).

## Conséquences

- Pour les mondes pris en charge : génération sans Python, plus rapide, reproductible, avec erreurs lisibles. Les autres mondes gardent exactement le comportement actuel.
- Le Python embarqué n'est pas retiré en V2 : se passer complètement de lui demande des mondes natifs (V3) ou portés. Au début, seul APQuest s'en passe.
- Chaque monde porté en Rust doit être suivi à chaque montée de version d'Archipelago (noms, numéros, options, `slot_data`) : la comparaison avec l'officiel (section 8) le détecte.
- Nouvelle dépendance : `rand_chacha` (et `rand_core`), MIT / Apache-2.0, version épinglée. Écrire du pickle demande du soin : il est limité à la liste blanche et vérifié contre le vrai Python.
- Une même seed ne donne pas la même partie selon le moteur : à afficher clairement dans l'interface et le spoiler.

## Décisions validées (18 novembre 2026)

| # | Question | Choix |
| --- | --- | --- |
| 1 | Découpage `logic` / `fill` / `generate`, mondes Rust dans `generate::worlds` en attendant `worlds-native` (V3) | Oui |
| 2 | Sortie en vrai `.archipelago` (écrivain de pickle dans `multidata`) plutôt qu'un format de seed Raftup | `.archipelago` |
| 3 | Reproductible dans Raftup (ChaCha), pas identique au placement du Python | Oui |
| 4 | Premier monde : APQuest ; Slime Rancher par l'export des règles plus tard | APQuest |
| 5 | Un seul moteur par multiworld ; moteur Rust sur option jusqu'à l'étape 5, puis par défaut | Oui |
| 6 | Ordre des étapes 1 à 6 ci-dessus, en commençant par `logic` | Oui |

## Mise en œuvre de l'étape 4 (18 novembre 2026)

Choix pris en codant, dans le cadre des décisions ci-dessus :

- **Hasard** : ChaCha20 (`rand_chacha` 0.10.0, `rand_core` 0.10.1, épinglés) ; un flux par usage (`yaml`, `world`, `fill`) et par slot, graine = SHA-256 de (étiquette, seed, slot). Le mélange (Fisher-Yates) et les tirages bornés sont écrits dans `fill`, pour ne pas dépendre de l'algorithme de la crate `rand`.
- **Impasse du placement supposé** : selon l'ordre tiré, un objet peut ne plus avoir de lieu ; `fill` recommence avec la suite du même flux, 20 essais au plus.
- **Nom de seed** : le numéro de seed sur 20 chiffres (`00000000000000012345`).
- **YAML** : lus par `yaml-rust2` 0.13 (MIT / Apache-2.0). Le moteur Rust ne prend la génération que s'il comprend tout : jeu porté, options connues, valeurs simples, poids ou `random` ; `accessibility: full` ; options communes avancées vides ; pas de substitution dans les noms. Sinon la raison est écrite en tête du journal et Archipelago génère.
- **Launcher** : `RustBackend` remplit le même contrat que `PythonBackend` (zip `AP_<seed>.zip` dans le dossier de sortie) ; il ne touche ni aux apworlds ni au dossier d'Archipelago, et ne demande pas l'installation de Python. Case « Moteur Rust (expérimental) » dans « Créer une partie », décochée par défaut.
- **Ignoré pour l'instant** : `progression_balancing` (étape 5), le spoiler, le `meta.yaml` des préréglages.
