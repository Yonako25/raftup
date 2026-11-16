# 0013 — Commandes `!` du chat

- Statut : accepté
- Date : 2026-11-16

## Contexte

Le serveur officiel accepte des commandes qui commencent par `!` dans le chat : `!help`, `!players`, `!hint`, `!release`… Les joueurs s'en servent tous les jours, et les mods ne les filtrent pas : la commande passe comme un message texte.

Raftup doit donner les mêmes réponses, avec les mêmes textes, pour qu'un joueur ne voie pas de différence. Le serveur officiel découpe les arguments avec `shlex` et corrige les fautes de frappe avec une recherche floue (`get_intended_text`, dans `Utils.py`).

Certaines commandes plantent le serveur officiel quand les arguments sont mauvais (une exception Python, donc un traceback). Raftup ne doit pas reproduire un plantage.

## Décision

### Commandes gérées côté client

`help`, `license`, `options`, `players`, `status`, `release`, `collect`, `countdown`, `remaining`, `missing`, `checked`, `alias`, `getitem`, `hint`, `hint_location`, `admin`.

Les textes des réponses sont identiques à ceux du serveur officiel.

### Découpage des arguments

- Découpage façon `shlex.split` en mode POSIX, sans gestion des commentaires.
- Si le découpage échoue (guillemets non fermés, par exemple), repli sur un découpage simple par espaces.
- Pour les commandes « brutes » (`admin`, `missing`, `checked`, `alias`, `getitem`, `hint`, `hint_location`) : l'argument reçoit tout ce qui suit le nom de la commande, sans le redécouper.

Exemple : `!alias "Mon Objet" Épée` donne les arguments `Mon Objet` et `Épée`. `!alias "Mon Objet` (guillemet non fermé) passe par le repli et donne `"Mon` et `Objet`.

### Recherche floue

Quand un nom ne correspond à rien exactement, on cherche le plus proche avec la distance de Damerau-Levenshtein non restreinte, calculée sur les minuscules.

- Score = `int(ratio * 100)`, où `ratio` = 1 - distance / longueur maximale (en caractères).
- Égalité exacte : score 1.01 (le plus haut).
- Tri stable, décroissant, on garde les 2 premiers.
- Textes : `Perfect Match`, `Case Insensitive Perfect Match`, `Close Match` (écart de plus de 5 points), `Only Option Match` (plus de 90 %), et les messages `Didn't find something that closely matches '{x}', did you mean '{y}'? ({n}% sure)` et `Too many close matches for '{x}', did you mean '{y}'? ({n}% sure)`.

Exemple : `!hint Épe` propose `Épée` avec un score de 75 %.

### Décisions propres à Raftup

- `!options` masque le mot de passe de la salle : il s'affiche en `*`, comme `server_password`. Ça répond à NF09 (aucun mot de passe dans les logs ni les réponses). Écart déclaré : le serveur officiel l'affiche en clair.
- `!license` affiche la notice de Raftup (double licence MIT / Apache-2.0), et non celle d'Archipelago.
- Erreur d'arguments qui fait planter le serveur officiel : Raftup répond par un texte court (par exemple « Argument manquant pour !hint ») et ne ferme pas la connexion.
- Les commandes serveur qui suivent `!admin` sont reportées à J3.

## Conséquences

- Un joueur qui tape `!help` ou `!hint` obtient la même réponse qu'avec le serveur officiel.
- Le découpage et la recherche floue sont des fonctions pures : elles ont des tests avec des vecteurs tirés du code Python.
- Écarts assumés : `!options` masqué, licence de Raftup, erreurs courtes à la place des tracebacks. Ils sont à reporter dans la section « Écarts connus » de `docs/etat-du-projet.md`.
- `!admin` reste incomplet jusqu'à J3 : les autres commandes serveur ne sont pas encore disponibles par cette voie.
