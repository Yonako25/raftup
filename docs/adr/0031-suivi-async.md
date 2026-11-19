# 0031 — Suivi asynchrone : résumé de retour, absence, release par vote, notifications

- Statut : accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice
- Date : 2026-11-19

## Contexte

La SPEC (V2) demande un « suivi async : résumé de retour, notifications, release par vote, mode absence ». Une partie asynchrone dure des jours : un joueur revient sans savoir ce qui s'est passé, un autre part en vacances, un troisième abandonne et bloque tout le monde (BK).

Contraintes :

- un mod Archipelago existant ne doit voir **aucune différence de protocole** : tout passe par ce qu'il sait déjà afficher ou envoyer (chat `!commande`, `PrintJSON`, stockage `Get` / `SetNotify`) ;
- les tests différentiels (ADR 0008) comparent Raftup au serveur officiel : le comportement par défaut du serveur ne doit pas changer ;
- l'état doit survivre à un redémarrage (sauvegarde `.raftup`, ADR 0014).

## Décision

### Activation

- Option de la salle `Room::with_async_follow(true)`, **désactivée par défaut**. Le binaire `raftup-server` l'active avec `--async_follow` ; le launcher l'active toujours (`Host::start`), sans réglage dans l'interface.
- Désactivée, le serveur se comporte exactement comme avant : mêmes commandes, même `!help`, mêmes messages. Les tests différentiels lancent le binaire sans l'option : ils ne voient rien.

### Commandes `!` ajoutées (seulement si l'option est active)

Ajoutées **à la fin** de la liste des commandes (donc aussi dans « Known commands » et `!help`) :

| Commande | Effet |
| --- | --- |
| `!away [message]` | Marque son slot « absent », avec un message (200 caractères au plus). Annoncé à tous. |
| `!back` | Retire l'absence. Annoncé à tous. |
| `!summary` | Redonne l'état asynchrone : absents, votes en cours, dernières entrées du fil d'activité. |
| `!vote_release [joueur]` | Vote pour relâcher le monde d'un joueur. Sans argument : liste les votes en cours. |

Réponses en anglais, comme les autres messages du serveur, en `PrintJSON` (`CommandResult` pour la réponse au demandeur, texte simple sans `type` pour les annonces) : tout client qui affiche le chat les affiche.

### Résumé de retour

- Quand la **dernière** connexion d'un slot part, le serveur note pour ce slot l'heure et le nombre d'objets reçus (« vu pour la dernière fois »). À l'arrêt propre du serveur, les slots encore connectés sont notés aussi.
- À la connexion suivante de ce slot (nouvelle connexion, pas une reconnexion sur la même), après le message `Tutorial`, le serveur envoie au seul joueur un résumé en `PrintJSON` : depuis combien de temps il est parti, les objets reçus depuis (10 noms au plus, puis « and N more »), les entrées du fil d'activité depuis (objectifs, release, absences, votes), les absents, et les votes contre son monde.
- Première connexion d'un slot : pas de résumé.

### Mode absence

- État par slot : heure de début et message.
- Message automatique : un message de chat (pas une commande) qui contient le nom ou le surnom d'un joueur absent (casse ignorée) reçoit, pour son seul auteur, la réponse `Bob is away (since 2 h 05 min): <message>`. Jamais pour l'absent lui-même.
- L'absence se lève par `!back`, ou d'elle-même dès que le joueur visite un nouveau lieu (il joue de nouveau).

### Release par vote

- Votent les **joueurs** de l'équipe (pas les groupes), sauf le joueur visé. Il faut la **majorité stricte** : `votants / 2 + 1` (3 autres joueurs → 2 voix ; 1 autre → 1 voix).
- Refus : voter pour soi (`!release` existe pour ça), viser un groupe, un joueur qui a atteint son objectif, ou un monde déjà entièrement visité ; partie solo (personne pour voter).
- Le vote **passe outre `release_mode`** : c'est la décision collective que la SPEC demande pour un joueur qui abandonne. L'hôte garde `!admin /release` et `/forbid_release` pour le reste.
- Majorité atteinte : les votes sont effacés et le monde est relâché comme par `/release` (même message `Release`, mêmes objets envoyés).
- Les votes contre un joueur sont annulés dès qu'il visite un nouveau lieu (annonce à tous).

### Repli par le stockage

Deux clés en lecture seule, comme les clés `_read_…` officielles, que n'importe quel client ou tracker peut lire par `Get` et suivre par `SetNotify` :

- `_read_raftup_away_<équipe>` : `{"<slot>": {"since": 1794..., "message": "..."}}` ;
- `_read_raftup_release_votes_<équipe>` : `{"<slot visé>": [slots votants]}`.

Option désactivée : ces clés valent `null`, comme toute clé `_read_` inconnue du serveur officiel.

### Fil d'activité et notifications

- Le serveur garde un fil d'activité borné (200 entrées, numérotées) : objectif atteint, release, absence, retour, vote, release par vote. Il sert au résumé et à `!summary`.
- Le fil d'événements du launcher (`GameEvent`, ADR 0010) reçoit `Away`, `Back` et `ReleaseVote` ; `SlotStatus` porte l'absence et le nombre de votes.
- Le webhook Discord (F10) annonce l'absence, le retour, chaque vote et la release par vote.
- L'écran « Partie en cours » montre l'absence et les votes dans l'équipage et dans le fil d'activité.

### Sauvegarde

- Le `.raftup` passe en `format_version` **2** : l'instantané gagne `away`, `seen`, `release_votes`, `feed` (absents ou vides par défaut) et quatre deltas (`away`, `seen`, `release_votes`, `feed`).
- Un fichier version 1 se lit toujours (les nouveaux champs sont vides) ; il est réécrit en version 2 au compactage du démarrage. Un ancien Raftup refuse un fichier version 2 (version inconnue), au lieu de le lire à moitié.

## Conséquences

- Un mod existant n'a rien à changer : il affiche les messages et peut envoyer les commandes par son chat.
- Le comportement par défaut du serveur reste celui du serveur officiel ; seules les parties lancées par le launcher (ou avec `--async_follow`) ont ces fonctions.
- Les noms de commandes sont propres à Raftup : sur archipelago.gg, `!away` répond « Could not find command ». C'est voulu (rien de propriétaire sans repli : la partie se joue sans).
- Le résumé ne compte que ce qui est arrivé pendant l'absence de **toutes** les connexions du slot ; un tracker resté connecté empêche donc le résumé du jeu.
- Après un arrêt brutal du serveur (pas d'arrêt propre), un joueur connecté à ce moment peut recevoir un résumé qui reprend des objets déjà vus.
