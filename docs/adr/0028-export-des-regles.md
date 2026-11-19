# 0028 — Export des mondes Python vers le moteur Rust (V2, étape 6)

- Statut : **accepté** le 19 novembre 2026 (les 7 décisions du bas, telles que recommandées)
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

## Décision

### 1. Deux appels au worker Python, le placement entre les deux en Rust

Dans le même process worker (ADR 0016, 0017), pour une génération :

1. **`export_worlds`** : le worker crée le `MultiWorld` d'Archipelago avec les YAML, puis lance les étapes des mondes jusqu'à `pre_fill` compris (`generate_early`, `create_regions`, `create_items`, `set_rules`, `connect_entrances`, `generate_basic`, `pre_fill`). Il **n'appelle pas** le placement d'Archipelago. Il écrit la description de chaque monde (section 2) dans un fichier JSON.
2. **Placement en Rust** : le JSON devient un `logic::MultiWorld` (nouveau module `generate::imported`), avec les mondes portés en Rust à côté s'il y en a ; puis `fill` place et équilibre.
3. **`finish_worlds`** : Rust renvoie le placement (lieu → objet). Le worker pose les objets dans son `MultiWorld`, puis laisse Archipelago **finir sa génération** comme d'habitude (`post_fill`, `fill_slot_data`, indices, `generate_output`, spoiler) ; son propre équilibrage est désactivé, Raftup l'a déjà fait.
4. **Archipelago écrit la seed** (`AP_<seed>.zip`), exactement comme le générateur officiel : le serveur et les mods ne voient aucune différence. *(Révisé le 19 novembre 2026, validé par l'utilisatrice : au départ, Rust devait écrire le `.archipelago` ; laisser Archipelago l'écrire garde exact tout ce qui dépend du placement, pour beaucoup moins de code.)*

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

## Décisions validées (19 novembre 2026)

| # | Question | Choix |
| --- | --- | --- |
| 1 | Export en deux appels (`export_worlds`, puis `finish_worlds` avec le placement Rust) dans le même worker | Oui |
| 2 | Lambdas traduites par **sondage** (arbre de décision), avec les plafonds et les cas de refus de la section 3 | Oui |
| 3 | Une règle opaque ⇒ tout le monde reste au générateur Python (pas d'évaluation Python pendant le placement) | Oui |
| 4 | Contrôle croisé de chaque règle sondée sur 64 inventaires, à chaque export | Oui |
| 5 | Export transmis par fichier JSON (64 Mio au plus) plutôt que par une ligne de `py-bridge` | Fichier |
| 6 | `item_rule` non pris en charge au départ ; `progress_type` (exclu, prioritaire) ajouté | Oui |
| 7 | Ordre des étapes 1 à 5 ci-dessus, APQuest exporté comparé au portage Rust en premier | Oui |

## Mise en œuvre de l'étape 1 (19 novembre 2026)

- **Arrêt avant le placement** : le worker remplace `Main.distribute_items_restrictive` (et `flood_items`) par une fonction qui lève une exception portant le `MultiWorld`. Toutes les étapes d'Archipelago jusqu'à `pre_fill` tournent donc exactement comme dans le générateur officiel (inventaire de départ, `local_items`, exclusions, liens d'objets…), sans rien recopier de `Main.py`.
- **Module d'export** : `raftup_export.py` est envoyé en texte dans la requête `export_worlds`, puis exécuté par le worker. Le worker passe par `python -c`, dont la ligne de commande est limitée : il reste sous 16 Ko (test `test_taille_script`). Nouvelle erreur `-32013` (export échoué).
- **Règles `rule_builder`** : reconnues par **type exact** de la classe `Resolved` ; une règle propre à un monde (sous-classe) reste opaque. `HasFromList` et `HasGroup` (somme des exemplaires) deviennent un groupe d'objets anonyme (`raftup:liste N`). `HasFromListUnique`, `HasGroupUnique` et `AtLeast` deviennent un `Or` de `And` sur les combinaisons, 256 au plus ; au-delà, export refusé. `logic` gagne seulement `set_entrance_rule` et `set_location_rule`, pour poser les règles une fois tout créé.
- **Datapackage** : le `checksum` est recalculé par Raftup et comparé à celui d'Archipelago à chaque import. Un écart refuse l'export.
- **Vérifié en vrai** (`launcher-core/tests/export_real.rs`, Archipelago 0.6.8) : APQuest exporté est identique au portage Rust pour les 16 combinaisons d'options et pour 2 joueurs. Sont comparés : régions, entrées, lieux, numéros, événements, pool, inventaire de départ, datapackage, et ce qui est atteignable pour 300 inventaires. Aucune règle opaque.
- **Point ouvert, reproductibilité (§ 7)** : `PYTHONHASHSEED=0` est sans effet, parce que le worker est lancé avec `-I`, qui ignore les variables `PYTHON*` (ADR 0016). C'est à régler à l'étape 4.
- **À décider à l'étape 4** : au lieu de faire écrire le `.archipelago` par Rust (§ 1, point 4), le worker pourrait reprendre `Main.main` avec le placement de Rust, et laisser Archipelago écrire lui-même la seed (indices, `slot_data`, fichiers de sortie). Ce serait plus fidèle et demanderait moins de code. Cela change le § 1 : à proposer avant de coder.

## Mise en œuvre de l'étape 2 (19 novembre 2026)

- **Sondeur** (`raftup_export.py`, `probe`) : un faux `CollectionState` (`ProbeState`) répond aux questions `has`, `has_all`, `has_any`, `has_all_counts`, `has_any_count`, `has_from_list(_unique)`, `has_group(_unique)` (groupe remplacé par sa liste d'objets), `count` comparé à un nombre (`>=`, `>`, `<`, `<=`, `==`), `can_reach` (nom ou objet) et `can_reach_region/location/entrance`. L'arbre est exploré en profondeur ; les termes vrais minimaux donnent la règle « ou » de « et ».
- **Monotonie** : vérifiée exactement. Chaque feuille fausse doit contredire chaque terme vrai par une de ses réponses « Faux ».
- **Refus** : méthode ou attribut inconnu (`state.prog_items`…), calcul sur un compteur, exception, résultat non booléen, chemin rejoué qui ne donne pas le même résultat, plus de 4 096 chemins ou de 64 questions.
- **Contrôle croisé** : 64 inventaires, graine fixe, densités de 5 % à 95 %, de 1 à 40 exemplaires. Le vrai `CollectionState` est évalué avec `prog_items` remplacé par l'inventaire. Le contrôle porte sur **toutes** les règles (entrées, lieux, conditions de victoire), pas seulement celles qui ont été sondées : il vérifie aussi la traduction de `rule_builder`. Rust refuse le monde au premier désaccord, ou si la règle Python plante (`'e'`).
- **`collect` personnalisé** : un monde qui redéfinit `collect`, `collect_item` ou `remove` (objets progressifs renommés, compteurs maison) est refusé. Le modèle de Raftup compte chaque objet par son nom.
- **Monde de test** `raftup_sonde` (`launcher-core/tests/data`, écrit pour Raftup) : 9 règles en lambdas, avec fonction d'aide, `count`, groupe, `can_reach` et liste. En vrai : 9 règles sondées, 0 opaque, contrôle croisé réussi, monde jouable. Avec l'option `bad_rule` (`not state.has(...)`), la règle est refusée (« non monotone ») et le monde reste au générateur Python.
- **Reste** : `checks` est facultatif dans le JSON, pour les exports écrits à la main des tests ; le branchement de l'étape 4 l'exigera.

## Mise en œuvre de l'étape 3 (19 novembre 2026)

- **Modèle** : `logic::Progress` (`Default`, `Priority`, `Excluded`) sur chaque lieu, avec `MultiWorld::set_progress`. L'import lit `progress_type` (1, 2, 3) ; une autre valeur rend l'export incohérent.
- **Placement** (d'après `distribute_items_restrictive` d'Archipelago 0.6.8) : la progression va dans le premier lieu **prioritaire** atteignable, sinon dans le premier lieu normal, jamais dans un lieu **exclu**. Les lieux exclus reçoivent ensuite du remplissage (ni progression ni utile), puis les utiles et le reste vont partout ailleurs. S'il y a moins de remplissage que de lieux exclus : `FillError::TooManyExcluded`, avant tout placement.
- **Écart avec Archipelago** : il fait d'abord une passe de placement réservée aux lieux prioritaires (avec ses objets « déprioritisés »), puis la passe normale. Raftup préfère simplement un lieu prioritaire à chaque objet de progression. Mesuré : au moins 9 lieux prioritaires sur 10 reçoivent de la progression dans le monde de test de `fill`.
- **Équilibrage** : comme Archipelago, un objet posé dans un lieu prioritaire n'est jamais avancé, et un lieu exclu ne reçoit jamais l'objet avancé.
- **Vérifié** : 300 seeds à 2 joueurs, sans aucune progression ni objet utile dans un lieu exclu, avant comme après l'équilibrage ; lieux prioritaires inchangés par l'équilibrage ; reproductible. En vrai, le monde `raftup_sonde` exporte un lieu prioritaire et un lieu exclu, retrouvés tels quels dans le monde Rust.

## Mise en œuvre de l'étape 4 (19 novembre 2026)

Deux choix validés par l'utilisatrice le 19 novembre 2026 : Archipelago écrit la seed (§ 1, révisé) ; `-s -P` au lieu de `-I`, avec `PYTHONHASHSEED=0` (ADR 0016, décision 4, révisée).

- **Worker** : `export_worlds` lance `Main.main` dans un thread. Le placement d'Archipelago (`distribute_items_restrictive`, `flood_items`) est remplacé par une attente : le `MultiWorld` est exporté, puis le thread attend. `finish_worlds` lit `placement.json` (`[joueur du lieu, lieu, joueur de l'objet, objet]`), pose chaque objet du pool (`push_item`), vérifie que tout est placé, puis laisse `Main.main` finir. `balance_multiworld_progression` est désactivé. Sans `finish_worlds` (arrêt du launcher), le thread meurt avec le process. Nouvelle erreur `-32014` (aucun export en cours).
- **Export** : en plus, `progression_balancing` de chaque joueur (après le `meta.yaml`, appliqué par Archipelago) et `early_items`. Un joueur avec des objets à placer tôt est refusé : c'est le placement d'Archipelago qui s'en occupe, et Raftup le remplace.
- **Placement** : `generate::imported::fill_imported` (placement, puis équilibrage), avec le flux ChaCha `fill` de la seed. La seed est tirée par Raftup si l'hôte n'en donne pas, et passée aux deux côtés.
- **Launcher** : `ExportBackend` (`launcher-core`). Si l'import ou le placement échoue (monde non pris en charge, contrôle croisé, placement impossible), la raison va dans le journal, et la génération repart avec le générateur Python seul, dans un worker neuf. Un export sans contrôle croisé est refusé. Dans l'interface, la case « Moteur Rust » choisit le moteur Rust seul s'il comprend tous les YAML, sinon ce moteur « export ».
- **Vérifié en vrai** : seed complète à 2 joueurs (`raftup_sonde` et APQuest) placée par Raftup, écrite par Archipelago, relue par `multidata` (14 lieux remplis). Même seed deux fois : mêmes placements et même `slot_data`. Le repli a lieu quand une règle est refusée. `version` du worker donne `hash_randomization = 0`.
- **Constat** : sur le petit monde de test (2 lieux ouverts au départ), le placement d'Archipelago échoue sur certaines seeds (`FillError`), alors que celui de Raftup réussit grâce à ses nouveaux essais.
- **Reste** : le moteur « export » n'a pas encore généré un vrai monde tiers (Slime Rancher, étape 5) ; le journal du launcher ne montre pas encore le moteur utilisé à la fin.

## Étape 5 : Slime Rancher (19 novembre 2026)

Apworld `slime_rancher` 0.3.2.1, installée sur le PC par la Bibliothèque, lue par le test réel (`RAFTUP_SLIME_APWORLD`), jamais copiée dans le dépôt.

- **Premier essai** : 109 règles sondées, 38 opaques. Cause unique : beaucoup de lambdas renvoient `a or b` avec une **valeur d'option** (un entier 0 ou 1) au lieu d'un booléen. Le sondeur lit maintenant la vérité du résultat, comme Archipelago (`if rule(state)`) : booléens, entiers, `None` et compteurs sont acceptés, et tout autre type reste refusé.
- **Ensuite** : 147 règles sondées, **0 opaque**. Le contrôle croisé contre le vrai Archipelago passe (64 inventaires). 25 régions, 32 entrées, 284 lieux, 152 objets dans le pool.
- **Génération complète** (seed 11), comparée à la même seed générée par Archipelago seul : même nom de seed, même datapackage, mêmes lieux, même pool d'objets, même inventaire de départ, **même `slot_data`**. Seul le placement diffère. Durée : 2,9 s, contre 2,8 s pour Archipelago seul.
- **Pas encore fait** : une vraie partie de Slime Rancher sur une seed placée par Raftup (il faut l'utilisatrice devant le jeu).

