# Plan d'évolution de l'interface du launcher

Rédigé le 21 novembre 2026, après comparaison des maquettes (`docs/maquette/`) avec des captures du launcher actuel (macro `RAFTUP_DEV_CAPTURE`).

## Constat

Le launcher a l'identité visuelle de la maquette (barre latérale, palette Acier, polices), mais pas son **parcours**. Il montre la technique au lieu de guider le joueur.

| Écran | Maquette | Launcher actuel |
| --- | --- | --- |
| Accueil | joués récemment, parties en cours, mises à jour | « Bientôt » |
| Bibliothèque | jaquettes, fiche du jeu, profil d'installation | proche ; mais champs adresse / slot / mot de passe dans la fiche, jaquette « SR » |
| Salon de préparation | jeu + préréglage Rapide / Standard / Long, équipage et état de chacun, « Tester ma connexion », « Générer et héberger » | absent : « Créer une partie » demande des fichiers YAML, des apworlds, un port, des certificats PEM |
| Parties en cours | liste des parties | « En cours » ouvre un formulaire « Héberger une partie » |
| Partie en cours | « Qui te bloque », progression, fil d'activité, « Lancer et connecter » | onglets techniques |
| Catalogue, Mises à jour, Téléchargements, Profil | écrans complets | « Bientôt » |
| Amis | panneau d'amis | réglage de l'adresse du Rafthub au milieu de l'écran, hub local par défaut |

Les problèmes principaux :
1. **Jargon partout** : YAML, apworld, port, wss, PEM, slot, `.archipelago`. Un joueur ne sait pas quoi remplir.
2. **Pas de chemin principal** : « jouer avec des amis » est éclaté entre Créer, Héberger, Rejoindre et la Bibliothèque.
3. **Six entrées du menu mènent à « Bientôt »** : l'application paraît inachevée.
4. **Des réglages d'expert au premier plan** : adresse du hub, port, chiffrement, webhook.
5. **Le chiffrement par défaut demande des certificats PEM** : un débutant est bloqué dès le départ.

## Principes

- Un seul chemin principal : **choisir un jeu → créer un salon → inviter → lancer**. Tout le reste est secondaire.
- Le vocabulaire du joueur : « partie », « salon », « équipage », « préréglage ». Les termes techniques vont dans « Options avancées ».
- De bons réglages par défaut, sans rien à remplir : port libre trouvé seul, chiffrement sans fichier, hub officiel.
- Aucun bouton qui mène à un écran vide : un écran absent est caché, pas annoncé.
- Chaque état dit quoi faire ensuite (« Il manque 2 joueurs prêts : tu peux attendre ou lancer sans eux »).

## Étapes

### Étape 1 : ménage et réglages par défaut (rapide, sans nouvel écran)
- Cacher les entrées « Bientôt » du menu (ou les griser, avec « arrive bientôt » au survol) tant que l'écran n'existe pas.
- « Créer une partie » et « Héberger » : port, adresse publique, chiffrement, webhook et apworlds en plus passent dans un bloc repliable **Options avancées**.
- Le chiffrement par défaut ne demande aucun fichier (certificat généré par Raftup, ou mode adapté au réseau local). Les fichiers PEM passent en avancé.
- Bibliothèque : adresse, slot et mot de passe passent sous « Se connecter à la main ». Les jaquettes deviennent des aplats colorés avec le titre du jeu (SPEC § 7, jamais de visuels officiels).
- Amis : l'adresse du Rafthub part dans un écran **Paramètres**. Le hub par défaut est le hub officiel. Le texte « Aucun Rafthub public » disparaît.
- « Rejoindre » : le lien est lu tout seul s'il est dans le presse-papiers.

### Étape 2 : Salon de préparation (le cœur)

> Version locale faite le 21 novembre 2026 (ADR 0044) : l'hôte saisit l'équipage, Raftup écrit les YAML. Reste le salon par le Rafthub et « Tester ma connexion » avant le lancement.

- L'écran de la maquette « Salon de préparation » : choix du jeu parmi ceux prêts, préréglage Rapide / Standard / Long, « Options avancées… », « Tester ma connexion », hébergement « Sur ce PC / Mon serveur », « Générer et héberger ».
- Les **fichiers d'options sont produits par Raftup** à partir du préréglage du jeu (préréglages dans les profils de jeu). Importer un YAML reste possible en avancé.
- L'équipage : invitation par lien `raftup://salon/<code>` ou à un ami du Rafthub ; état de chaque joueur (choisit son jeu, pas testé, prêt).
- À décider (ADR) : le salon passe par le Rafthub (nouvelles routes `/v1/lobbies`) pour que les invités le voient sans serveur de jeu lancé.

### Étape 3 : Parties en cours et Partie en cours

> Faite en partie le 21 novembre 2026 (branche `cloud/plan-interface-3-5`) : « En cours » montre la progression de chaque joueur de la partie hébergée ; « Partie en cours » a le panneau « Je joue : » et un seul bouton « Lancer et connecter » (le jeu de la Bibliothèque, avec l'adresse et le slot ; le mot de passe reste à taper). Questions ouvertes : suivre une partie **rejointe** (un client de suivi connecté au serveur de l'ami, et où garder la liste des parties) ; cartes « Rien ne te bloque / Qui tu bloques » (calcul à partir des objets de progression de la seed, côté serveur ou launcher).

- « En cours » liste les parties (hébergées ou rejointes), avec la progression de chacun.
- L'écran « Partie en cours » de la maquette : un seul bouton « Lancer et connecter », les cartes « Rien ne te bloque / Qui tu bloques », la progression, les onglets Équipage / Objets / Indices / Chat, le fil d'activité, « Joueur absent » (vote de release, déjà fait côté serveur).

### Étape 4 : Accueil, Catalogue, Mises à jour, Téléchargements

> Faite en partie le 21 novembre 2026 : Accueil (prochain pas, parties en cours, invitation reçue, jeux prêts) et Catalogue (filtre, origine du profil, badge de confiance d'un profil du registre, « Ajouter » ou « Ouvrir » vers la fiche). Questions ouvertes : « Joués récemment » demande un historique des parties (format et emplacement à décider) ; la file de téléchargements avec pause et reprise demande de changer l'installation de `launcher-core` (une seule tâche à la fois aujourd'hui) ; « Mises à jour » et « Téléchargements » restent cachés.

- Accueil : joués récemment, parties en cours, mises à jour disponibles, invitations reçues.
- Catalogue : les jeux pris en charge (profils et registre), filtres, « Ajouter », badge du niveau de confiance (ADR 0045).
- Mises à jour et Téléchargements : file d'installation avec progression, pause et reprise.

### Étape 5 : premier lancement, Paramètres, Profil

> Faite en partie le 21 novembre 2026 : assistant au premier lancement en trois pas (`welcome_done` dans `config.toml`, champ facultatif sans changer `format_version`) ; Paramètres : panneau « Dossiers » ; écran « Mon profil » (le panneau Profil de « Amis », ou une invitation à se connecter). Questions ouvertes : thèmes Océan, Braise, Uni (couleurs absentes de la SPEC, et les couleurs sont des constantes aujourd'hui) ; langue (aucune décision sur la traduction) ; statistiques de parties (même historique que « Joués récemment »).

- Assistant au premier lancement : compte Rafthub (ou continuer sans), détection des jeux, premier jeu prêt.
- Paramètres : hub, dossiers, langue, thème (Acier, Océan, Braise, Uni).
- Profil : pseudo, Discord lié, présence, statistiques de parties.

## Vérification

Chaque étape :
- se vérifie sur captures avec la macro, et un scénario de capture est ajouté par écran nouveau ;
- se compare à la maquette du même écran.

Le test final reste l'essai par une personne qui n'a jamais vu Raftup : elle doit pouvoir lancer une partie avec un ami sans aide (reporté avec les autres essais en vrai, à la fin de la V3).
