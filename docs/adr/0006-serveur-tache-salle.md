# 0006 — Serveur : une tâche « salle » possède l'état de la partie

- Statut : accepté
- Date : 2026-11-15

## Contexte

À partir de l'étape 4 de J1, plusieurs connexions partagent l'état d'une partie : les joueurs connectés, puis les lieux visités, les objets reçus, les indices.
Le serveur doit aussi envoyer des messages à plusieurs clients à la fois, par exemple « Alice a rejoint ».
Le serveur officiel tourne sur une seule boucle `asyncio`. Il traite donc les paquets un par un, dans l'ordre d'arrivée.
La spec ne dit pas comment Raftup organise cet état.

## Décision

- Une seule tâche tokio, la **tâche salle**, possède tout l'état de la partie. Aucune autre tâche n'y touche.
- Chaque connexion a sa propre tâche, qui ne fait que du réseau :
  - elle lit les trames, les découpe en paquets et les envoie à la tâche salle par un canal (`mpsc`) ;
  - elle reçoit de la tâche salle, par son propre canal, les trames à envoyer, ou l'ordre de fermer.
- La tâche salle traite les événements un par un : arrivée d'un client, paquets reçus, départ d'un client.

## Conséquences

- L'état n'a pas de verrou (`Mutex`), donc pas de risque d'interblocage ni de verrou gardé pendant une attente réseau.
- Les messages partent dans le même ordre que sur le serveur officiel, ce qui facilite les tests différentiels (NF02).
- La sauvegarde (NF07) se fera au même endroit, avant l'envoi des réponses.
- Une seule tâche traite toute la partie, donc un seul cœur de processeur par salle. C'est largement suffisant pour une partie, et plusieurs salles pourront tourner en parallèle.
- Les canaux vers les clients sont pour l'instant sans limite. Un client très lent pourrait faire grossir la mémoire. À traiter avec la robustesse (J1, étape 6).
