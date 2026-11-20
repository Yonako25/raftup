# 0042 — Conversion apworld ↔ monde natif (V3, phase B)

- Statut : **accepté en autonomie le 20 novembre 2026, à relire par l'utilisatrice**
- Date : 2026-11-20
- Prolonge l'ADR 0041 (format de monde natif) et l'ADR 0028 (export des mondes Python).

## Contexte

- La SPEC (V3) demande un « outil de conversion assistée apworld → natif, avec export inverse en apworld ». Règle du projet : aucun monde exclusif à Raftup ; un monde natif doit donc pouvoir redevenir un apworld.
- On sait déjà décrire un apworld en données : l'export de l'ADR 0028 (`raftup_export.py`, lu par `generate::imported`) donne régions, entrées, lieux, pool, inventaire de départ et règles traduites (par `rule_builder` ou par sondage des lambdas), avec un contrôle croisé contre le vrai Archipelago.
- Mais cet export est pris **après** les options : il décrit le monde pour **un seul** jeu d'options. Le format natif, lui, décrit tous les cas avec `when` et des listes de cas.
- L'export perd aussi deux choses : l'**ordre de création** des éléments (il parcourt région par région), qui décide du placement, et la **façon de remplir** le pool (un monde qui tire ses pièges au hasard ne le dit pas).
- Il ne contient ni les options du monde, ni le `slot_data` (calculé après le placement).

## Décision

### 1. Une conversion = un jeu d'options, et on le dit

- Le convertisseur part d'**un** export (un joueur) et écrit un monde natif fidèle **pour ce jeu d'options-là** : un instantané.
- On **ne fusionne pas** plusieurs exports pour deviner les `when`. Deviner la condition qui explique qu'un lieu apparaisse dans certains exports et pas dans d'autres, c'est de l'apprentissage de fonction booléenne : faux dès que deux options jouent ensemble, et impossible à vérifier. C'est le travail de l'auteur (conversion **assistée**).
- Pour l'aider, la sortie est **déterministe** (même export ⇒ même texte, ordre de création) : deux conversions avec des options différentes se comparent avec n'importe quel outil de diff, et les différences sont exactement ce qui doit recevoir un `when`.
- Les options du monde sont écrites **en commentaire** dans le fichier (type, bornes, choix, valeur de l'instantané), pas déclarées : un YAML qui règle une option du jeu est refusé avec un message clair, au lieu de donner sans rien dire le monde de l'instantané. L'auteur décommente une option quand il a écrit les `when` qui vont avec.
- L'en-tête du fichier dit d'où il vient (jeu, version d'Archipelago, valeurs des options) et liste les **points à vérifier** : tout ce que le convertisseur a dû supposer.

### 2. L'export gagne trois champs facultatifs (toujours `format_version` 2)

- `n` sur chaque entrée et chaque lieu, `region_n` par joueur : **ordre de création**, noté par un petit crochet du worker sur `Region`, `Entrance` et `Location` (attribut `_raftup_n`). Le convertisseur écrit les éléments dans cet ordre, celui du code de l'apworld, donc celui d'un portage à la main.
- `options` par joueur : les options propres au monde (hors options communes), avec type, bornes, choix, alias, défaut et valeur. Lecture seule : rien n'est créé ni tiré au hasard.
- `generate::imported` ignore ces champs ; un export sans eux reste lisible (ordre du fichier).

### 3. Le convertisseur (`generate::convert`, binaire `raftup-monde`)

- Entrées : l'export JSON, le numéro du joueur, et, en option, la seed `.archipelago` de la même génération pour en reprendre le `slot_data` (écrit en valeurs littérales).
- Sorties : le texte du `world.toml`, relu par `worlds_native::parse_world` avant d'être rendu, et la liste des points à vérifier.
- Règles : `has` (nombre), `has_all`, `has_any`, `has_all_counts` et `has_any_count` (dépliés en `and` / `or` de `has`), `can_reach_*`, `and`, `or`, `true`, `false` se traduisent tels quels. `at_least` et `has_from_list_unique` deviennent un `or` de `and` sur les combinaisons (256 au plus, comme l'import). `has_from_list` (somme d'exemplaires) n'existe pas dans le format 1 : traduit en `has_group` seulement si la liste est exactement un groupe d'objets déclaré, sinon refusé.
- Pool : dans l'ordre de l'export, copies consécutives regroupées (`count`). La fin du pool, si elle n'est faite que de copies d'un même objet de remplissage, devient `[filler]` ; point à vérifier : « le monde tire-t-il des pièges au hasard ? » (ajouter `trap` et `trap_percent`).
- Refusés (erreur claire, rien d'écrit) : règle opaque, `collect` redéfini, objet verrouillé qui n'est pas un événement (lieu avec numéro), objet d'événement posé à deux endroits, noms en double.
- Signalés sans être convertis (points à vérifier) : règles d'objet (`item_rule`, qui peuvent aussi venir du YAML : `local_items`), objets à placer tôt (`early_items`, souvent du YAML), entrée reliée à rien (ignorée), classification d'un objet absent du pool (écrit `filler`), drapeaux de classification inconnus.

### 4. L'export inverse (`worlds_native::apworld`, même binaire)

- À partir d'un monde natif, on écrit un apworld Python minimal : `__init__.py` (la classe du monde), `items.py`, `locations.py`, `regions.py`, `rules.py`, `options.py` et `archipelago.json`. L'appelant le zippe en `.apworld`.
- Règles en **lambdas** Python (`state.has`, `state.has_all`, `state.has_group`, `state.can_reach_region`…), pas en `rule_builder` : elles marchent avec toutes les versions 0.6, et le sondeur de l'ADR 0028 sait les relire (aller-retour possible). Les parties statiques (`option(...)`) lisent `world.options` au moment de l'appel.
- `when` et listes de cas deviennent des `if` Python dans `create_regions` / `create_items` ; le remplissage tire ses pièges avec `self.random` (Archipelago ne suit pas le flux de Raftup : la seed d'Archipelago n'est pas celle de Raftup, et n'a pas à l'être).
- Texte déterministe ; noms échappés comme des chaînes Python (`repr` à la main), jamais du code venu du fichier : un monde natif hostile ne peut pas injecter de Python.
- Vérifié contre le vrai Archipelago 0.6.8 (test `#[ignore]`) : même datapackage que le monde natif (noms, numéros, groupes, `checksum`), et seed générée puis relue.

### 5. Pas de `format_version` 2 pour l'instant

Ce qui manque au format 1, et ce qu'on fait sans :

| Manque | Aujourd'hui |
| --- | --- |
| Règles d'objet (`item_rule`, lieux qui refusent certains objets) | Signalé, non converti. Les `local_items` / `non_local_items` du YAML restent gérés par `generate`, hors du monde. |
| Objets à placer tôt (`early_items`) | Option du YAML, pas du monde : hors format. |
| `accessibility` | Option commune du YAML : hors format. |
| Objet verrouillé avec numéro (pas un événement) | Refusé. |
| Somme d'exemplaires sur une liste (`has_from_list`) hors groupe déclaré | Refusé. |
| Entrées mélangées, remplissages pondérés, noms spéciaux des `range` | Non converti (les noms spéciaux sont recopiés en commentaire). |

APQuest et Slime Rancher se convertissent sans rien de tout cela. On ouvrira une `format_version` 2 quand un vrai apworld à convertir en aura besoin, en commençant par les règles d'objet propres au monde et `has_from_list` ; d'ici là, un manque donne un refus ou un point à vérifier, jamais un monde faux.

### 6. Slime Rancher : converti seulement sur le PC

L'apworld Slime Rancher n'a **pas de licence** (ADR 0021). Sa version native serait une œuvre dérivée : elle **n'entre pas dans le dépôt**. Un test `#[ignore]` la produit sur le PC à partir de l'apworld installée (`RAFTUP_SLIME_APWORLD`), la relit et la compile ; rien n'est écrit dans le dépôt.

## Conséquences

- Un apworld à règles simples devient un monde natif en une commande, plus une relecture guidée par la liste des points à vérifier.
- Le monde converti d'un seul jeu d'options n'est complet que pour ce jeu d'options ; tant que l'auteur n'a pas écrit les `when`, le YAML ne peut pas régler les options du jeu.
- Tout monde natif peut redevenir un apworld : la règle « aucun monde exclusif » est tenue par un outil, pas par une promesse.
- Le worker grossit un peu (crochet d'ordre de création) ; il reste sous la limite de 16 Ko de `python -c`.
- Les retouches que l'outil ne peut pas deviner (tirage des pièges, options) restent à la main.
