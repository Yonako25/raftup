# 0044 — Salon local : équipage saisi par l'hôte, YAML écrits par Raftup

- Statut : accepté (choix de l'utilisatrice le 21 novembre 2026, « Salon local d'abord »)
- Date : 2026-11-21
- Complète l'ADR 0019 (écran « Créer une partie »), l'ADR 0023 (préréglages) et l'ADR 0024 (salon côté hôte)

## Contexte

La maquette « Salon de préparation » (`docs/maquette/`) part d'un jeu et d'un préréglage. L'écran « Créer une partie » demandait au contraire un fichier YAML par joueur : un débutant ne sait pas ce que c'est.
L'étape 2 du plan d'interface (`docs/plan-interface-launcher.md`) laissait deux choix ouverts :
- qui écrit les fichiers d'options ;
- si le salon passe par le Rafthub (routes `/v1/lobbies`), pour que les invités le voient avant le lancement du serveur.

## Décision

1. **Le salon reste local**, sans le hub. L'hôte saisit l'équipage : son jeu, puis, pour chaque ami, un pseudo et un jeu choisi dans le catalogue. Le « prêt ? » de chaque joueur reste le test de connexion de l'ADR 0024, une fois la partie hébergée.
2. **Raftup écrit un YAML par joueur**, au clic sur « Générer et héberger », dans `generations/salon/` (vidé de ses anciens YAML à chaque génération). Le contenu se limite à trois choses :
   - `name` : le pseudo, entre guillemets doubles et échappé ;
   - `game` : le nom Archipelago du jeu (`ap_game` du profil) ;
   - une section vide pour ce jeu, ce qui laisse les options par défaut de l'apworld.

   Le préréglage reste celui de l'ADR 0023 (délai de génération et `meta.yaml`, communs à tous les jeux).
3. **Le format des profils ne change pas.** Des options par jeu et par préréglage dans les profils restent possibles plus tard, avec un nouvel ADR.
4. **Les pseudos sont vérifiés** avant d'écrire quoi que ce soit (module pur `launcher_core::crew`) :
   - de 1 à 16 caractères, la limite d'Archipelago ;
   - ni accolade, parce qu'Archipelago y remplace `{player}` et `{number}` ;
   - ni caractère de contrôle ;
   - uniques sans tenir compte de la casse.
5. **Les YAML importés à la main restent possibles**, dans les « Options avancées » de l'écran. Quand il y en a, ils remplacent l'équipage.

## Conséquences

- Plus besoin de connaître le YAML pour lancer une partie. En contrepartie, les options propres à un jeu (objectif, difficulté…) gardent leur valeur par défaut tant qu'on n'importe pas de YAML.
- Le joueur ne choisit pas son jeu lui-même : c'est l'hôte qui le saisit. La version par le hub (invités qui choisissent et se déclarent prêts) reste à décider plus tard.
- La forme exacte du YAML (section vide acceptée par `Generate.py` 0.6.8) est à vérifier par une vraie génération.
