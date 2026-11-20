# ADR 0037 — Rafthub : base intégrée par défaut et premier admin depuis le navigateur

- **Statut** : accepté en autonomie le 20 novembre 2026 (l'utilisatrice a laissé le choix), à relire
- Complète l'ADR 0033 (hub, SQLite ou PostgreSQL) et l'ADR 0036 (console d'admin)

## Contexte

L'utilisatrice veut un Rafthub « avec une base de données intégrée » : on le lance, et il marche.

Le hub savait déjà utiliser SQLite, mais :
- la base par défaut (`raftup-hub.db`) était créée dans le **dossier courant** : lancé depuis un autre dossier, le hub repartait d'une base vide ;
- le premier admin demandait **deux étapes en ligne de commande** : créer un compte par l'API, puis `rafthub admin create <e-mail>`.

## Décision

### Base intégrée
- SQLite reste la base intégrée : un seul fichier, rien à installer, déjà testé (ADR 0033). Pas de nouvelle dépendance.
- Sans `--db` ni `RAFTUP_HUB_DB`, le hub range ses données dans `rafthub-data/` **à côté de l'exe** : la base `rafthub.db` et le cache du launcher `cache/` (au lieu de `rafthub-cache` dans le dossier courant). Le dossier est créé au premier lancement.
- PostgreSQL reste possible avec `--db postgres://…`, pour un gros hub.

### Premier admin depuis le navigateur
- Au démarrage, si aucun compte n'a le rôle d'admin, le hub tire un **code d'installation** (16 caractères en base32 de Crockford, comme les invitations) et l'écrit dans son journal.
- `/admin/login` renvoie alors vers `/admin/installation` : code, e-mail, pseudo, mot de passe. Le compte est créé (le mode d'inscription ne s'applique pas), il reçoit le rôle d'admin, sa session s'ouvre, et le journal `admin_events` note `account_created` (« premier admin, page d'installation »).
- Le code prouve qu'on a accès au serveur : sans lui, le premier venu pourrait prendre la console d'un hub tout juste installé.
- Dès qu'un admin existe, la page renvoie vers la connexion. Le code n'est gardé qu'en mémoire : il change à chaque démarrage, et n'est plus tiré quand un admin existe.
- Mêmes protections que la connexion : jeton anti-CSRF par cookie, mot de passe haché argon2, pas d'e-mail dans le journal.
- `rafthub admin create|remove <e-mail>` reste là pour les admins suivants et pour le dépannage.

## Conséquences

- Un hub neuf se lance avec `rafthub`, puis on ouvre `/admin` et on suit la page.
- Une base `raftup-hub.db` laissée dans un dossier courant n'est plus lue par défaut : il faut la passer avec `--db` (aucun hub n'est encore déployé).
- Deux personnes qui envoient le formulaire avec le bon code au même instant peuvent créer deux admins : c'est accepté, puisque les deux connaissent le code.
