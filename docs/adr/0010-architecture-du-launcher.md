# 0010 — Architecture du launcher (hébergement d'une partie)

- Statut : accepté
- Date : 2026-11-16

## Contexte

Le launcher (J3) commence par les écrans « Héberger une partie » et « Partie en cours » (maquette du 16 novembre 2026).
L'hôte choisit une seed, règle le port, le mot de passe et le chiffrement, démarre le serveur, puis suit la partie : joueurs, lieux visités, objectifs, activité, journal.
SPEC.md fixe Tauri v2 et une interface en HTML/CSS/TypeScript sans framework lourd ; CLAUDE.md impose que `launcher-ui` ne dépende que de `launcher-core`.
Il faut décider comment le launcher fait tourner le serveur et comment il le suit.

## Décision

- **Le serveur tourne dans le launcher**, dans le même process (F09 : héberger en un clic) : `launcher-core` dépend de la crate `server`.
- **Le serveur s'ouvre au suivi** sans perdre sa tâche salle unique (ADR 0006) :
  - `Server::start` renvoie un `RunningServer` : adresse, arrêt (`stop`), photo de l'état (`status`) et fil d'événements (`subscribe`) ;
  - la photo est demandée à la tâche salle par un message, qui y répond : personne d'autre ne lit l'état ;
  - les événements (rejoint, parti, objet envoyé, chat, objectif) sont des données structurées, avec les noms des objets et des lieux ; le launcher écrit lui-même les phrases en français.
- **`launcher-core`** : `host` (démarrer, suivre, arrêter une partie), `seed` (lire une seed pour montrer ses slots avant le démarrage), journal du serveur gardé en mémoire par une couche `tracing`.
- **`launcher-ui`** : application Tauri v2. Des commandes Tauri appellent `launcher-core` ; l'interface interroge l'état une fois par seconde, ce qui suffit pour un tableau de bord et reste simple.
- **Interface** : HTML et CSS écrits à la main, TypeScript compilé par `tsc` seul (dépendance npm de développement), sans bundler. Polices Chakra Petch et Inter intégrées à l'application (licence OFL) : le launcher marche hors ligne.

## Conséquences

- Fermer le launcher arrête la partie hébergée : c'est voulu au MVP.
- `server` gagne une petite API publique de suivi, testée à part ; le binaire `raftup-server` n'en a pas besoin et ne change pas.
- Node.js et npm deviennent des outils de développement du launcher (pas des joueurs : l'application livrée ne contient que le JavaScript compilé).
- L'interrogation chaque seconde coûte un message à la tâche salle par seconde : négligeable.
