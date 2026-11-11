# 0002 — Service `raftup-hub` : comptes, connexion et amis

- Statut : accepté
- Date : 2026-11-11

## Contexte

Le launcher prévoit un panneau d'amis (`docs/SPEC.md`, § 7). Raftup vise un usage à grande échelle : beaucoup de joueurs, plusieurs appareils par joueur, des demandes d'ami envoyées à des joueurs hors ligne.
Une liste d'amis stockée seulement sur le PC, avec un relais en mémoire, ne tient pas à cette échelle.

Modèle suivi : Modrinth. Le code est public, la base de données de l'instance officielle est privée, et les joueurs ont un vrai compte avec une page de connexion.

## Décision

### Service

- Un service séparé, `raftup-hub`, gère les comptes, les amis et la présence. Son code est public, sa base est privée.
- **Stockage** : PostgreSQL, via la crate `sqlx`.
- **Calendrier** : après le MVP. Les jalons J0 à J3 n'utilisent pas de comptes ; le launcher reste utilisable sans compte.

### Connexion

- Au départ, deux méthodes : **e-mail + mot de passe** et **Discord** (OAuth 2.0).
- D'autres méthodes s'ajouteront plus tard (Steam, GitHub, passkeys…) sans changer le modèle de données.
- Le compte et ses moyens de connexion sont séparés : une table `users` (le compte) et une table `user_identities` (fournisseur, identifiant chez le fournisseur). Un compte peut avoir plusieurs méthodes liées.
- **E-mail + mot de passe** :
  - mot de passe haché avec argon2id, jamais stocké ni journalisé en clair ;
  - adresse vérifiée par un lien envoyé par e-mail ;
  - « mot de passe oublié » par lien à usage unique et durée courte ;
  - nombre de tentatives limité par compte et par adresse IP.
- **Discord** : flux OAuth 2.0 avec PKCE, ouvert dans le navigateur du système ; le launcher récupère le résultat par un lien profond (`raftup://`) ou un port local temporaire.
- **Session** : après connexion, le hub délivre un jeton de session révocable. Le launcher le range dans le trousseau du système (crate `keyring`), jamais dans un fichier en clair.

### Pseudo

- Chaque compte a un **pseudo unique**, choisi à la création. Il n'y a pas de code ami.
- Connexion par Discord : le pseudo Discord est proposé par défaut, mais le joueur en choisit un autre s'il est déjà pris.
- Unicité **sans tenir compte de la casse** : `Mimi` et `mimi` sont le même pseudo. On stocke le pseudo tel qu'il est affiché, plus une forme normalisée (minuscules, Unicode NFKC) qui porte la contrainte d'unicité en base.
- Caractères autorisés : lettres, chiffres, `_`, `-` et `.` ; de 3 à 24 caractères.
- Pseudos réservés : `raftup`, `admin`, `moderator`, `support`, etc.
- Changement de pseudo autorisé, avec un délai minimum entre deux changements. L'ancien pseudo reste bloqué un temps, pour éviter qu'on usurpe quelqu'un juste après son changement.
- Les amitiés et les demandes pointent vers l'identifiant interne du compte, jamais vers le pseudo : changer de pseudo ne casse rien.

### Amis

- Un joueur envoie une demande en tapant le pseudo de l'autre, qui l'accepte. Les demandes sont conservées si le destinataire est hors ligne.
- **Présence** (« en ligne », « en partie ») : en mémoire, jamais en base. Redis seulement le jour où plusieurs instances du hub tournent en parallèle.
- Données minimales en base : e-mail, hash du mot de passe, identités liées, pseudo, amitiés, demandes, blocages.

## Conséquences

- Il faut un service d'envoi d'e-mails (vérification, réinitialisation), hébergé en Europe de préférence.
- Les mots de passe deviennent une donnée sensible à protéger : NF09 s'applique au hub.
- RGPD à appliquer : export et suppression du compte par le joueur, hébergement en Europe, politique de confidentialité.
- Les pseudos sont faciles à deviner : nombre de demandes d'ami limité par minute, possibilité de refuser toutes les demandes ou de bloquer un joueur, présence visible seulement par les amis.
- Une instance du hub hébergée par un tiers a ses propres comptes et pseudos, sans lien avec l'instance officielle.
- L'API du hub est versionnée dès sa première version.
- Le hub de production ne tourne pas sur le serveur personnel partagé : machine dédiée, ou au moins base sauvegardée hors du serveur.
