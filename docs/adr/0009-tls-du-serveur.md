# 0009 — Serveur : chiffrement wss par certificat PEM

- Statut : accepté
- Date : 2026-11-15

## Contexte

NF06 demande « wss par défaut ; ws en clair uniquement sur option explicite ».
Jusqu'ici, `raftup-server` ne savait que le clair, avec `--no-tls`.
En wss, les clients vérifient le certificat du serveur : un certificat auto-signé serait refusé par les mods de jeu. Il faut donc un vrai certificat, lié à un nom de domaine.
Obtenir et renouveler ce certificat automatiquement (Let's Encrypt) est un gros travail, qui a sa place avec l'hébergement en un clic du launcher (J3).

## Décision

- Le serveur lit un certificat et sa clé privée dans deux fichiers PEM, avec les mêmes options que le serveur officiel : `--cert <fichier>` et `--cert_key <fichier>`.
- Sans certificat, le serveur refuse de démarrer, sauf avec `--no-tls`.
- Un port sert soit wss, soit ws, jamais les deux.
- TLS : `tokio-rustls` (déjà tiré par tokio-tungstenite), moteur `ring` comme le client (ADR 0005). Les PEM sont lus avec `rustls-pki-types`, déjà présent.
- La connexion est écrite une seule fois, générique sur le flux (TCP en clair ou TLS) : limites, ping et fermeture restent identiques.
- Chaque erreur de configuration dit ce qui ne va pas : fichier absent, PEM sans certificat ou sans clé, clé qui ne correspond pas au certificat.
- Tests : certificat auto-signé créé pendant le test avec `rcgen` (dépendance de test), auquel le client de test fait confiance. Aucune clé privée dans le dépôt.

## Conséquences

- Un hôte qui a un nom de domaine et un certificat sert ses joueurs en wss.
- Un hôte sans certificat doit choisir `--no-tls` en connaissance de cause : le mot de passe de la salle passe alors en clair.
- Pas de rechargement du certificat à chaud : un certificat renouvelé demande un redémarrage. À revoir avec Let's Encrypt (J3).
- Pas de wss et ws sur le même port. On pourra l'ajouter derrière une option explicite si un jeu du catalogue n'a pas de TLS.
