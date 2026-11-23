# ADR 0052 — Partie hébergée par Rafthub ou en self-host

- Statut : **accepté** le 23 novembre 2026 par l'utilisatrice, avec toutes les recommandations (points 1 à 12) ; rien n'est codé
- Date : 2026-11-23
- Complète l'ADR 0002 et l'ADR 0033 (hub et son API `/v1`), l'ADR 0035 (client du hub), l'ADR 0031 (suivi async), l'ADR 0043 (chiffrement et empreinte dans le lien) et la proposition de salon par le Rafthub (branche `cloud/adr-salon-rafthub`, numérotée 0048 là-bas, à renuméroter à la fusion)

## Contexte

Aujourd'hui, une partie tourne toujours sur le PC de celui qui l'héberge (« self-host »). Le serveur de jeu (crate `server`, MIT/Apache) est lancé par le launcher, et la sauvegarde `.raftup` reste sur ce PC. Si l'hôte éteint son PC, la partie s'arrête pour tout le monde. C'est gênant pour les parties longues jouées en async (ADR 0031), où chacun joue à son rythme sur plusieurs jours.

Rafthub (crate `hub`, AGPL) tourne déjà en continu sur un serveur (`https://hub.raftup-games.com`, Docker + Traefik). L'utilisatrice veut qu'à la création d'une partie, on puisse choisir :

- **hébergée par Rafthub** : le hub fait tourner le serveur de jeu et garde la partie ;
- **self-host** : comme aujourd'hui, sur le PC de l'hôte.

Contraintes qui ne changent pas :
- un mod d'Archipelago non modifié doit se connecter à une partie hébergée par Rafthub exactement comme à une partie en self-host. Il ne connaît qu'une adresse `hôte:port` : on ne peut pas mettre plusieurs parties derrière un seul port en jouant sur le chemin de l'URL ;
- une crate MIT/Apache ne dépend jamais du hub. L'inverse est permis : le hub (AGPL) peut se servir de `server` et de `multidata` (MIT/Apache) ;
- le self-host reste toujours possible, sans compte et sans hub ;
- jamais de jeu, de ROM ni d'asset sur le hub : il ne reçoit que la seed (`.archipelago`), qui ne contient que la logique de la partie.

## Décision proposée

Chaque point est à valider ; la recommandation est en premier.

1. **Choix du mode à la création.**
   *Recommandé* : l'écran « Créer une partie » (et le salon, s'il est validé) gagne un choix « Héberger sur mon PC » / « Héberger sur Rafthub ». Le second n'apparaît que si le joueur est connecté à un hub qui l'annonce dans `GET /v1/info` (`hosting: true`), comme la connexion Discord (ADR 0035). Un hub auto-hébergé peut le couper dans ses réglages.

2. **Qui génère la seed.**
   *Recommandé* : **le PC de l'hôte**, comme aujourd'hui (moteur Python dans son bac à sable, ou moteur Rust). Le launcher envoie ensuite au hub le fichier `.archipelago` obtenu (`POST /v1/games`, 32 Mio au plus). Le hub n'a donc besoin ni de Python, ni des apworlds, ni des YAML. Il relit la seed avec la lecture sécurisée de `multidata`, qui refuse tout ce qui n'est pas une donnée de partie attendue.
   *Autre choix* : le hub génère lui-même à partir des YAML. Les mondes tiers seraient alors à installer sur le serveur, et le générateur Python à faire tourner en bac à sable sous Linux : bien plus lourd, à garder pour plus tard.

3. **Où tourne le serveur de jeu.**
   *Recommandé* : **dans le processus du hub**, une tâche `tokio` par partie, avec `server::Server` (`bind`, `with_tls`, `with_save`, `start`, `stop`) et les limites de `server::Limits`. C'est simple à déployer (un seul conteneur), et le code du serveur est le même qu'en self-host.
   *Autre choix* : un conteneur par partie, mieux isolé mais beaucoup plus compliqué à piloter (accès à Docker depuis le hub). À reconsidérer seulement si une partie peut faire tomber le hub.

4. **Adresse et ports.**
   *Recommandé* : une **plage de ports** réservée aux parties (par exemple 38281 à 38380, réglable), publiée directement par Docker sans passer par Traefik. Chaque partie prend un port libre de la plage, gardé tant qu'elle existe. L'adresse donnée aux joueurs est `hub.raftup-games.com:<port>`. Les connexions sont chiffrées (`wss`) avec le certificat du domaine du hub. Le `ws` en clair reste accepté sur le même port pour les mods qui ne savent pas faire de `wss` (comme l'ADR 0043). Le lien d'invitation `raftup://join?…` ne change pas de forme.

5. **Sauvegarde.**
   *Recommandé* : le hub garde la sauvegarde `.raftup` de chaque partie dans son dossier de données (`/data/parties/<id>/`) et l'écrit régulièrement, comme le launcher. L'hôte peut à tout moment **télécharger** la seed et la sauvegarde (`GET /v1/games/{id}/export`) pour reprendre la partie en self-host. Il peut aussi faire l'inverse : envoyer une partie self-host déjà commencée, avec sa sauvegarde, pour qu'elle continue sur le hub. Le format `.raftup` ne change pas.

6. **Mise en veille et réveil.**
   *Recommandé* : une partie sans aucun joueur connecté pendant 2 h est mise **en veille** : le serveur s'arrête, la sauvegarde reste et le port reste réservé. L'écran « Rejoindre » du launcher et la page de la partie la **réveillent** (`POST /v1/games/{id}/wake`) avant de se connecter. Un mod lancé seul, sans le launcher, trouve donc une partie en veille fermée : le launcher l'explique (« Ouvre la partie depuis Raftup pour la réveiller »).
   *Autre choix* : laisser toutes les parties allumées en continu. C'est plus simple pour les joueurs, mais la mémoire du serveur fixe alors le nombre de parties.

7. **Limites** (réglables dans la console d'admin, valeurs de départ) :
   - 2 parties actives au plus par compte, et 40 sur tout le hub ;
   - 16 joueurs au plus par partie, et 32 Mio au plus pour la seed ;
   - une partie sans aucune connexion pendant 30 jours est supprimée, après un avertissement à l'hôte (dans « Amis », et par e-mail si le SMTP est réglé) 7 jours avant ;
   - les limites de connexion et de débit du serveur de jeu (`server::Limits`) s'appliquent à chaque partie.

8. **Mot de passe et commandes d'admin.**
   *Recommandé* : le serveur de jeu a besoin du mot de passe de la partie en clair pour le comparer à celui d'un joueur. Le hub le garde donc, mais **chiffré au repos** avec une clé propre au hub (dans `/data`). La console d'admin ne l'affiche jamais. Le mot de passe d'admin de la partie (`!admin`) est tiré au hasard par le hub et n'est montré qu'à l'hôte, dans son launcher.

9. **Routes `/v1/games`** (toutes avec le jeton du compte) :

   | Route | Qui | Effet |
   | --- | --- | --- |
   | `POST /v1/games` | compte | envoie une seed (et éventuellement une sauvegarde), avec un nom et un mot de passe facultatif ; rend l'identifiant, l'adresse et l'empreinte `fp` |
   | `GET /v1/games` | compte | mes parties hébergées (état : active, en veille) |
   | `GET /v1/games/{id}` | hôte ou membre | état : joueurs connectés, avancement par slot (comme « Partie en cours ») |
   | `POST /v1/games/{id}/wake` | hôte ou membre | réveille une partie en veille |
   | `GET /v1/games/{id}/export` | hôte | télécharge la seed et la sauvegarde |
   | `DELETE /v1/games/{id}` | hôte | arrête et supprime la partie |

   Un « membre » est un membre du salon de la partie, s'il est validé. Sinon, c'est un joueur à qui l'hôte a donné l'identifiant par le lien d'invitation.

10. **Console d'admin.** Une page « Parties » liste les parties (hôte, nombre de joueurs, état, taille, dernière activité). Elle permet de les mettre en veille ou de les supprimer, et de régler les limites du point 7. Le journal `admin_events` note la création, la veille et la suppression d'une partie, sans mot de passe ni contenu de la seed.

11. **Vie privée et RGPD.** Les parties d'un compte figurent dans son export (`GET /v1/me/export`, sous forme de métadonnées, la seed restant téléchargeable par `/export`). Elles sont supprimées avec le compte. La politique de confidentialité du hub officiel est mise à jour : le hub garde désormais la seed, la sauvegarde et le nom des slots de chaque partie hébergée.

12. **Côté launcher.** Les appels vont dans `launcher_core::hub_client` (MIT/Apache, sans dépendre du hub). « Partie en cours » sait suivre une partie hébergée sur le hub, en interrogeant le hub au lieu du serveur local. Les tests de bout en bout entre le vrai hub et le client du launcher vont dans `crates/hub/tests/`.

## Conséquences

- Une partie hébergée sur Rafthub continue quand l'hôte éteint son PC : c'est ce qui rend les longues parties async vraiment jouables.
- Le hub devient aussi un serveur de jeu. Il consomme du CPU, de la mémoire et de la bande passante en proportion des parties actives. Les limites du point 7 et la veille du point 6 servent à tenir sur le serveur actuel ; il faudra les mesurer sur le hub officiel.
- Une plage de ports est à ouvrir sur le pare-feu du serveur et dans `docker-compose.yml`, en plus de Traefik.
- Le hub garde de nouvelles données (seed, sauvegarde, mot de passe chiffré) : export, suppression et politique de confidentialité à mettre à jour.
- Le self-host ne change pas et reste le repli : aucune partie ne dépend du hub pour être jouée, et une partie hébergée peut toujours être récupérée pour être reprise en self-host.
- Avec le salon (branche `cloud/adr-salon-rafthub`), le chemin complet devient : créer un salon → les amis choisissent leur jeu → l'hôte génère sur son PC → la partie part sur Rafthub → tout le monde rejoint, même quand l'hôte n'est plus là.
- Tranché le 23 novembre 2026 : seed générée par le PC de l'hôte (point 2), serveur dans le processus du hub (point 3), veille après 2 h (point 6), limites du point 7 telles quelles. Les parties hébergées passent avant le salon, qui attend toujours sa validation.

## Étapes proposées (une fois validé)

1. Hub : héberger une seed envoyée, avec sauvegarde et plage de ports, testé sans réseau extérieur (`crates/hub/tests/`).
2. Launcher : choix du mode dans « Créer une partie », envoi de la seed, « Rejoindre » et « Partie en cours » pour une partie hébergée.
3. Veille et réveil, export et reprise en self-host.
4. Console d'admin (page « Parties », limites) et RGPD.
5. Déploiement sur le hub officiel (ports, pare-feu), puis une vraie partie avec un mod non modifié.
