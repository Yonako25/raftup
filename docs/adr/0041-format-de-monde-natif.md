# 0041 — Format de monde natif déclaratif (V3, phase A)

- Statut : **accepté en autonomie le 20 novembre 2026, à relire par l'utilisatrice**
- Date : 2026-11-20

## Contexte

- La SPEC (V3) demande un « format de world natif déclaratif (TOML + mini-langage de règles), spécifié dans un dépôt séparé et versionné », puis (phase B) une conversion assistée apworld → natif avec export inverse en apworld.
- Aujourd'hui, un monde du moteur Rust est soit **porté à la main en Rust** (`generate::worlds::apquest`), soit **exporté depuis un apworld Python** (ADR 0028, `generate::imported`). Les deux remplissent les mêmes structures de `crates/logic` : régions, entrées, lieux, sortes d'objets, groupes, règles (`logic::Rule`).
- Écrire un monde en Rust demande de recompiler Raftup ; l'export demande Python. Un format déclaratif permet d'écrire un monde dans un simple fichier, lu sans exécuter de code.
- Règle du projet : aucun monde exclusif à Raftup. Le format doit donc rester traduisible vers un apworld (phase B) : il ne doit rien exprimer qu'Archipelago ne sache pas faire.

## Décision

### 1. Un fichier TOML versionné, compilé vers `logic`

- Un monde natif = un fichier TOML (`format_version = 1`), éventuellement dans un dossier avec d'autres fichiers plus tard (lu par `world.toml`).
- Sections : `[game]` (nom, région de départ, objectif), `[[options]]`, `[[items]]`, `[[item_groups]]`, `[[regions]]`, `[[entrances]]`, `[[locations]]`, `[[location_groups]]`, `[[events]]`, `[[pool]]`, `[filler]`, `[[start]]`, `[[slot_data]]`.
- Tout est écrit en **listes ordonnées** (`[[…]]`) : l'ordre du fichier est l'ordre de création dans `logic`, donc la seed est reproductible et un portage peut donner exactement les mêmes octets.
- **Aucun nouveau moteur** : la crate `worlds-native` lit, valide et **compile** le fichier vers un `logic::MultiWorld` (plus le pool, l'inventaire de départ et le `slot_data`), comme le portage Rust d'APQuest. Le placement reste celui de `fill`.
- Ce qui dépend des options est réglé **à la compilation** : un élément peut porter `when = "<condition>"`, une valeur peut changer selon les options (liste de cas `{ when, value }`, le premier qui convient gagne). Les règles de `logic` ne connaissent pas les options (ADR 0027, § 2).

### 2. Mini-langage de règles

- Grammaire (précédence : `not` > `and` > `or`, parenthèses) :
  `has(objet[, n])`, `has_count(objet, n)`, `has_all(a, b, …)`, `has_any(a, b, …)`, `has_group(groupe[, n])`, `can_reach(nom)`, `can_reach_region(nom)`, `can_reach_location(nom)`, `can_reach_entrance(nom)`, `option(nom)`, comparaisons `option(nom) == valeur` (et `!=`, `<`, `<=`, `>`, `>=`), `true`, `false`.
- Noms : mot simple (`Sword`) ou chaîne entre guillemets (`"Health Upgrade"`).
- Deux types : **statique** (ne dépend que des options : `option`, comparaisons, `true`, `false`) et **dynamique** (dépend de l'inventaire). `not` ne s'applique qu'au statique, comme dans `logic` qui n'a pas de « non » (une règle ne fait que devenir vraie quand l'inventaire grandit). Un `when` doit être statique.
- Une erreur donne la **ligne et la colonne dans le fichier** (exactes pour une chaîne littérale `'…'`, recommandée pour les règles ; approchées si la chaîne contient des échappements).
- Compilation vers `logic::Rule`, avec simplification des constantes : `has(Sword) and (not option(hard_mode) or has(Shield))` devient `Has(Sword)` sans le mode difficile. `has_all` donne `HasAll`, `has_any` `HasAny`, comme le portage.
- `can_reach` vers une région ou un lieu absent avec ces options vaut `false`.

### 3. Validation au chargement, sans panique

- Lecture bornée : fichier de 4 Mio au plus, 100 000 éléments par liste, règle de 16 Kio, profondeur 64, nom de 200 caractères.
- Refusé : `format_version` inconnu, champ inconnu (`deny_unknown_fields`), nom en double (les objets d'événement partagent l'espace des objets, les lieux d'événement celui des lieux), numéro en double ou nul, référence inconnue, règle mal typée, région inaccessible depuis le départ (graphe de toutes les entrées, sans règles), valeur par défaut hors bornes, option qui porte le nom d'une option commune à tous les jeux (`progression_balancing`, `accessibility`, `local_items`…).
- À la compilation (options connues) : élément présent qui pointe vers un élément absent, région présente mais inaccessible, plus d'objets que de lieux.

### 4. Datapackage

- Objets et lieux du datapackage : tous ceux qui ont un numéro (`id`), **quelles que soient les options**, rangés par numéro croissant ; groupes déclarés. Le `checksum` est calculé par `generate` comme pour les autres mondes.

### 5. Spécification dans `spec/monde-natif/`, pas dans un dépôt séparé

- La SPEC parle d'un dépôt séparé ; rien ne sort du PC sans l'utilisatrice. La spécification est donc écrite dans `spec/monde-natif/` (README + exemples), **sans dépendance au reste du dépôt**, prête à être déplacée telle quelle dans son propre dépôt (licence MIT OR Apache-2.0).

### 6. APQuest natif, identique au portage

- `spec/monde-natif/exemples/apquest/world.toml` réécrit APQuest. Avec la même seed et les mêmes options, `generate` donne **le même `.archipelago` à l'octet près** que le portage Rust (test d'empreinte `generate/tests/natif.rs`, toutes les combinaisons d'options booléennes comparées structure par structure).
- `generate::GameOptions::Native` porte un monde natif et ses valeurs d'options ; la lecture des YAML sait lire les options d'un monde natif (même tirage au hasard que le portage).
- Un monde natif passé explicitement à la lecture des YAML **l'emporte** sur un monde porté en Rust du même nom.

### 7. Pas encore dans le format 1 (gardé pour la suite)

- Règles d'objet (`item_rule`, `local_items`…), objets à placer tôt, `accessibility`, entrées aléatoires, plusieurs remplissages pondérés, noms spéciaux des options numériques. La phase B les ajoutera si la conversion d'apworlds en a besoin (`format_version` 2).

## Conséquences

- Un monde s'écrit sans Rust ni Python, et se lit comme une donnée hostile (aucun code exécuté).
- Le portage Rust d'APQuest reste la référence ; le test d'empreinte garde les deux identiques. À terme, le portage pourra être retiré au profit du fichier.
- Le langage est volontairement petit : tout ce qu'il exprime existe dans `logic` et dans le `rule_builder` d'Archipelago, donc l'export natif → apworld (phase B) est possible.
- Les positions dans une chaîne TOML avec échappements sont approchées.
- Le launcher lit les mondes natifs posés dans `mondes-natifs/` (dossier des données de Raftup : fichiers `.toml` ou dossiers avec un `world.toml`) quand il choisit le moteur Rust ; un monde illisible est écarté (journal), deux mondes du même jeu : le premier par ordre alphabétique gagne. La Bibliothèque ne les affiche pas encore, et rien ne les installe : on les copie à la main.
