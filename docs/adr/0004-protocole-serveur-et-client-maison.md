# 0004 — Protocole, serveur et client écrits par Raftup

- Statut : accepté
- Date : 2026-11-11
- Remplace : la décision « Serveur : on réutilise pahoa » et le choix de `archipelago_rs` comme client (`docs/SPEC.md`, version 1)

## Contexte

La version 1 de la spec prévoyait trois sources pour les mêmes paquets réseau :
- `ap-protocol` (nos types) ;
- `pahoa-proto` (types du serveur pahoa, MIT, version 0.2.0) ;
- `archipelago_rs` (client, MIT, version 3.0.1, basé sur `smol` et non sur `tokio`).

Avec ces trois sources, il aurait fallu des conversions partout. Raftup dépendrait aussi de deux projets tiers jeunes pour son cœur.
Choix du projet : Raftup écrit lui-même son protocole, son serveur et son client.

## Décision

- **`ap-protocol`** définit lui-même tous les paquets, dans **les deux sens** (client → serveur et serveur → client), avec `serde`. C'est la seule source de vérité, utilisée par `server` et `client-core`.
- **`server`** est écrit par Raftup, sur `tokio`. Il n'a pas de dépendance à pahoa.
- **`client-core`** est écrit par Raftup, sur `tokio` et `ap-protocol`. Il n'a pas de dépendance à `archipelago_rs`.
- **`multidata`** écrit son propre lecteur de `.archipelago` (zlib + pickle), en liste blanche, avec des quotas (NF08).
- Les projets tiers servent de **références**, pas de dépendances :
  - le serveur Python officiel d'Archipelago est l'**oracle des tests différentiels** (NF02) ;
  - pahoa et `archipelago_rs` peuvent être lus pour comprendre le protocole et ses pièges ;
  - les captures de paquets réels servent de vecteurs de test.
- **Pas de copie de code.** Si un passage est malgré tout porté depuis un projet MIT, sa notice va dans `THIRD_PARTY_NOTICES.md`.

## Pièges connus du protocole, à couvrir par des tests

- Ordre des clés des dictionnaires conservé (comme en Python) : `serde_json` avec la fonctionnalité `preserve_order`.
- Entiers plus grands que `u64` (bitfields dans le DataStorage) : `serde_json` avec `arbitrary_precision`, sinon ils sont corrompus en `f64`.
- Champs inconnus ou optionnels : tolérés à la lecture, pour suivre les évolutions du protocole.
- Profondeur d'imbrication JSON limitée, en plus de la taille maximale des messages (NF03).

## Conséquences

- Tout le code est à Raftup, sur un seul runtime async (`tokio`), avec un seul jeu de types.
- Plus de travail : J0 (protocole + client) et J1 (serveur + lecture du `.archipelago`) seront plus longs que prévu.
- La conformité repose sur nos tests : vecteurs de référence et tests différentiels contre le serveur Python dès J1.
- Le risque « pahoa abandonné » disparaît ; il est remplacé par le risque « écart de comportement avec le serveur officiel ».
- C'est aussi un bon terrain d'apprentissage de Rust : les types du protocole forment une première étape simple et bien testable.
