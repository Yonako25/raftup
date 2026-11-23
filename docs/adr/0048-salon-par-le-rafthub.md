# 0048 — Salon de préparation par le Rafthub (routes `/v1/lobbies`)

- Statut : **proposé** (en attente de la validation de l'utilisatrice ; rien n'est codé)
- Date : 2026-11-21
- Complète l'ADR 0044 (salon local), l'ADR 0024 (« prêt ? » par vraie connexion), l'ADR 0035 (client du hub) et l'ADR 0043 (empreinte dans le lien)

## Contexte

L'ADR 0044 a fait un salon **local** : l'hôte saisit l'équipage (pseudo et jeu de chaque ami), Raftup écrit les YAML, puis l'hôte génère et héberge. Ses conséquences laissaient ouverte la version par le hub, où les invités voient le salon, choisissent leur jeu et se déclarent prêts **avant** que le serveur de jeu ne tourne. Le plan d'interface (`docs/plan-interface-launcher.md`, étape 2) la demande aussi.

Ce qui manque avec le salon local :
- l'hôte doit connaître le jeu de chaque ami et le taper lui-même ;
- un invité ne voit rien avant de recevoir le lien de la partie déjà générée ;
- « prêt ? » n'existe qu'une fois le serveur lancé (ADR 0024) : on découvre trop tard qu'un ami n'a pas installé son jeu.

Le Rafthub (crate `hub`, AGPL) a déjà des comptes, des amis, la présence et des blocages ; le launcher lui parle par `launcher_core::hub_client` (MIT/Apache, HTTP seulement, ADR 0035). Un salon partagé a besoin d'un endroit que tous les joueurs voient sans serveur de jeu : le hub est cet endroit.

Contraintes :
- le hub ne voit **jamais** le mot de passe de la partie ni le contenu d'un YAML (rien qui serve à entrer dans la partie sans y être invité) ;
- sans hub (ou hub hors ligne), le salon local de l'ADR 0044 reste possible et inchangé ;
- une crate MIT/Apache ne dépend jamais du hub (CLAUDE.md, ADR 0003) ;
- rien ne change pour un mod de jeu : le salon s'arrête au lancement, la partie se joue ensuite sur le serveur de jeu, comme aujourd'hui.

## Décision proposée

Chaque point est à valider ; la recommandation est en premier.

1. **Qui peut entrer dans un salon.**
   *Recommandé* : un compte connecté qui a le **code** du salon, partagé par un lien `raftup://salon/<code>` ou par une invitation à un ami du hub (qui reçoit le code dans « Amis »). Un joueur bloqué par l'hôte (ou qui l'a bloqué) est refusé. Le code est tiré au hasard, 10 caractères en base 32 sans caractères ambigus (≈ 50 bits), et il change si l'hôte le régénère.
   *Autre choix* : amis du hub seulement (plus fermé, mais pas d'invité hors de la liste d'amis).

2. **Routes `/v1/lobbies`** (toutes avec le jeton du compte, comme `/v1/friends`) :

   | Route | Qui | Effet |
   | --- | --- | --- |
   | `POST /v1/lobbies` | tout compte | crée un salon : nom, préréglage (`Rapide`, `Standard`, `Long`), nombre maximal de joueurs ; rend le code |
   | `GET /v1/lobbies` | membre | les salons dont je suis membre |
   | `GET /v1/lobbies/{code}` | membre | l'état du salon : membres, jeu choisi, prêt, version (voir 4) |
   | `POST /v1/lobbies/{code}/join` | compte avec le code | entre dans le salon |
   | `PATCH /v1/lobbies/{code}/me` | membre | mon jeu (`ap_game`), mon nom de slot, prêt ou non |
   | `POST /v1/lobbies/{code}/leave` | membre | quitte le salon |
   | `DELETE /v1/lobbies/{code}/members/{pseudo}` | hôte | retire un membre |
   | `POST /v1/lobbies/{code}/invite` | hôte | invite un ami du hub (il voit le salon dans « Amis ») |
   | `POST /v1/lobbies/{code}/launch` | hôte | fige l'équipage et publie l'adresse du serveur de jeu et l'empreinte `fp` (ADR 0043), visibles des seuls membres |
   | `DELETE /v1/lobbies/{code}` | hôte | ferme le salon |

   Les réponses d'erreur suivent les codes typés du hub (`ErrorCode`), avec une phrase en français côté launcher.

3. **Ce que le hub garde d'un salon.**
   *Recommandé* : en base, deux nouvelles tables (`lobbies`, `lobby_members`, schéma en version 7), sur SQLite, PostgreSQL, MySQL et MariaDB comme le reste (mêmes tests). Pour chaque membre : pseudo du compte, nom de slot (16 caractères, mêmes règles que `launcher_core::crew`), `ap_game` (64 caractères au plus, texte libre vérifié par le launcher de l'hôte au lancement), prêt (oui/non), heure de la dernière mise à jour. Pour le salon : hôte, nom, préréglage, code, état (`ouvert`, `lancé`, `fermé`), adresse et empreinte une fois lancé. **Jamais** de mot de passe de partie, de YAML, d'apworld ni de seed.
   *Autre choix* : en mémoire seulement (plus simple, mais tout est perdu au redémarrage du hub).

4. **Mise à jour à l'écran.**
   *Recommandé* : le launcher relit `GET /v1/lobbies/{code}` toutes les 3 s tant que l'écran du salon est ouvert (60 s sinon, comme la présence), avec un numéro de version du salon : une réponse « pas changé » est courte. Pas de nouvelle connexion permanente (WebSocket, SSE) : c'est le modèle déjà utilisé par la présence (ADR 0035), et il passe par les mêmes proxys.
   *Autre choix* : flux SSE (plus réactif, mais une connexion ouverte par joueur sur le hub et un nouveau type de route à tester).

5. **Sens de « prêt ».**
   *Recommandé* : avant le lancement, « prêt » est **déclaré** par le joueur ; le launcher ne propose de cocher « prêt » que si le jeu choisi est détecté et ses mods installés (Bibliothèque). Après le lancement, le vrai test de connexion de l'ADR 0024 reprend sa place. L'hôte peut lancer sans que tout le monde soit prêt (« Il manque 2 joueurs prêts : tu peux attendre ou lancer sans eux »).

6. **Lancement.**
   *Recommandé* : au clic sur « Générer et héberger », le launcher de l'hôte relit le salon, fabrique l'équipage de l'ADR 0044 à partir des membres (pseudo de slot et jeu de chacun), écrit les YAML, génère et héberge **sur son PC** comme aujourd'hui, puis appelle `/launch` avec l'adresse publique et l'empreinte. Chaque membre voit alors « Rejoindre », qui fabrique son lien `raftup://join?…&slot=…&game=…&fp=…` localement et passe par l'écran « Rejoindre » existant. Le mot de passe de la partie, s'il y en a un, ne passe pas par le hub : l'hôte le donne autrement (ou n'en met pas, l'accès au salon étant déjà limité aux membres).

7. **Limites.**
   *Recommandé* : 16 membres au plus par salon (groupes d'amis, SPEC § 1) ; 3 salons ouverts au plus par hôte ; un salon sans activité pendant 24 h est fermé, un salon lancé est fermé 7 jours après ; les routes du salon ont la même limite de requêtes que `/v1/friends`.

8. **Vie privée et RGPD.**
   Les salons d'un compte figurent dans son export (`GET /v1/me/export`) et disparaissent avec lui (le salon d'un hôte supprimé est fermé). Le journal `admin_events` note la création et la fermeture d'un salon, sans pseudo de slot ni adresse. La console `/admin` ne montre que des compteurs.

9. **Côté launcher.**
   Les appels vivent dans `launcher_core::hub_client` (MIT/Apache, HTTP par `ureq`, sans dépendre du hub) ; les tests de bout en bout entre le vrai routeur du hub et le client du launcher vont dans `crates/hub/tests/` (le hub AGPL dépend du launcher en test, jamais l'inverse). L'écran « Créer une partie » gagne un choix « Salon local / Salon en ligne » ; sans compte ou sans hub, seul le salon local est proposé.

## Conséquences

- Les invités choisissent leur jeu eux-mêmes et voient avant la génération qui manque : le chemin « choisir un jeu → créer un salon → inviter → lancer » du plan d'interface devient complet.
- Le hub porte un nouvel état partagé, à tenir sur quatre bases de données, avec migration de schéma (version 7) et tests pour chacune.
- Le hub connaît, pour chaque salon, qui joue avec qui, à quel jeu et l'adresse du serveur de jeu une fois lancé : donnée personnelle de plus à couvrir par l'export, la suppression et la politique de confidentialité du hub officiel.
- La relecture toutes les 3 s charge le hub en proportion des salons ouverts ; acceptable pour des groupes d'amis, à mesurer sur le hub officiel.
- Le salon local de l'ADR 0044 reste le repli : aucune partie ne dépend du hub pour être jouée.
- Questions qui restent à trancher avant de coder : le choix 1 (code ou amis seulement), le choix 3 (base ou mémoire), le choix 4 (relecture ou SSE), et le texte exact à montrer pour le mot de passe de la partie (point 6).
