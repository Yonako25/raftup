# 0049 — Statut « non testé » pour les profils d'installation

- Statut : accepté (choix de l'utilisatrice le 22 novembre 2026)
- Date : 2026-11-22
- Complète l'ADR 0015 (profils d'installation)

## Contexte

L'utilisatrice veut intégrer au catalogue tous les jeux PC du monde intégré d'Archipelago 0.6.8, en commençant par les quatre du MVP (Risk of Rain 2, Stardew Valley, Hollow Knight, A Short Hike).
Ces profils sont écrits à partir des guides officiels et des fichiers réellement téléchargés (sha256 calculés), mais personne ne les a essayés avec le vrai jeu : les sessions cloud tournent sous Linux, sans les jeux.
Le format n'avait que deux statuts : `available` (présenté comme prêt) et `broken` (non installable). Aucun des deux ne dit la vérité sur ces profils.

## Décision

- Nouveau statut `untested` dans `[game] status`. Le champ existait déjà : `format_version` ne change pas. Un launcher plus ancien refuse ce profil (valeur inconnue) au lieu de le présenter comme prêt.
- Un profil `untested` s'installe, se lance et se désinstalle comme un profil `available`.
- Le launcher l'annonce : marqueur « Non testé » sous la jaquette dans la Bibliothèque, et une phrase en orange dans la fiche du jeu.
- Après un essai réel réussi (installation, connexion à une partie, désinstallation propre), le profil passe en `available`, avec la date de l'essai en commentaire.

## Conséquences

- Le catalogue peut grandir sans attendre un essai sous Windows pour chaque jeu, sans tromper le joueur.
- Les profils `untested` restent soumis à toutes les règles de l'ADR 0015 (sha256, liste blanche des hôtes, chemins bornés) : ce statut ne relâche aucune vérification.
- Il faudra suivre les profils encore `untested` (liste dans `docs/etat-du-projet.md`).
