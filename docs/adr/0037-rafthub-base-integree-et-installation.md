# ADR 0037 — Rafthub : assistant de premier lancement (base de données, premier admin)

- **Statut** : accepté en autonomie le 20 novembre 2026 (l'utilisatrice a laissé le choix), à relire
- Complète l'ADR 0033 (hub, SQLite ou PostgreSQL) et l'ADR 0036 (console d'admin)

## Contexte

L'utilisatrice veut un Rafthub « avec une base de données intégrée » : on le lance, et il marche.

Le hub savait déjà utiliser SQLite, mais :
- la base par défaut (`raftup-hub.db`) était créée dans le **dossier courant** : lancé depuis un autre dossier, le hub repartait d'une base vide ;
- le premier admin demandait **deux étapes en ligne de commande** : créer un compte par l'API, puis `rafthub admin create <e-mail>`.

## Décision

### Étape 1 : choix de la base (demandé par l'utilisatrice le 20 novembre 2026)
- Le hub garde ses données dans `rafthub-data/` **à côté de l'exe**, créé au premier lancement : le fichier `rafthub.toml`, la base intégrée `rafthub.db` et le cache du launcher `cache/` (au lieu de `rafthub-cache` dans le dossier courant).
- Ordre pour trouver la base : `--db` ou `RAFTUP_HUB_DB`, sinon `rafthub-data/rafthub.toml` (`format_version = 1`, `database_url`), sinon **l'assistant**.
- L'assistant : le hub ne sert alors que `/admin/installation` (le reste y renvoie, l'API `/v1` répond 503). La page propose :
  - **base intégrée (SQLite)** : rien à installer, pour un petit groupe ;
  - **base externe**, choisie dans une liste : PostgreSQL (MySQL / MariaDB affiché « pas encore pris en charge », grisé), avec adresse, port, nom de la base, utilisateur, mot de passe.
- Le choix est **essayé** (connexion et création des tables, 10 s au plus) avant d'être écrit dans `rafthub.toml`. Un échec remontre la page avec les champs tapés, sans jamais réafficher le mot de passe ni l'URL.
- Ensuite le petit serveur s'arrête et le vrai hub démarre sur la base choisie, sur le même port ; la page passe d'elle-même à l'étape 2.
- La base intégrée est écrite `sqlite://rafthub.db`, **relative** au dossier de données : on peut déplacer le dossier du hub.
- L'URL d'une base externe contient son mot de passe : `rafthub.toml` est aussi secret que la base.
- Sans JavaScript : les champs de la base externe sont cachés en CSS (`:has()`) quand la base intégrée est cochée.

### Étape 2 : premier admin depuis le navigateur
- Au démarrage, si aucune base n'est choisie ou si aucun compte n'a le rôle d'admin, le hub tire un **code d'installation** (16 caractères en base32 de Crockford, comme les invitations) et l'écrit dans son journal.
- Le même code sert aux deux étapes. `/admin/login` renvoie vers `/admin/installation` : code, e-mail, pseudo, mot de passe. Le compte est créé (le mode d'inscription ne s'applique pas), il reçoit le rôle d'admin, sa session s'ouvre, et le journal `admin_events` note `account_created` (« premier admin, page d'installation »).
- Le code prouve qu'on a accès au serveur : sans lui, le premier venu pourrait prendre la console d'un hub tout juste installé.
- Dès qu'un admin existe, la page renvoie vers la connexion. Le code n'est gardé qu'en mémoire : il change à chaque démarrage, et n'est plus tiré quand un admin existe.
- Mêmes protections que la connexion : jeton anti-CSRF par cookie, mot de passe haché argon2, pas d'e-mail dans le journal.
- `rafthub admin create|remove <e-mail>` reste là pour les admins suivants et pour le dépannage.

### Cache de la feuille de style
- `rafthub.css` et `copy.js` sont servis en `Cache-Control: no-cache` (les polices gardent un jour de cache) : avec un jour de cache, un admin voyait l'ancienne mise en page après une mise à jour du hub.

## Conséquences

- Un hub neuf se lance avec `rafthub`, puis on ouvre `/admin` et on suit les deux étapes.
- `toml` passe des dépendances de test aux dépendances du hub (déjà dans le workspace).
- Changer de base après coup : modifier ou supprimer `rafthub.toml` (pas de migration de données d'une base à l'autre).
- Une base `raftup-hub.db` laissée dans un dossier courant n'est plus lue par défaut : il faut la passer avec `--db` (aucun hub n'est encore déployé).
- Deux personnes qui envoient le formulaire avec le bon code au même instant peuvent créer deux admins : c'est accepté, puisque les deux connaissent le code.
