# 0001 — Le catalogue de jeux est public, dans le dépôt

- Statut : accepté
- Date : 2026-11-11

## Contexte

Le launcher installe des mods sur le PC du joueur à partir de profils d'installation déclaratifs (`docs/SPEC.md`, F02).
Il faut décider où vit le catalogue (la liste des jeux et leurs profils) : dans le dépôt public, ou dans une base de données privée servie par un serveur Raftup.

Un profil dit ce qui sera téléchargé et exécuté chez le joueur : tout le monde doit pouvoir le relire.

## Décision

- Le catalogue entier est public, dans le dépôt, sous `profiles/` : un fichier TOML par jeu.
- Il n'y a pas de base de données privée pour le catalogue.
- Les ajouts et corrections passent par des pull requests, relues comme du code.
- Chaque profil épingle les versions et le hash (`sha256`) de ce qu'il installe.

## Conséquences

- Transparence : n'importe qui peut auditer ce que le launcher installe.
- La communauté peut maintenir les profils, sans dépendre d'un serveur Raftup.
- Une mise à jour du catalogue demande une pull request fusionnée : c'est plus lent qu'une base, mais c'est relu.
- Les profils ne contiennent que des métadonnées techniques et des URL avec leur hash : jamais de jeu, d'asset, de jaquette officielle ni de mod ré-hébergé.
- Ajout du 21 novembre 2026 (ADR 0047) : un profil peut donner un identifiant IGDB (`igdb_id`) ; les illustrations viennent du hub, jamais du dépôt.
- Le catalogue est embarqué dans chaque version du launcher : pas de téléchargement séparé, pas de serveur de catalogue.
- Conséquence : corriger un profil cassé demande une nouvelle version du launcher, distribuée par l'updater (F12).
