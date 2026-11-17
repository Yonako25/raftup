# Données de test du serveur

## `roominfo-seed-mit.json`

- Trame brute `RoomInfo` envoyée par le serveur officiel d'Archipelago 0.6.8 (`MultiServer.py --disable_save`) pour la seed `crates/multidata/tests/data/seed-mit.archipelago`.
- Capturée le 15 novembre 2026 avec un petit client Python `websockets` (script [`scripts/roominfo.py`](scripts/roominfo.py)).
- `tests/socle.rs` vérifie que Raftup envoie les mêmes valeurs. Seule l'heure (`time`) change d'un lancement à l'autre ; le test reprend celle de la capture.

## `pyset-vectors.json`

- Ordre de parcours d'un `set` Python d'indices, noté par CPython 3.12.10 avec `PYTHONHASHSEED=0`, le 17 novembre 2026 (script [`scripts/pyset_vectors.py`](scripts/pyset_vectors.py), graine 18).
- 31 suites d'opérations tirées au hasard (`add`, `discard`, reconstruction, `copy`, `update`, `-`, `set(liste)`), avec l'ordre après chaque opération et le `hash()` de chaque indice. Textes vides, latin-1, UCS-2 et UCS-4 ; indices de même hachage mais différents.
- `src/pyset.rs` (test `matches_cpython_vectors`) rejoue les opérations et compare (ADR 0018).

## `connexion-seed-mit.json`

- 16 scénarios de connexion rejoués sur le serveur officiel 0.6.8 avec la même seed, le 15 novembre 2026 (script [`scripts/connect_vectors.py`](scripts/connect_vectors.py)).
- Pour chaque étape : le client qui envoie, les paquets envoyés (`envoi`) et les trames brutes reçues par chaque client (`recu`). `<fermé>` signale une connexion fermée par le serveur.
- Couvre : connexion acceptée, inventaire de départ, `slot_data` refusé, `items_handling`, outil `TextOnly`, chaque refus de `ConnectionRefused`, `Connect` incomplet, reconnexion, changement de slot et groupe (le serveur officiel ferme la connexion), `GetDataPackage`.
- `tests/connexion.rs` rejoue les scénarios sur Raftup et compare les trames, sans tenir compte de l'ordre des clés ni de l'ordre des erreurs de `ConnectionRefused` (un ensemble en Python). L'ordre des indices `_read_hints_…`, lui, est comparé (ADR 0018).

## `partie-seed-mit.json`

- Une partie de 20 étapes rejouée sur un serveur officiel 0.6.8 neuf, avec la même seed, le 15 novembre 2026 (script [`scripts/game_vectors.py`](scripts/game_vectors.py)). Même format que `connexion-seed-mit.json`.
- Cinq connexions : Alice, Bob, Chloé, un outil `TextOnly` sur le slot de Bob, puis une seconde connexion de Bob.
- Couvre : lieux visités (`LocationChecks`, y compris en double ou inconnus), messages `ItemSend`, objets distribués (`ReceivedItems`, avec le groupe `Lien`), `RoomUpdate`, refus pour un outil `TextOnly`, `Sync`, `ConnectUpdate` (`items_handling` et tags), reconnexion.
- `tests/connexion.rs` trie aussi `checked_locations` de `RoomUpdate`, un ensemble en Python.

## `objectif-seed-mit.json`

- Même méthode que `partie-seed-mit.json` (script `game_vectors.py objectif`, serveur officiel neuf), le 15 novembre 2026.
- Couvre : `StatusUpdate` (refus pour un outil `TextOnly`, statut « prêt »), objectif atteint (`Goal`), `collect` et `release` automatiques (options `auto` de la seed), `collect` du groupe `Lien` quand ses deux membres ont fini, objectif définitif, message de fin d'équipe.

## `echanges-seed-mit.json`

- Même méthode (script `game_vectors.py echanges`, serveur officiel neuf), le 15 novembre 2026.
- Couvre : `LocationScouts` (lieux valides, mauvais type, lieu inconnu qui ferme la connexion), chat (`Say`, texte non imprimable, client non connecté), `Bounce` avec chaque opérateur (`legacy`, `or`, `and`), par tags, slots, jeux et équipes, liste mal formée, `Bounce` sans `data`.
- Les commandes `!` sont dans `commandes-seed-mit.json`.

## `commandes-seed-mit.json` et `indices-seed-mit.json`

- Même méthode (scripts `game_vectors.py commandes` et `game_vectors.py indices`, serveur officiel neuf), le 17 novembre 2026.
- `commandes` : `!help`, commande inconnue, `!` seul, texte ordinaire, `!status` (avec un tag), `!remaining`, `!checked`, `!missing` (avec un filtre), `!admin login` puis `!admin`. Pas de `!options` : l'officiel y tire le nombre d'étoiles au hasard.
- `indices` : `LocationScouts` avec `create_as_hint` 0, 1 et 2, `CreateHints` (statuts valides ou non, lieu inconnu, joueur inconnu), `UpdateHint` (statuts, mauvais types), indice qui passe à « trouvé », clés `_read_hints_…`. Le serveur officiel y ferme une connexion.
- Le masque de `!admin login` (4 à 16 étoiles au hasard chez l'officiel, toujours 8 chez Raftup) est ramené à 8 étoiles avant la comparaison.
- Après une fermeture, `game_vectors.py` n'envoie plus rien sur cette connexion et note `<fermé>` à chaque étape, comme les captures précédentes.
- Le rejeu de `commandes` relève la limite de chat de Raftup (5 messages / 10 s, ADR 0007) : le scénario en envoie davantage.

## `stockage-seed-mit.json`

- Même méthode (script `game_vectors.py stockage`, serveur officiel neuf), le 15 novembre 2026.
- Couvre : `Get` (clés libres, clés `_read_…` : mode course, statut, `slot_data`, groupes d'objets et de lieux, indices, clés inconnues ou mal écrites, champ libre renvoyé), `Set` avec les opérations numériques, de texte, de liste et de dictionnaire, `want_reply`, `default`, `SetNotify`, refus (`_read_…`, listes mal formées), notification d'un changement de statut, indices qui passent à « trouvé » après une visite, opération impossible qui ferme la connexion.
- Particularité reproduite : `update`, `remove` et `pop` modifient la valeur sur place ; si elle venait de `default`, le `default` renvoyé dans le `SetReply` est modifié aussi.

## Régénérer les captures

Il faut le Python autonome de l'étape 0 avec la bibliothèque `websockets`, et le serveur officiel 0.6.8 lancé en local sur la seed MIT :

```text
python MultiServer.py --disable_save crates/multidata/tests/data/seed-mit.archipelago
python -I scripts/roominfo.py roominfo-seed-mit.json
python -I scripts/connect_vectors.py connexion-seed-mit.json
python -I scripts/game_vectors.py partie partie-seed-mit.json
```

`game_vectors.py` modifie l'état de la partie : relancer un serveur neuf avant chaque scénario (`partie`, `objectif`, `echanges`, `stockage`, `commandes`, `indices`…).

## `malformes-seed-mit.json` et `refus-connecte-seed-mit.json`

- Écarts trouvés par les tests différentiels (`tests/differentiel/`, ADR 0008), réduits à un petit scénario, puis capturés sur un serveur officiel 0.6.8 neuf (scripts `game_vectors.py malformes` et `game_vectors.py refus_connecte`), le 15 novembre 2026.
- `malformes` : `cmd` qui n'est pas du texte, paquet sans `cmd` (le serveur recopie le paquet à la façon de Python), outil de suivi qui envoie des lieux mal formés.
- `refus_connecte` : un `Connect` refusé d'un client déjà connecté change quand même son `items_handling`.

Les scénarios eux-mêmes sont dans [`scripts/scenarios.py`](scripts/scenarios.py).
