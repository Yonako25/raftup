# Tests différentiels

Le serveur officiel d'Archipelago et `raftup-server` jouent les mêmes scénarios ; le pilote compare ce que chaque client reçoit (ADR 0008, NF02).

- `differentiel.py` : le pilote. Il compile `raftup-server`, lance les deux serveurs sur la seed MIT, joue les scénarios et compare.
- `hasard.py` : le générateur de scénarios tirés au hasard.
- Les scénarios fixes sont dans [`../data/scripts/scenarios.py`](../data/scripts/scenarios.py), partagés avec les scripts de capture.

Ces tests ne tournent pas dans la CI : il faut Python et la source d'Archipelago.

## Préparer

1. Le Python autonome de l'étape 0 : `cpython-3.12.16+20261110-x86_64-pc-windows-msvc-install_only` (python-build-standalone), vérifié avec son `SHA256SUMS`.
2. Les dépendances du serveur officiel, dont `websockets` : `python -m pip install -r requirements.txt` depuis la source d'Archipelago.
3. La source d'Archipelago 0.6.8 (archive du dépôt `ArchipelagoMW/Archipelago`, étiquette `0.6.8`).

## Lancer

Depuis la racine du dépôt :

```text
set RAFTUP_ARCHIPELAGO=<dossier source d'Archipelago 0.6.8>
<python> -I crates/server/tests/differentiel/differentiel.py
```

Options :

- `--fixes` : seulement les scénarios fixes, un serveur neuf pour chacun ;
- `--hasard N --graine G` : N scénarios tirés au hasard ; la même graine redonne les mêmes scénarios ;
- `--longueur L` : étapes par scénario au hasard (25 par défaut) ;
- `--sortie DOSSIER` : où écrire les écarts et les journaux des serveurs (`target/differentiel` par défaut).

Sans option : les scénarios fixes, puis 50 scénarios au hasard (graine 1).
Le code de sortie vaut 1 s'il y a au moins un écart.

## Lire un écart

Chaque écart est écrit dans `ecart-<scénario>.json`, au format des vecteurs de `tests/data/` : pour chaque étape, le client, les paquets envoyés, les trames du serveur officiel (`recu`) et celles de Raftup (`recu_raftup`).

Pour corriger :

1. Écrire un petit scénario qui reproduit l'écart dans `scenarios.py` (`PARTIES`).
2. Le capturer sur le serveur officiel avec `game_vectors.py` (voir `../data/README.md`).
3. Ajouter un test qui rejoue la capture dans `tests/connexion.rs`, vérifier qu'il échoue, puis corriger Raftup.

## Comment le pilote se synchronise

Après chaque étape, chaque client envoie un `GetDataPackage` vide, la « barrière ». Sa réponse prouve que le serveur a traité tout ce qui précède ; elle est retirée avant la comparaison.
Le serveur officiel envoie certains messages par des tâches à part (`Join`, `SetReply`…) : le pilote fait donc au moins deux barrières, et continue tant qu'une barrière apporte encore des trames.
Une étape dure au moins 0,1 s : Raftup ferme une connexion au-delà de 50 paquets/s (ADR 0007).
Deux `Say` d'un même client sont espacés d'au moins 2,1 s : Raftup refuse plus de 5 messages de chat en 10 s.

## Ce qui n'est pas comparé

- Les écarts connus et acceptés (`docs/etat-du-projet.md`) : le générateur les évite.
- Le nombre d'étoiles du masque de `!admin login` : tiré au hasard par l'officiel, ramené à 8 des deux côtés.
- Une partie des commandes `!` : le générateur au hasard ne joue ni `!options`, ni `!license`, ni `!admin`, ni `!countdown`, ni `!hint <objet>` (tirage entre ex æquo, ADR 0012), ni de nom approché (ordre des suggestions au hasard en Python). Les indices (`create_as_hint`, `CreateHints`, `UpdateHint`, `!hint_location`, `!hint` sans argument) sont joués.
- Les indices ne sont **pas** triés : Raftup suit l'ordre du `set` Python (ADR 0018). Le pilote lance l'officiel avec `PYTHONHASHSEED=0`, le hachage des textes que Raftup imite.
- Les limites propres à Raftup (ADR 0007) : le serveur officiel n'en a pas.
- Le code de fermeture WebSocket : seule la fermeture elle-même compte (`<fermé>`).
