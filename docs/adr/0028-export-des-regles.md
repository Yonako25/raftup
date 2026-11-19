# 0028 — Export des mondes Python vers le moteur Rust (V2, étape 6)

- Statut : **proposé** (à valider par l'utilisatrice, voir « Décisions à valider » en bas)
- Date : 2026-11-19
- Prolonge l'ADR 0027 (§ 7, étape 6) et la SPEC § 4 V2 : « export des règles depuis les apworlds : module de règles déclaratives d'abord, puis traçage des lambdas, sinon règle opaque ».

## Contexte

- Le moteur Rust (ADR 0027) ne génère qu'APQuest, porté à la main. On ne peut pas porter chaque monde à la main, et pour certains on n'en a pas le droit : l'apworld Slime Rancher n'a **pas de licence** (ADR 0021).
- Dans Archipelago 0.6.8, seuls 3 mondes sur environ 75 utilisent `rule_builder` (APQuest, Muse Dash, Pokémon Émeraude). Tous les autres écrivent leurs règles en **lambdas Python**.
- Constats sur l'apworld Slime Rancher 0.3.2.1 (environ 1 550 lignes, lu sur le PC, rien recopié) :
  - toutes ses règles sont des lambdas qui appellent de petites fonctions (`has`, `has_region`, `has_jetpack`, `has_energy`…) ; au fond, chacune ne fait que `state.has(objet, joueur, nombre)` et lire des options ;
  - il utilise aussi des lieux prioritaires ou exclus (`progress_type`), des objets verrouillés (événements) et un inventaire de départ (`push_precollected`) ;
  - `fill_slot_data` tire au hasard (`self.random`) et lit `multiworld.seed`.
- Il faut donc faire tourner le code de l'apworld (sur le PC du joueur, comme aujourd'hui) et en **extraire** une description que le moteur Rust sait remplir.

## Décision proposée

### 1. Deux appels au worker Python, le placement entre les deux en Rust

Dans le même process worker (ADR 0016, 0017), pour une génération :

1. **`export_worlds`** : le worker crée le `MultiWorld` d'Archipelago avec les YAML, puis lance les étapes des mondes jusqu'à `pre_fill` compris (`generate_early`, `create_regions`, `create_items`, `set_rules`, `connect_entrances`, `generate_basic`, `pre_fill`). Il **n'appelle pas** le placement d'Archipelago. Il écrit la description de chaque monde (section 2) dans un fichier JSON.
2. **Placement en Rust** : le JSON devient un `logic::MultiWorld` (nouveau module `generate::imported`), avec les mondes portés en Rust à côté s'il y en a ; puis `fill` place et équilibre.
3. **`finish_worlds`** : Rust renvoie le placement (lieu → objet). Le worker pose les objets dans son `MultiWorld`, puis appelle les étapes de fin des mondes (`post_fill`, `fill_slot_data`, `extend_hint_information`, `generate_output`). Il rend `slot_data`, `er_hint_data`, les versions minimales de client et les fichiers éventuellement produits.
4. Rust construit le `.archipelago` (ADR 0027, § 3) : le serveur et les mods ne voient aucune différence.

Ce chemin a toujours besoin de Python et d'Archipelago, mais Archipelago n'y place plus rien. On y gagne le placement Rust (faisabilité vérifiée, erreurs expliquées, équilibrage) et le mélange avec les mondes natifs dans une même partie. Se passer de Python pour ces mondes reste l'affaire de la V3 (format natif).

### 2. Ce que décrit l'export (JSON versionné, `format_version = 1`)

Pour chaque joueur :

- les régions, les entrées (origine, arrivée, règle), les lieux (nom, numéro, région, règle, `progress_type`), la région de départ ;
- les objets du pool (nom, numéro, classification), les objets déjà posés (événements et objets verrouillés), l'inventaire de départ ;
- la condition de victoire ;
- le datapackage du jeu (noms, numéros, groupes) ; Rust recalcule le `checksum` comme Archipelago ;
- `required_client_version`.

**Transport** : un fichier `export.json` dans le dossier de la génération, lu par Rust avec une taille maximale (64 Mio, comme une seed, NF08). On évite ainsi d'élargir la limite d'une ligne de `py-bridge` (1 Mio, ADR 0016). Le fichier est lu comme une donnée non fiable : noms bornés, index vérifiés, nombre d'éléments plafonné.

### 3. Les règles : trois façons, de la plus sûre à la moins sûre

1. **`rule_builder`** : la règle résolue sait se sérialiser (`to_dict`). On la traduit telle quelle ; c'est exact. Les règles de `rule_builder` absentes de `logic` (`AtLeast`, `HasAnyCount`, `HasFromList`, `HasFromListUnique`, `HasGroupUnique`) sont ajoutées à `logic` au fur et à mesure.
2. **Lambdas, par sondage** : le worker appelle la lambda avec un **faux inventaire** (`ProbeState`). Chaque question qu'elle pose (`has`, `has_all`, `has_any`, `count` comparé à un nombre, `has_group`, `can_reach`…) est notée, et le sondeur choisit la réponse. En essayant toutes les suites de réponses (exploration en profondeur), on obtient l'arbre de décision de la lambda, puis une règle `Or` de `And`.
   - Ça marche parce que les opérateurs `and` / `or` de Python s'arrêtent dès qu'ils connaissent la réponse : on suit exactement les chemins que la lambda emprunte.
   - On refuse (règle **opaque**) si : plus de 4 096 chemins ou 64 questions sur un chemin ; une méthode ou un attribut de l'inventaire inconnu du sondeur (`state.prog_items`, `state.multiworld`…) ; une réponse qui ne dépend pas que des questions (deux appels identiques, deux résultats) ; une règle **non monotone** (avoir un objet de plus rend la règle fausse : `logic` n'a pas de « non ») ; une exception.
3. **Opaque** : une seule règle opaque, et le monde reste au générateur Python (section 5). On n'évalue pas de règle Python pendant le placement Rust : il faudrait un aller-retour entre les deux process à chaque évaluation, beaucoup trop lent.

### 4. Vérification à chaque export

- **Contrôle croisé des règles** : pour chaque règle sondée, le worker l'évalue avec un **vrai** `CollectionState` sur 64 inventaires tirés au hasard (graine fixe), et Rust évalue la règle traduite sur les mêmes inventaires. Un seul désaccord, et le monde passe au Python, avec la règle en cause dans le journal.
- **Tests** : APQuest exporté depuis Python doit donner le même `MultiWorld` que le portage Rust (mêmes régions, lieux, règles, pool), pour toutes ses combinaisons d'options. C'est le test différentiel le plus direct dont on dispose.
- **Slime Rancher** : test réel `#[ignore]` (vrai Python, apworld installée par la Bibliothèque) ; les règles sondées passent le contrôle croisé ; la seed est jouable de bout en bout ; la seed est chargée par le serveur Raftup, et le mod Slimipelago s'y connecte sans modification.

### 5. Choix du moteur

- Le moteur Rust prend la génération si **chaque** joueur joue à un monde porté en Rust **ou** exportable (export réussi, aucune règle opaque, contrôle croisé réussi). Sinon toute la partie passe au générateur Python, comme aujourd'hui : un seul moteur par partie (ADR 0027, § 6).
- Le refus est visible : la raison (monde, règle, ou option en cause) va en tête du journal de génération.
- On ne sait qu'après l'export si un monde est exportable : quand l'export échoue, la génération repart en Python dans un process neuf, et le journal le dit.

### 6. Ce que le modèle `logic` / `fill` doit gagner

- `progress_type` des lieux : **exclu** (que du remplissage : ni progression ni utile, comme Archipelago) et **prioritaire** (progression d'abord) ;
- les règles de `rule_builder` qui manquent (section 3) ;
- les règles d'objet par lieu (`item_rule`) ne sont **pas** prises en charge au départ : un lieu dont `item_rule` n'est pas celle par défaut rend le monde non exportable (il reste en Python). Slime Rancher n'en a pas.

### 7. Reproductibilité

- Le worker est lancé avec `PYTHONHASHSEED=0` : l'ordre des `set` de chaînes ne change plus d'un lancement à l'autre.
- Le hasard des mondes Python vient d'Archipelago lui-même (`MultiWorld.set_seed` avec le numéro de seed) ; le placement vient du flux ChaCha de Raftup.
- Garantie : même seed, mêmes YAML, même version de Raftup, d'Archipelago **et** des apworlds ⇒ mêmes octets. La version de chaque apworld est écrite dans le spoiler.

### 8. Ordre des étapes

1. `export_worlds` sur APQuest : JSON écrit, puis relu en `logic::MultiWorld` et comparé au portage Rust (règles par `rule_builder`).
2. Sondeur de lambdas et contrôle croisé, testés sur des lambdas écrites pour l'occasion dans un monde de test du dépôt, pas sur du code tiers.
3. `progress_type` dans `logic` et `fill`.
4. `finish_worlds` et `.archipelago` complet ; branchement dans `RustBackend` (qui demande alors le runtime Python).
5. Slime Rancher en test réel, puis une vraie partie.

## Conséquences

- Slime Rancher et, on peut l'espérer, une bonne part des mondes à règles simples profitent du placement Rust, sans qu'une ligne de leur code entre dans le dépôt.
- Le Python embarqué reste nécessaire pour ces mondes ; seuls les mondes portés en Rust s'en passent.
- Le code de l'apworld tourne toujours sans bac à sable (même risque qu'aujourd'hui, réseau bloqué, ADR 0019) ; l'export ajoute une **lecture** de JSON côté Rust, à traiter comme une donnée hostile.
- Le sondeur dépend de l'API de `CollectionState` d'Archipelago : une méthode ajoutée dans une version future rend les règles qui l'utilisent opaques (le monde reste en Python). Pas de mauvaise seed, seulement un repli.
- Le passage par Python en deux temps rend la génération plus longue qu'en Python seul pour un petit monde (deux appels, un JSON). C'est à mesurer à l'étape 4.

## Décisions à valider

| # | Question | Recommandation |
| --- | --- | --- |
| 1 | Export en deux appels (`export_worlds`, puis `finish_worlds` avec le placement Rust) dans le même worker | Oui |
| 2 | Lambdas traduites par **sondage** (arbre de décision), avec les plafonds et les cas de refus de la section 3 | Oui |
| 3 | Une règle opaque ⇒ tout le monde reste au générateur Python (pas d'évaluation Python pendant le placement) | Oui |
| 4 | Contrôle croisé de chaque règle sondée sur 64 inventaires, à chaque export | Oui |
| 5 | Export transmis par fichier JSON (64 Mio au plus) plutôt que par une ligne de `py-bridge` | Fichier |
| 6 | `item_rule` non pris en charge au départ ; `progress_type` (exclu, prioritaire) ajouté | Oui |
| 7 | Ordre des étapes 1 à 5 ci-dessus, APQuest exporté comparé au portage Rust en premier | Oui |
