# 0055 — Sommes de compteurs dans les règles (format de monde natif, version 4)

- Statut : **accepté** le 24 novembre 2026 par l'utilisatrice (option A, « écrit comme en Python ») ; codé le même jour
- Date : 2026-11-24
- Prolonge l'ADR 0041 (format natif), l'ADR 0042 (conversion, § 5 : « somme d'exemplaires sur une liste » prévue pour une version plus haute), l'ADR 0028 (export et sondeur), l'ADR 0053 (lots de conversion) et l'ADR 0054 (version 3)

## Contexte

### Ce que font les mondes d'Archipelago

Beaucoup de règles Python **additionnent des compteurs** avant de comparer à un seuil :

| Jeu | Règle (simplifiée) |
| --- | --- |
| Overcooked! 2 | `state.count("Star") + state.count("Bonus Star") >= 30` |
| Starcraft 2 | `count(amélioration) + count_from_list(paquets) + … >= niveau` |
| Kingdom Hearts, Kingdom Hearts 2, Super Mario Land 2 | `count(x) * 2 + count(y) >= n`, sommes de `count` |
| ChecksFinder | `has_from_list(["Map Width", "Map Height", "Map Bombs"], i + 1)` : somme des exemplaires d'une liste qui n'est pas un groupe déclaré |
| Yu-Gi-Oh! 2006 | `count_from_list_unique([4 à 20 cartes]) >= n` : nombre de cartes **différentes** |

### Ce que fait Raftup aujourd'hui

- **Format natif (versions 1 à 3)** : seuls `has(objet, n)` et `has_group(groupe, n)` comptent. Un groupe est écrit dans le datapackage : en inventer un pour une liste changerait le datapackage du monde (et casserait l'aller-retour « même datapackage »).
- **`logic::Rule`** : `Has`, `HasGroup` (somme sur un groupe déclaré), `And`, `Or`… Pas de somme sur une liste libre, pas de poids.
- **Convertisseur** : `has_from_list` hors groupe déclaré est refusé (ChecksFinder) ; `has_from_list_unique` est déplié en « ou » de « et » de chaque combinaison, refusé au-delà de 256 (Yu-Gi-Oh! 2006 : « au moins 6 parmi 23 »).
- **Sondeur (lot 2, première moitié, faite)** : une règle qui calcule sur un compteur (`+`, `*`, `range(count)`) est traduite en demandant la valeur exacte de chaque compteur (« au moins 1 », « au moins 2 »… jusqu'au nombre d'exemplaires qui existent). C'est exact, mais le nombre de chemins est le **produit** des valeurs possibles : `count("Star") + count("Bonus Star") >= 30` avec 60 + 30 étoiles fait des milliers de chemins, au-delà du plafond, et le texte produit serait énorme.

Mesuré le 24 novembre 2026 sur les 21 jeux visés après la première moitié (`state.multiworld` et `prog_items` en lecture seule, méthodes de LogicMixin, `reachable_regions`, résultat non booléen, `count_from_list_unique`, plafonds réglables) : **6 jeux de plus se convertissent** (Castlevania 64, EarthBound, Subnautica, Timespinner, The Wind Waker, Wargroove). Restent bloqués par ce que cet ADR règle :

| Jeu | Blocage après la première moitié |
| --- | --- |
| ChecksFinder | `has_from_list` hors groupe déclaré |
| Yu-Gi-Oh! 2006 | « au moins 6 parmi 23 » : plus de 256 combinaisons |
| Overcooked! 2 | plus de 64 questions (somme `Star` + `Bonus Star` énumérée) |
| Starcraft 2 | plus de 4 096 chemins (sommes de `count` et `count_from_list`) |
| Kingdom Hearts, Kingdom Hearts 2 | plus de 4 096 chemins (`count("Puppy") * n`, sommes) |

Ne relèvent pas de cet ADR : Kirby's Dream Land 3 (« et » de 8 « ou » sans court-circuit, 65 000 termes), Lingo (règle non monotone, parcours des régions atteintes), Zillion (copie de `prog_items`), shapez et Super Mario Land 2 (causes du convertisseur qui étaient cachées derrière la règle opaque).

## Options

### A. Une somme pondérée écrite comme en Python, en version 4 (recommandée)

```toml
format_version = 4

[[locations]]
name = "Level 4-1"
rule = 'count("Star") + count("Bonus Star") >= 30'

[[locations]]
name = "Tile 9"
rule = 'count_from_list("Map Width", "Map Height", "Map Bombs") >= 4'

[[locations]]
name = "TD31 Special Summon C"
rule = 'count_unique("Aqua Spirit", "Rock Spirit", "Inferno", "Gigantes", "Silpheed") >= 5'

[[locations]]
name = "Boss"
rule = '2 * count("Big Gem") + count("Small Gem") >= 10'
```

- Une **somme** est une suite de termes séparés par `+` ; un terme est `count(objet)`, `count_from_list(a, b, …)` (tous les exemplaires de la liste), `count_unique(a, b, …)` (nombre d'objets différents de la liste possédés), `count_group(groupe)`, éventuellement précédé d'un poids entier positif `n *`.
- Une somme ne s'utilise que dans une comparaison `>=` ou `>` à un nombre (ou à `option(nom)`, comme `has`) : la règle reste **monotone** (avoir un objet de plus ne la rend jamais fausse), exigence du format (§ Types de la spec). `<`, `<=`, `==`, `!=` sur une somme sont refusés.
- `has_from_list` d'Archipelago s'écrit `count_from_list(…) >= n`, `has_from_list_unique` s'écrit `count_unique(…) >= n` : plus de dépliage en combinaisons.
- Le texte se lit comme le Python d'origine ; l'export en apworld écrit `state.count(…, player) + 2 * state.count(…, player) >= n` sans rien inventer.

### B. Des fonctions seulement, sans `+` ni `*`

`has_from_list(a, b, c, 4)`, `has_from_list_unique(a, b, c, 2)`, `has_weighted(10, "Big Gem", 2, "Small Gem", 1)`. Plus simple à analyser (aucun nouvel opérateur), mais les poids en paires « objet, nombre » se lisent mal et se trompent facilement ; une somme de deux `count` simples (Overcooked! 2) devient `has_weighted(30, "Star", 1, "Bonus Star", 1)`.

### C. Ne rien ajouter au format

Le sondeur garde l'énumération (première moitié du lot) avec des plafonds relevés. Exact, mais Overcooked! 2, Starcraft 2 et ChecksFinder restent refusés, et les règles converties sont des « ou » de centaines de « et ».

## Décision (option A)

### 1. Format, version 4

- `FORMAT_VERSION` passe à 4 ; une somme écrite dans un fichier de version 1 à 3 est refusée avec un message qui cite la version 4.
- Grammaire (seul `comparison` change) :

```text
comparison = sum ( ">=" | ">" ) count_value
           | primary [ op value ]
sum        = term { "+" term }
term       = [ number "*" ] counter
counter    = "count" "(" objet ")" | "count_group" "(" groupe ")"
           | "count_from_list" "(" objet { "," objet } ")"
           | "count_unique" "(" objet { "," objet } ")"
count_value = number | "option" "(" nom ")"
```

- Poids : entier de 1 à 1 000 ; 256 objets cités au plus par somme, un groupe comptant pour un (bornes contre un texte hostile). Une liste peut citer un objet jamais créé (il compte 0, comme en Python) ; un objet cité deux fois dans `count_from_list` ou `count_unique` compte deux fois (comme `count_from_list` et `count_from_list_unique` d'Archipelago, qui parcourent la liste).
- `count(…)` seul reste interdit hors comparaison ; `has(objet, n)` et `has_group(groupe, n)` ne changent pas.

### 2. Logique

- Nouvelle variante `logic::Rule::Sum { terms, at_least }` ; chaque terme est `(objet, poids, unique)` (`unique` : l'objet compte au plus 1). `count_group` et les listes sont dépliés en termes à la compilation (le groupe est connu).
- Évaluation sans allocation, avec `saturating_add` comme `HasGroup` ; `walk`, `validate` et l'explication du placement (`fill::explain`) la connaissent.
- Une somme d'un seul terme `count(objet)` de poids 1 se compile en `Has` (même résultat, arbre plus simple).

### 3. Sondeur et export (ADR 0028)

- Le sondeur garde les sommes **symboliques** : `count(a) + 2 * count(b)` devient un objet « somme » ; sa comparaison à un nombre pose **une seule question** `sum`. Un calcul qui sort du linéaire (`range(count)`, flottant non entier, division, `max`) retombe sur l'énumération de la première moitié du lot.
- Nouvelle règle de l'export `{"t": "sum", "player", "terms": [[objet, poids, unique]], "count": n}` ; `format_version` de l'export passe à 3. Le moteur « export » (`generate::imported`) la traduit en `Rule::Sum` : plus de groupes inventés pour `has_from_list`.
- Le contrôle croisé (64 inventaires) vérifie ces règles comme les autres.

### 4. Convertisseur et apworld

- Le convertisseur écrit la plus petite version qui suffit : 4 seulement si une somme apparaît ; les 29 mondes déjà convertis gardent leur version.
- `has_from_list` hors groupe déclaré → `count_from_list(…) >= n` ; `has_from_list_unique` → `count_unique(…) >= n` (plus de plafond à 256 combinaisons).
- L'export en apworld écrit les sommes en Python (`state.count`, `state.count_from_list`, `state.count_from_list_unique`, `state.count_group`).

### 5. Ce qui ne change pas

Un fichier de version 1 à 3 est lu exactement comme avant. Un Raftup plus ancien refuse un fichier de version 4 (« format_version inconnu ») : voulu, il ne saurait pas évaluer la somme. Pas de soustraction, de produit de deux compteurs, de flottants ni de `max` : ces règles restent à l'énumération du sondeur, ou au portage à la main.

## Conséquences

- En bien : Overcooked! 2, Starcraft 2, ChecksFinder et Yu-Gi-Oh! 2006 deviennent convertibles (à mesurer), les textes convertis restent courts et se lisent comme le Python d'origine ; le moteur « export » n'invente plus de groupes.
- En moins bien : le mini-langage gagne deux opérateurs (`+`, `*`) et quatre fonctions ; la spécification (`spec/monde-natif/`), le lecteur, l'export en apworld et `logic` changent ensemble ; une version de format de plus à tester.

## Plan de travail, une fois validé

1. `logic` : `Rule::Sum`, évaluation, validation, explication ; tests.
2. `worlds-native` : grammaire, version 4, compilation, export en apworld ; tests de lecture et d'erreurs ; spec à jour.
3. Sondeur : somme symbolique et question `sum` ; tests Python ; export en version 3.
4. `generate` : import (`Rule::Sum`) et convertisseur ; tests de conversion.
5. Inventaire relancé, `docs/inventaire-conversion.md` à jour.
