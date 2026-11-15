# Données de test du lecteur pickle

Chaque fichier de données a un fichier `<nom>.attendu.json` : le résultat attendu, **calculé par Python**, dans une forme JSON « canonique » que `tests/comparaison_python.rs` reproduit en Rust.
Les fichiers ont été écrits par le script `gen_pickle_vectors.py` (hors dépôt), le 15 novembre 2026, avec le Python autonome de l'étape 0 et Archipelago 0.6.8.

## `seed-mit.archipelago`

- Seed générée par le générateur officiel d'Archipelago 0.6.8 (`Generate.py --seed 4242`), sans le spoiler.
- 3 joueurs : Alice et Bob sur ChecksFinder, liés par un « item link » (groupe `Lien`), et Chloé sur Yacht Dice. Alice a un indice de départ.
- Ces deux jeux font partie du dépôt officiel (licence MIT, voir `THIRD_PARTY_NOTICES.md`).
- Résultat attendu : fonction officielle `Utils.restricted_loads` du serveur, celle qui lit les seeds.
- On y trouve les classes `NetworkSlot`, `SlotType`, `Hint` et `HintStatus`, et de grands entiers (`LONG1`).

## Petits pickles (`*.pickle`)

Données simples fabriquées exprès pour couvrir ce que la seed n'a pas (nombres à virgule, octets, `frozenset`, entiers géants, texte long, valeur partagée, `Counter`), aux protocoles 3, 4 et 5. Résultat attendu : `pickle.loads`.

`ensembles-p2.pickle` est un cas de **refus** : au protocole 2, Python écrit l'ancien nom `__builtin__.set`, que le serveur officiel refuse. Son `.attendu.json` contient l'erreur du serveur officiel, et le test vérifie que Raftup refuse aussi.

## Forme canonique

| Valeur Python | JSON |
| --- | --- |
| `None`, booléen, texte | tel quel |
| entier qui tient dans un `i64` | nombre |
| entier plus grand | `{"bigint": octets petit-boutistes en hexadécimal}` |
| nombre à virgule | `{"float": 8 octets petit-boutistes en hexadécimal}` |
| octets | `{"bytes": hexadécimal}` |
| liste, tuple | `{"list": […]}`, `{"tuple": […]}` |
| dictionnaire | `{"dict": [[clé, valeur], …]}`, dans l'ordre |
| ensemble | `{"set": […]}`, `{"frozenset": […]}` ; ordre libre (trié avant comparaison) |
| objet d'une classe autorisée | `{"object": "module.Classe", "args": […], "state": … ou null}` |

## `seed-mit.modele.json`

Le modèle de partie attendu pour `seed-mit.archipelago`, calculé par le script `gen_model_expected.py` (hors dépôt) avec le code officiel : `Utils.restricted_loads`, puis `NetUtils.encode` pour les valeurs que le serveur envoie en JSON (`slot_data`, slots, indices). `tests/modele.rs` construit la même forme à partir de `MultiData` et compare champ par champ.

Le test `seed_externe_comme_python` (ignoré par défaut) fait la même comparaison sur une seed hors dépôt :

```text
RAFTUP_SEED=<seed.archipelago> RAFTUP_SEED_MODELE=<modèle.json> cargo test -p multidata --test modele -- --ignored
```

Il a été lancé le 15 novembre 2026 sur la seed Slime Rancher de l'étape 0 : identique à Python.
