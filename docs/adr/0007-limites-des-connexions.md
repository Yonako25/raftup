# 0007 — Serveur : limites des connexions

- Statut : accepté
- Date : 2026-11-15

## Contexte

La spec fixe des limites pour protéger le serveur (NF03, NF04, NF05) :

- message de plus de 1 Mio, ou chat de plus de 2 000 caractères ;
- 50 paquets par seconde, 5 messages de chat par 10 secondes ;
- ping toutes les 20 s, connexion muette fermée après 40 s.

Elle ne dit pas quoi faire quand une limite est dépassée.
Les canaux entre les connexions et la tâche salle (ADR 0006) sont aussi sans limite : un client très lent pourrait faire grossir la mémoire.

## Décision

Toutes les valeurs sont dans une structure `Limits`. Les valeurs par défaut sont celles de la spec ; les tests les raccourcissent.

| Limite | Où | Si dépassée |
| --- | --- | --- |
| Message > 1 Mio | connexion (tungstenite) | fermeture de cette connexion, code 1009 |
| Plus de 50 paquets/s | connexion | fermeture de cette connexion, code 1008 |
| Chat > 2 000 caractères | salle | message non diffusé, l'auteur est prévenu |
| Plus de 5 chats / 10 s | salle | message non diffusé, l'auteur est prévenu |
| Rien reçu pendant 40 s | connexion | fermeture de cette connexion |
| File connexion → salle pleine (1 024) | connexion | la connexion attend avant de lire la suite |
| File salle → client pleine (256) | salle | fermeture de ce client, par un signal séparé |

Détails :

- Le débit compte les **paquets**, pas les trames, avec une réserve qui permet une rafale de 50 paquets.
- Au-delà du débit, on ferme au lieu d'ignorer : ignorer pourrait perdre des `LocationChecks`, alors qu'un client reconnecté renvoie tous ses lieux visités.
- Les limites du chat sont vérifiées par la salle, et non par la connexion : seule la salle sait si le client est connecté à un slot et s'il refuse les messages texte (`NoText`), et sa réponse part dans l'ordre des autres.
- La longueur du chat se compte en caractères Unicode, comme `len()` en Python.
- La salle n'attend jamais un client : elle utilise `try_send`. Le signal de fermeture passe par un `Notify`, qui marche même quand la file du client est pleine.

## Conséquences

- Un client abusif ou trop lent est déconnecté sans gêner les autres.
- La mémoire utilisée par client est bornée.
- Le serveur officiel n'a ni limite de débit ni limite de chat : un client qui s'y comporte bien s'y comporte bien ici aussi, mais un client très bavard pourrait être refusé chez Raftup et pas chez Archipelago.
- Le délai de 40 s est le même que celui de la bibliothèque `websockets` du serveur Python (ping 20 s + attente du pong 20 s).
