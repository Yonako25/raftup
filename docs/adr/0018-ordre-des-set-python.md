# 0018 — Ordre des indices comme un `set` Python

- Statut : accepté
- Date : 2026-11-17
- Remplace la partie « ordre » de l'ADR 0012 (étape 4 de la diffusion : « dans l'ordre croissant »).

## Contexte

Le serveur officiel range les indices de chaque joueur dans un `set` Python (`MultiServer.py`, `self.hints`). L'ordre où il les parcourt se voit :

- `!hint` sans argument affiche les indices dans cet ordre ;
- `_read_hints_<équipe>_<slot>` (`SetReply`, `Retrieved`) envoie la liste dans cet ordre ;
- `get_hint` prend le premier indice qui convient ;
- `!hint <objet>` range les indices déjà connus avec `set(hints) - ...`.

De petits `set` d'entiers fixent aussi l'ordre des `SetReply` : `new_hint_events`, `updated_slots`, `concerning_slots`, et `slot_set` pour un groupe.

Jusqu'ici Raftup gardait les indices dans une liste, dans l'ordre d'arrivée. L'écart était déclaré. La porteuse a choisi de reproduire l'ordre exact.

## Décision

Un module `server/src/pyset.rs` imite un `set` de CPython 3.12, la version épinglée par l'étape `generate` (ADR 0017) :

- la même table de hachage que `Objects/setobject.c` : taille puissance de 2, sondage linéaire puis `perturb`, cases fantômes après un retrait, agrandissement aux 3/5 ;
- les mêmes opérations que le serveur officiel : `add`, `discard`, `set(liste)`, `copy`, `update` (`set_merge`), `-` (`set_difference`) ;
- le même hachage : entiers modulo 2^61 - 1, tuples (xxHash, CPython 3.8 et après), textes (SipHash-1-3).

Un indice est haché comme `Hint.__hash__` : `hash((receiving_player, finding_player, location, item, entrance))`. Il est comparé sur ses 8 champs.

`GameState` garde un `PySet<Hint>` par (équipe, slot) et refait chaque opération du serveur officiel, dans le même ordre :

- indices de départ : `update` avec le `set` lu dans le pickle, dans l'ordre du fichier ;
- `notify_hints` : `add` ;
- `replace_hint` : `discard` puis `add` ;
- `recheck_hints`, `!hint` sans argument : un `set` neuf, rempli dans l'ordre de l'ancien ;
- reprise d'une sauvegarde `.raftup` : un `set` neuf, rempli dans l'ordre enregistré, comme l'officiel qui relit son pickle.

Vérification : `tests/data/scripts/pyset_vectors.py` joue 31 suites d'opérations tirées au hasard sur le vrai CPython 3.12, et note l'ordre après chaque opération. Le test `pyset::tests::matches_cpython_vectors` les rejoue. Le différentiel ne trie plus les indices.

## Limites (écarts encore déclarés)

- **Textes non vides** : Python tire au hasard la clé de hachage des textes à chaque lancement. Raftup prend la clé nulle, celle de `PYTHONHASHSEED=0`. L'ordre est donc exact quand `entrance` est vide, le cas le plus courant. Avec une entrée renseignée (jeux à entrées mélangées), le serveur officiel change lui-même d'ordre à chaque lancement : l'ordre de Raftup correspond à un serveur officiel lancé avec `PYTHONHASHSEED=0`. Le différentiel lance l'officiel ainsi.
- **Sauvegarde automatique de l'officiel** : toutes les 60 secondes, le serveur officiel appelle `recheck_hints()` sur tous les slots, ce qui reconstruit chaque `set`. Cette reconstruction peut, dans de rares cas de collision, changer l'ordre. Elle dépend de l'heure : Raftup ne la reproduit pas. Le différentiel lance l'officiel avec `--disable_save`.
- **Version de Python** : l'imitation suit CPython 3.12. Un serveur officiel lancé avec une autre version peut parcourir ses `set` autrement.
- Le tirage de `!hint <objet>` entre ex æquo reste déterministe et différent du Mersenne Twister (ADR 0012).

## Conséquences

- `!hint` sans argument, `_read_hints_…` et l'ordre des `SetReply` sont maintenant joués par le différentiel au hasard.
- Le format `.raftup` ne change pas : l'ordre de la liste d'indices, déjà enregistrée, devient significatif.
- Toute nouvelle opération sur les indices doit passer par `PySet`, dans l'ordre du serveur officiel.
