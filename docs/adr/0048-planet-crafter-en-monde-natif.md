# ADR 0048 — The Planet Crafter en monde natif, avec le mod TPCA

- **Statut** : accepté le 22 novembre 2026 par l'utilisatrice
- Prolonge les ADR 0041 (format de monde natif), 0042 (`raftup-monde`) et 0046 (mondes natifs dans le registre).

## Contexte

L'utilisatrice veut ajouter The Planet Crafter (Miju Games, Unity, PC) à Raftup. Il existe déjà, le 22 novembre 2026 :

- **TPCA** (`github.com/Hawkrex/TPCA`, MIT) : mod BepInEx 5 qui parle le protocole Archipelago (`Archipelago.MultiClient.Net`), en version 0.0.3 « POC », testé avec Archipelago 0.6.7 et le jeu en 2.104 ;
- **l'apworld de Hawkrex** (fork `Hawkrex/Archipelago-ThePlanetCrafter`, branche `ThePlanetCrafter`, dossier `worlds/the_planet_crafter`, version 0.0.1 « POC »). Le fork porte la licence MIT d'Archipelago ; le dossier du monde n'a pas de licence à lui (à confirmer avec Hawkrex). Sa note de version dit : **pas de logique**. Toutes les régions sont ouvertes ; la fin exige tous les objets de progression.

Ce qui manque, c'est la logique. C'est exactement ce que le format natif sait décrire sans code.

## Décision

1. **Pas de nouveau mod.** Raftup s'appuie sur TPCA, sans le modifier (règle de compatibilité stricte).
2. **Même datapackage que l'apworld de Hawkrex**, pour que TPCA marche tel quel :
   - nom du jeu `The Planet Crafter` ;
   - objets : le champ `Id` de `datas/Items.py` comme nom, `ApId` comme numéro, même classification ; `NoItem` (1) est le remplissage ;
   - lieux : numérotés à partir de 1 dans l'ordre de `datas/RegionDatas.py` (235 lieux) ;
   - `slot_data` : `goal` et `death_link`, mêmes valeurs.
3. **Dépôt à part** dans `D:\planet-crafter` (MIT, notice de Hawkrex), et non dans le dépôt Raftup : Raftup ne garde pas de données de jeux tiers. Le monde se publie ensuite dans le registre Raftup comme paquet `native_world` (ADR 0046) et s'exporte en apworld par `raftup-monde apworld`, qu'on pourra proposer à Hawkrex.
4. **Logique par paliers nommés** : une région par étape de terraformation du jeu (Ciel bleu, Nuages, Pluie, Lacs, Mousse, Flore, Arbres, Insectes, Animaux…). Chaque palier exige les plans de production qui permettent de l'atteindre ; chaque plan de « Progress Blueprints » va dans le palier où le jeu le débloque. Le format ne sait pas additionner des quantités, d'où ce modèle grossier mais lisible.
5. **Seuils pris sur le wiki du jeu** (chiffres seulement, des faits, sans recopier de texte), puis quelques-uns vérifiés en jeu par l'utilisatrice. Pas d'outil d'extraction des fichiers Unity.
6. **Deux étapes** :
   1. `world.toml` qui reproduit l'apworld de Hawkrex à l'identique (même datapackage, même absence de logique), vérifié par la génération (moteur Rust) et par l'aller-retour `raftup-monde apworld` ;
   2. la logique par paliers, puis une vraie partie avec TPCA sur le serveur Raftup.

## Conséquences

- Aucune ligne de C# à écrire ; le travail porte sur les règles du jeu.
- Si Hawkrex change ses numéros, TPCA et notre monde divergent : le monde note la version de l'apworld qu'il suit, et on en parle avec Hawkrex avant de publier.
- Une mise à jour du jeu peut casser TPCA ; ce n'est pas de notre ressort, mais la fiche du registre dira la version testée.
- La logique par paliers peut être trop stricte ou trop lâche à certains endroits ; on l'ajustera après une vraie partie.
- Prendre contact avec Hawkrex reste à faire par l'utilisatrice (message envoyé en son nom).
