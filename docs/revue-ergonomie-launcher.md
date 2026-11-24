# Revue d'ergonomie du launcher

24 novembre 2026. Revue faite sur les captures sans fenêtre (`cargo captures`, dossier `target/captures/`), avec quatre écrans ajoutés aux captures pour l'occasion : salon avec « Héberger sur Rafthub » (`16`), partie hébergée par Rafthub en ligne et en veille (`17`, `17b`), « En cours » avec une partie de ce PC et une partie du hub (`18`).

**Tout a été accepté le 24 novembre 2026, points « à valider » compris, et codé en six étapes** (branche `ergonomie-launcher`, un commit par étape). Écarts avec la liste ci-dessous :

- point 9 : la phrase « Sans invitation, le jeu démarre sans se connecter » est retirée, sans infobulle (le bouton dessiné à la main n'en a pas) ;
- point 11 : « Nom chez Archipelago » reste en clair dans la fiche du Catalogue (non replié) ;
- point 8 : annulé après essai dans la vraie fenêtre (il manquait l'indicateur d'origine du monde) ; chaque jaquette garde son badge « Natif », « Converti » ou « Archipelago », et la fiche du jeu le reprend avec une phrase d'explication ;
- en plus : « Indique ton pseudo. » au lieu de « Ton pseudo : Indique un pseudo. », bouton « Ouvrir » à largeur minimale, ligne d'une partie Rafthub sans « sur Rafthub » en double.

## Constat général

Le launcher est propre visuellement (palette, angles, polices respectés), mais il parle encore comme un outil de développeur et il cache son parcours principal.

1. **Le parcours principal est caché.** « Créer une partie » n'est pas dans la barre latérale. On y arrive par l'Accueil (seulement si un jeu est prêt), ou par En cours (vide) → « Héberger une partie » (un fichier `.archipelago`) → « Créer depuis des YAML ». Ce dernier libellé est faux : l'écran est maintenant le salon avec l'équipage, sans YAML à fournir.
2. **Jargon partout** : *seed*, *slot*, *apworld*, *World*, *Mod loader*, *hash vérifié*, *versions épinglées*, *wss / ws*, *certificat*, *process séparé*, *Profil du catalogue*. Un joueur ne sait pas ce que c'est, et n'en a pas besoin.
3. **Les parties n'ont pas de nom** : une partie de ce PC s'appelle « seed 71849263510274 » (en-tête, Accueil, En cours).
4. **Trop d'explications techniques** : chiffrement expliqué sur 2 écrans, génération expliquée 2 fois dans le salon, versions de chaque composant en panneau entier.
5. **Deux façons de montrer une partie** : la partie de ce PC (bandeau, onglets, fil d'activité) et la partie sur Rafthub (pile de 5 panneaux) ne se ressemblent pas.
6. **Tutoiement et vouvoiement mélangés** : « Ton pseudo », « Tu organises » d'un côté ; « Collez le lien », « Vérifiez », « Ajoutez au moins un fichier YAML », « Hébergez une seed » de l'autre.
7. **Mots différents pour la même chose** : « lieux » (En cours, Rafthub) et « emplacements » (partie en cours) ; « Invitations » (menu) et « Rejoindre une partie » (titre) ; « Salon de préparation » et l'onglet « Salon » de la partie en cours, qui fait autre chose (tester les connexions).

## Écran par écran

| Écran | Ce qui gêne |
| --- | --- |
| **Barre latérale** (`01`) | Pas d'entrée « Nouvelle partie ». « RÉGLAGES » et « CATALOGUE » : deux sections pour un seul lien chacune. En bas, trois cartes : « Amis » et « Joueur » mènent toutes deux à « Sans compte » ; « Serveur intégré · Arrêté » répète « En cours ». |
| **Accueil** (`12`) | Bien. Quand une partie tourne, la vitrine (« Une partie tourne sur ce PC ») et le panneau « Parties en cours » disent la même chose. Partie nommée « seed 7184… ». Badge « ARCHIPELAGO » sur la jaquette. |
| **Catalogue** (`13`, `13b`) | Badge « ARCHIPELAGO » sur les 10 jaquettes : il ne distingue rien. « Jeu non trouvé » sonne comme une erreur. Fiche : « Nom chez Archipelago », « Licence : unlicensed », « Apworld … · World ». |
| **Bibliothèque, fiche** (`02`, `04`) | « Réinstaller » a le même poids que « Lancer le jeu ». « Profil du catalogue de Raftup », « Profil d'installation · versions épinglées », « hash vérifié », « Mod loader / Mod client / World ». « Sans adresse ni slot, le jeu démarre sans se connecter » sous le bouton principal. Champ « Slot », exemple `ws://hote:38281`. |
| **Salon de préparation** (`05`, `16`) | Panneau entier « Profil de session » (versions d'Archipelago, de Raftup, du monde). Hébergement : « Port 38281 · adresse localhost · chiffré par Raftup (wss, sans fichier) ». Avec Rafthub : les deux choix s'empilent au lieu d'être côte à côte, puis une étiquette « Sur Rafthub · hôte » répète le choix ; la génération est expliquée deux fois (« La seed est générée sur ce PC, puis envoyée… »). La colonne de droite déborde de la marge de 9 px. |
| **Héberger un fichier** (`07`) | Bouton « Créer depuis des YAML » (faux, voir plus haut). « Dans cette partie · slots ». « Les joueurs se connecteront à wss://hôte:port ». Le panneau de gauche chevauche celui de droite de 6 px. |
| **Partie en cours, ce PC** (`08`, `09`) | Titre « seed 7184… ». Trois boutons de copie (« Copier l'invitation » en haut, « Copier l'adresse », « Copier le lien » par joueur). Phrase sur le chiffrement (« un mod de jeu qui se connecte avec cette adresse passe en clair »). « choisir mon slot ». Onglet « Salon » : « Tester ouvre une vraie connexion sur le slot », et un second champ de mot de passe. « emplacements » au lieu de « lieux ». |
| **Partie sur Rafthub** (`17`, `17b`) | Cinq panneaux empilés, tous ouverts. Phrase technique : « il essaie wss puis ws ; les liens d'invitation vérifient le certificat du hub ». « Mot de passe d'admin » en panneau entier (outil d'expert). « Continuer sans Rafthub » et « Supprimer » occupent autant de place que l'équipage. Étiquette « Sur Rafthub » qui répète le fil d'Ariane. |
| **En cours** (`18`) | Pas de bouton « Nouvelle partie » dès qu'une partie existe. « seed 7184… », « seed 4242 », adresse du hub dans la ligne. Bouton « Ouvrir » élargi par des espaces. État vide : « Hébergez une seed générée par Archipelago » mène à l'écran du fichier `.archipelago`, pas au salon. |
| **Rejoindre** (`06`) | Deux clics (« Lire le lien » puis « Continuer »). Vouvoiement. « Slot ». Le menu dit « Invitations », le titre « Rejoindre une partie ». |
| **Amis, Profil** (`10`, `14`) | Corrects. L'en-tête montre « Rafthub · 127.0.0.1:1 » même avec le hub par défaut. Vouvoiement (« vous@exemple.org ou votre pseudo »). |
| **Paramètres** (`11`) | Seul réglage : l'adresse du serveur Rafthub, avec une explication sur `https://` et `raftup://hub/<hôte>`. C'est un réglage d'expert en premier plan. |
| **Assistant** (`15`) | « Passer » (quitter l'assistant) et « Continuer sans compte » côte à côte se confondent. |

## Changements proposés, classés par impact

Les points marqués **(à valider)** retirent quelque chose de l'écran ou changent la navigation : ils ne seront faits qu'avec un accord explicite.

### Impact fort

1. **« Nouvelle partie » dans la barre latérale (à valider)** : section PARTIES = « Nouvelle partie » (le salon), « En cours », « Rejoindre » (au lieu d'« Invitations »). Bouton « Nouvelle partie » en haut d'« En cours » ; l'état vide d'« En cours » mène au salon. L'écran « Héberger un fichier .archipelago » reste, atteint depuis le salon (bouton déjà là) ; son bouton « Créer depuis des YAML » devient « Retour au salon ».
2. **Nommer les parties** : titre = nom donné à la partie, sinon « Partie de <hôte> » (pseudo de l'organisateur), sinon les jeux (« Slime Rancher · 3 joueurs »). Le numéro de seed descend dans le journal et l'infobulle. Touché : en-tête de la partie en cours, Accueil, En cours, sous-titre Rafthub.
3. **Chasse au jargon** (textes seulement, aucune fonction retirée) :
   - *slot* → « nom de joueur » / « joueur » ; *seed* → « partie » dans les phrases (« Génération de la partie… », « La partie est trop grosse pour Rafthub ») ;
   - « hash vérifié » → « vérifié » ; « Mod loader / Mod client / World » → « Chargeur de mods / Mod du jeu / Règles Archipelago » ; « Profil d'installation · versions épinglées » → « Ce que Raftup a installé » ; « Profil du catalogue de Raftup » retiré ;
   - phrases sur wss / ws / certificat / chiffrement retirées des écrans de partie (gardées en infobulle sur l'étiquette « Chiffrée ») ;
   - « process séparé », « Serveur Raftup intégré, protocole Archipelago » retirés ;
   - exemple d'adresse `ws://hote:38281` → `adresse:38281`.
4. **Salon plus léger** :
   - **(à valider)** panneau « Profil de session » remplacé par une ligne discrète sous « Lancer la partie » (« Archipelago 0.6.8 · Raftup 0.1.0 »), détails en infobulle ;
   - choix « Sur mon PC | Sur Rafthub » en sélecteur à segments sur une ligne, sans l'étiquette qui le répète ; une seule phrase d'explication par mode ;
   - résumé du serveur réduit à « Tes amis se connecteront à <adresse>:<port> » (avec un avertissement seulement si l'adresse publique manque) ;
   - note sous « Générer et héberger » retirée (elle répète l'hébergement).
5. **Partie sur Rafthub plus proche de la partie de ce PC** : en-tête (nom, état, « Copier l'adresse »), puis Équipage ; « Mot de passe d'admin », « Continuer sans Rafthub » et « Supprimer » rangés dans un bloc replié « Gérer la partie ». Aucune fonction retirée.

### Impact moyen

6. **Tutoiement partout** : Rejoindre, Amis, Bibliothèque, En cours, messages d'erreur (« Ajoute au moins un joueur »).
7. **Mêmes mots pour les mêmes choses** : « lieux » partout ; « Rejoindre » partout ; l'onglet « Salon » de la partie en cours devient « Vérifier les joueurs ».
8. **Badges de monde (à valider)** : ne montrer que « Natif » et « Converti », l'exception ; « Archipelago » (le cas normal aujourd'hui) n'est plus écrit sur chaque jaquette, il reste dans la fiche.
9. **Fiche du jeu** : « Réinstaller » en petit bouton secondaire à côté de « Désinstaller » ; « Ce que Raftup a installé » replié par défaut ; la phrase « Sans adresse ni slot… » remplacée par l'infobulle du bouton.
10. **Partie en cours (ce PC)** : un seul bouton de copie par joueur (« Copier l'invitation ») et « Copier l'adresse » ; le bouton du haut devient « Copier toutes les invitations » ; « choisir mon slot » → « choisis ton joueur ».
11. **Catalogue** : « Jeu non trouvé » → « Pas sur ce PC » ; « Licence : unlicensed » → « Licence : non précisée » ; « Nom chez Archipelago » replié avec la liste des mods.
12. **Barre latérale (à valider)** : sections JEUX (Accueil, Bibliothèque, Catalogue) et PARTIES (Nouvelle partie, En cours, Rejoindre) ; « Paramètres » en bas ; cartes « Amis » et « Joueur » fusionnées en une carte profil (pseudo + amis en ligne) ; carte « Serveur intégré » montrée seulement quand une partie tourne.

### Impact faible

13. **Défauts de mise en page** : chevauchement des colonnes de « Héberger un fichier » (6 px) ; colonne droite du salon qui dépasse la marge (9 px) ; bouton « Ouvrir » élargi par des espaces (`"   Ouvrir   "`) au lieu d'une largeur minimale.
14. **Rejoindre en un clic** : le lien est lu dès qu'il est collé ; « Continuer » reste le seul bouton.
15. **Accueil** : panneau « Parties en cours » caché quand la vitrine montre déjà la partie.
16. **Amis** : adresse du Rafthub dans l'en-tête seulement si ce n'est pas le hub par défaut.
17. **Paramètres** : adresse du Rafthub dans un bloc replié « Avancé » ; les dossiers en premier.
18. **Assistant** : « Passer » → « Passer l'assistant », placé en lien discret, pour ne pas le confondre avec « Continuer sans compte ».

## Ordre de travail proposé

Une étape = un commit, avec ses tests egui_kittest adaptés et les captures régénérées et regardées.

| Étape | Points | Risque |
| --- | --- | --- |
| A | 3, 6, 7, 11, 18 (textes seulement) | faible : libellés cherchés par les tests |
| B | 2 (noms des parties) | faible |
| C | 4, 13 (salon, mise en page) | moyen : captures de référence `salon.png` |
| D | 5, 10 (écrans de partie) | moyen |
| E | 1, 12, 15 (navigation) | moyen : `app.rs`, tests du menu |
| F | 8, 9, 14, 16, 17 | faible |

Hors de cette revue : les écrans « Bientôt » (Téléchargements, Salons, Mises à jour), le contenu de « Options avancées » du salon (YAML à la main, moteur Rust), l'écran « Amis » une fois connecté (pas de capture avec un compte).
