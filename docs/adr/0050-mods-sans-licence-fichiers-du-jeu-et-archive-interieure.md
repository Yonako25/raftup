# 0050 — Mods sans licence, fichiers du jeu patchés et archive dans une archive

- Statut : accepté (décisions de l'utilisatrice le 22 novembre 2026)
- Date : 2026-11-22
- Étend l'ADR 0021 (mods sans licence) ; complète l'ADR 0015 (profils d'installation)

## Contexte

Les premiers profils des jeux PC d'Archipelago (lot du MVP) ont rencontré trois obstacles :
1. le mod Archipelago de Risk of Rain 2 n'a aucune licence, comme Slimipelago avant lui (ADR 0021) ;
2. l'API de modding de Hollow Knight (hk-modding/api) remplace `Assembly-CSharp.dll` par une version patchée : du code du jeu, que sa licence MIT ne couvre pas ;
3. SMAPI (Stardew Valley) est livré en installateur : ses fichiers Windows sont dans un zip (`internal/windows/install.dat`) à l'intérieur du zip téléchargé, que le format ne savait pas extraire.

## Décision

1. **Mods sans licence** : la règle de l'ADR 0021 vaut pour tous les mods, plus seulement Slimipelago. `license = "unlicensed"`, auteurs crédités, fichiers téléchargés chez l'auteur sur le PC du joueur, jamais redistribués ; le profil est retiré si l'auteur le demande. Décision de la mainteneuse : en droit d'auteur, l'absence de licence ne donne aucun droit par défaut ; Raftup s'appuie sur le fait qu'il ne redistribue rien, comme un gestionnaire de mods.
2. **Fichiers du jeu patchés par un outil de modding** : acceptés quand ils viennent de la source publique de la communauté (comme Lumafly pour Hollow Knight). Ils ne sont jamais dans le dépôt ; l'original est sauvegardé avant d'être remplacé et restauré à la désinstallation (ADR 0015).
3. **Archive dans une archive** : nouveau champ facultatif `inner` dans `install = { extract = "zip", … }`. C'est le chemin exact d'une archive zip dans le fichier téléchargé ; Raftup en sort cette seule entrée (taille bornée, nom contrôlé comme une entrée d'archive), puis l'extrait comme d'habitude. `format_version` ne change pas : un launcher plus ancien refuse ce champ inconnu au lieu d'installer de travers.

## Conséquences

- Risk of Rain 2, Hollow Knight, Stardew Valley et A Short Hike entrent au catalogue, en `untested` (ADR 0049).
- Un profil peut maintenant installer SMAPI sans exécuter son installateur ; un autre installateur qui fait plus que copier des fichiers resterait hors de portée.
- Le risque juridique des points 1 et 2 est assumé par la mainteneuse ; un retrait sur demande se fait en supprimant le profil.
