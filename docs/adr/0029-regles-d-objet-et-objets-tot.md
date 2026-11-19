# 0029 — Règles d'objet, objets à placer tôt et accessibilité dans le moteur « export »

- Statut : **accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice** (session `early-items`, `docs/plan-v2-sessions.md`)
- Date : 2026-11-19
- Prolonge l'ADR 0028 (§ 6 : « `item_rule` non pris en charge au départ ») et l'ADR 0027 (`fill`).

## Contexte

Avant cette décision, trois choses empêchaient le moteur « export » (ADR 0028) de prendre une génération, ou le faisaient mal :

- **règle d'objet par lieu** (`Location.item_rule`, `always_allow`) : le monde était refusé (repli sur Archipelago seul). C'est aussi par là qu'Archipelago applique les options communes `local_items` et `non_local_items` (`worlds/generic/Rules.py`, `locality_rules`) : dès qu'un joueur d'une partie à plusieurs s'en servait, **toute** la partie repassait au Python ;
- **objets à placer tôt** (`multiworld.early_items`, `local_early_items`, posés par les mondes dans `generate_early`) : refusés aussi ;
- **`accessibility: minimal`** (et `items`) : ignorée ; Raftup exigeait toujours que chaque lieu soit atteignable. Plus strict qu'Archipelago, donc des échecs de placement (et des replis) là où Archipelago réussit, et une partie plus contrainte que ce que le joueur a demandé.

Références lues (sans rien recopier) : `Fill.py` d'Archipelago 0.6.8 (`distribute_early_items`, `fill_restrictive`, `accessibility_corrections`, `inaccessible_location_rules`, `remaining_fill`), `BaseClasses.py` (`Location.can_fill`, `MultiWorld.fulfills_accessibility`), `worlds/generic/Rules.py` (`locality_rules`), `Main.py`.

## Décision

### 1. Règles d'objet : des listes calculées par Archipelago

Une règle d'objet est une fonction Python (`item_rule(item)`, `always_allow(state, item)`). Raftup ne l'évalue pas pendant son placement (ADR 0028, décision 3) : le worker la **calcule à l'export** sur un exemplaire de chaque sorte d'objet du **pool** (seuls ces objets sont placés), et l'export la décrit par des listes :

- `list` + `allow_only` : les sortes refusées, ou les seules acceptées (la plus courte des deux) ;
- `always` : les sortes que `always_allow` accepte, sauf celles qui sont `non_local_items` de leur joueur (comme `Location.can_fill`).

Les règles identiques sont rangées une seule fois (`item_rules` en tête de l'export, un index par lieu), et les fonctions déjà calculées sont reconnues par leur identité : `locality_rules` partage déjà la même fonction entre les lieux d'un joueur.

Dans `logic`, une règle d'objet est une arène (`ItemRule`, `ItemRuleId`) et `MultiWorld::can_hold(lieu, objet)` reprend `Location.can_fill` sans l'accès : `always` passe toujours (même dans un lieu exclu) ; sinon un lieu exclu refuse progression et objets utiles ; sinon la liste décide.

Refus (le monde reste en Python, raison dans le journal) : la règle lève une exception, `always_allow` lit l'inventaire (`NoState`), ou la classe du lieu redéfinit `can_fill`.

### 2. Contrôle croisé des règles d'objet

Comme pour les règles d'accès (ADR 0028, § 4) : le worker tire 256 paires (lieu vide, exemplaire du pool) avec une graine fixe, trois sur quatre parmi les lieux qui ont une règle, et note `Location.can_fill(state, item, False)` du vrai Archipelago. Rust compare avec `can_hold` ; au moindre désaccord, ou si Python plante, le monde reste en Python. Ce contrôle vérifie la composition (exclusion, `always_allow`, `non_local_items`) et, en partie, qu'un exemplaire d'une sorte se comporte comme les autres.

### 3. Placement (`fill`)

- Chaque pose vérifie `can_hold` : progression (placement supposé), lieux exclus, utiles et remplissage (premier lieu restant qui accepte l'objet), échanges de l'équilibrage (les deux objets doivent accepter leur nouveau lieu). Sans règle d'objet, le résultat est **identique** à avant (mêmes tirages, mêmes lieux).
- Un objet qu'aucun lieu restant n'accepte (`NoAllowedLocation`, `NoAllowedItem` pour un lieu exclu) fait recommencer le placement, comme une impasse du placement supposé. Archipelago, lui, échange des objets déjà posés (`remaining_fill`) ; Raftup ne le fait pas : s'il échoue 20 fois, la génération repasse au Python.

### 4. Objets à placer tôt

`fill::fill_with_early` (`fill` garde sa signature) reçoit les `EarlyItem` (objet, nombre, local). Comme `distribute_early_items` :

1. les lieux de la sphère 0 = lieux vides atteignables avec l'inventaire de départ et les événements atteints ;
2. les exemplaires sont pris dans les paquets mélangés (les locaux d'abord) ;
3. les objets qui ne sont pas de progression d'abord, dans les lieux de la sphère 0 non prioritaires, les locaux chez leur joueur puis les autres ; ensuite la progression, prioritaires compris ;
4. ce qui n'a pas de place retourne dans son paquet et sera placé plus loin (`FillReport::early_not_placed`), comme Archipelago (qui l'écrit en avertissement).

Écart assumé : Archipelago remplit la sphère 0 avec son `fill_restrictive` (avec échanges) ; Raftup prend le premier lieu de la sphère 0 qui accepte l'objet. Tous ces lieux sont atteignables dès le départ, donc la logique n'en dépend pas. Si les objets tôt prennent toute la sphère 0 et qu'il n'y reste rien pour la progression nécessaire, Raftup échoue (repli Python) là où Archipelago peut s'en sortir par un échange.

### 5. Accessibilité

`logic::Accessibility` (`Full`, `Items`, `Minimal`) par joueur, lue dans l'export (0, 1, 2).

- **Fin du placement** : `MultiWorld::must_reach(lieu)` reprend `fulfills_accessibility` : un lieu doit être atteint si son joueur est en `Full`, ou s'il garde un objet de progression d'un joueur qui n'est pas en `Minimal`. Les victoires sont toujours exigées. `explain` (donc la faisabilité avant, la vérification après et l'équilibrage) ne regarde plus que ces lieux.
- **Progression d'un joueur `Minimal`** : comme `fill_restrictive`, si toutes les victoires sont atteintes sans l'objet, il peut aller dans un lieu hors d'atteinte, sauf chez un joueur en `Full`.
- **Lieux restés fermés** : comme `inaccessible_location_rules`, ils ne reçoivent pas d'objet important (progression, utile) d'un joueur qui n'est pas en `Minimal`.
- `accessibility_corrections` n'a pas d'équivalent : le placement supposé de Raftup ne pose jamais la progression d'un joueur `Full` hors d'atteinte, il n'y a rien à corriger.

### 6. Format de l'export

`format_version` passe à **2** (le champ `default_item_rule` de la version 1 disparaît ; une version 1 aurait laissé passer une règle d'objet sans la lire). Nouveaux champs : `item_rules` (racine), `item_rule` (lieu), `accessibility`, `early_items`, `local_early_items`, `item_rule_error` (joueur), `checks.item_checks`.

## Conséquences

- Les parties à plusieurs avec `local_items` / `non_local_items`, les mondes qui posent des objets tôt ou des règles d'objet, et les joueurs en `accessibility: minimal` passent par le placement Raftup au lieu du repli.
- Le moteur Rust seul (APQuest porté) ne change pas : il refuse toujours `accessibility: minimal` et `local_items` dans les YAML (`generate::yaml`) ; c'est le moteur « export » qui les prend.
- Une règle d'objet qui dépend d'autre chose que la sorte de l'objet (un exemplaire précis, l'inventaire) n'est pas exacte ; le contrôle croisé en attrape une partie, pas tout.
- Le calcul coûte une évaluation par (règle distincte × sorte d'objet du pool), en Python : négligeable sur les mondes essayés, à mesurer sur un très gros multiworld.

## Mise en œuvre (19 novembre 2026)

- `logic` : `ItemRule`, `ItemRuleId`, `Accessibility`, `add_item_rule`, `set_item_rule`, `set_accessibility`, `can_hold`, `must_reach`.
- `fill` : `fill_with_early`, `EarlyItem`, `FillReport::early_not_placed`, `NoAllowedLocation`, `NoAllowedItem` ; `explain` filtré par `must_reach` ; `balance` respecte `can_hold`.
- `generate::imported` : lecture du format 2, contrôle croisé des règles d'objet ; `fill_imported` passe les objets tôt.
- `raftup_export.py` : calcul des règles d'objet, `item_checks`, `accessibility`, objets tôt. Le worker (`raftup_worker.py`) n'a pas changé.
- Monde de test `raftup_sonde` : options `early_items` et `item_rules`.
