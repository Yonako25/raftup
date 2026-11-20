# Déployer le Rafthub officiel (Docker + Traefik)

Ce dossier contient tout ce qu'il faut pour faire tourner le **Rafthub officiel**
(binaire `rafthub` compilé avec l'option `officiel`, qui permet la connexion
Discord, ADR 0038) sur un serveur Linux où **Traefik** gère déjà le HTTPS.

| Fichier | Rôle |
| --- | --- |
| `Dockerfile` | compile `rafthub` et produit deux images : le hub et sa sauvegarde |
| `Dockerfile.dockerignore` | fichiers laissés hors du build (target, .git…) |
| `docker-compose.yml` | le hub derrière Traefik + le service de sauvegarde nuitière |
| `.env.example` | modèle du fichier `.env` (domaine, réseau Traefik…) |
| `backup.sh` | sauvegarde à chaud de la base SQLite, avec rotation |

Le hub parle HTTP en clair sur le port 8787 **à l'intérieur** du réseau Docker ;
Traefik fait le TLS devant (ADR 0033). Aucun port n'est ouvert sur l'hôte.

## 1. Prérequis

- Docker avec le plugin `docker compose` (v2).
- Traefik déjà en place, avec :
  - un **réseau Docker** partagé avec les applications (`docker network ls`) ;
  - un **point d'entrée HTTPS** (souvent `websecure`, port 443) ;
  - un **certresolver** (souvent Let's Encrypt) : son nom est dans la configuration
    de Traefik, sous `certificatesResolvers`.
- Une copie du dépôt Raftup sur le serveur (le build part de la racine du dépôt).

Pour retrouver le réseau, le point d'entrée et le certresolver d'un service qui
marche déjà derrière Traefik :

```bash
docker inspect <un-conteneur-derrière-traefik> --format '{{json .Config.Labels}}'
```

## 2. Choisir le domaine (DNS)

1. Choisir un nom, par exemple `hub.exemple.org`.
2. Chez le fournisseur du domaine, créer un enregistrement **A** qui pointe vers
   l'IP publique du serveur (et **AAAA** pour l'IPv6, si le serveur en a une).
3. Attendre que le nom réponde :

```bash
dig +short hub.exemple.org
```

Traefik demande le certificat tout seul au premier accès, si le nom pointe bien
vers le serveur et que le port 80 ou 443 est joignable (selon le défi choisi dans
le certresolver).

## 3. Installer, pas à pas

Toutes les commandes se lancent sur le serveur.

1. Récupérer le dépôt (ou le mettre à jour) :

```bash
git clone https://github.com/Yonako25/raftup.git /opt/raftup
```

```bash
cd /opt/raftup/deploy/rafthub
```

2. Créer le fichier `.env` et le remplir (domaine, réseau, certresolver) :

```bash
cp .env.example .env
```

```bash
nano .env
```

3. Créer le dossier des sauvegardes et le donner à l'utilisateur du conteneur
   (uid 10001) :

```bash
mkdir -p backups && sudo chown 10001:10001 backups
```

4. Construire les images (compte 5 à 15 minutes la première fois) :

```bash
docker compose build
```

5. Démarrer :

```bash
docker compose up -d
```

6. Vérifier que le hub est « healthy » :

```bash
docker compose ps
```

## 4. Premier lancement : le code d'installation

Au premier démarrage, le hub n'a ni base ni admin. Il écrit un **code
d'installation** dans son journal (ADR 0037) :

```bash
docker logs rafthub 2>&1 | grep "code d'installation"
```

Puis, dans un navigateur :

1. ouvrir `https://<votre domaine>/admin` ;
2. choisir la base :
   - **base intégrée** (SQLite dans le volume, rien à installer) : le bon choix
     pour commencer ;
   - **base externe** (PostgreSQL, MySQL, MariaDB) : le serveur de base doit être
     joignable depuis le conteneur (même réseau Docker) ;
3. créer le **premier admin** avec le même code (e-mail, pseudo, mot de passe).

Le code change à chaque redémarrage tant qu'aucun admin n'existe ; il n'est plus
tiré ensuite. Le choix de la base est gardé dans `/data/rafthub.toml` (volume).

Ensuite, dans la console : **Réglages** (nom du hub, inscriptions, SMTP, Discord),
**Distribution du launcher**, **Joueurs**.

Pour Discord, déclarer chez Discord l'adresse de retour
`https://<votre domaine>/v1/oauth/discord/callback`.

## 5. Commandes utiles

Journaux en direct :

```bash
docker compose logs -f rafthub
```

Donner le rôle d'admin à un compte existant :

```bash
docker compose exec rafthub rafthub admin create joueur@exemple.org
```

Mettre à jour (après un `git pull`) :

```bash
docker compose build && docker compose up -d
```

Arrêter (les données restent dans le volume `rafthub-data`) :

```bash
docker compose down
```

## 6. Sauvegarde

### Base intégrée (SQLite)

Le service `backup` copie la base **chaque nuit** (à `BACKUP_HOUR`, 3 h par
défaut) dans le dossier `backups/`, sous la forme `rafthub-AAAAMMJJ-HHMMSS.db.gz`,
avec une copie de `rafthub.toml`. Les sauvegardes de plus de `BACKUP_KEEP_DAYS`
jours (14 par défaut) sont effacées.

La copie passe par `sqlite3 .backup` : elle est cohérente même si le hub écrit
pendant ce temps (base en mode WAL). Elle est vérifiée par `PRAGMA
integrity_check` avant d'être gardée.

Faire une sauvegarde tout de suite :

```bash
docker compose run --rm backup --now
```

Restaurer une sauvegarde (hub arrêté) :

```bash
docker compose stop rafthub
```

```bash
gunzip -c backups/rafthub-AAAAMMJJ-HHMMSS.db.gz > /tmp/rafthub.db
```

```bash
docker run --rm -v rafthub-data:/data -v /tmp/rafthub.db:/restore.db:ro --user 10001:10001 --entrypoint sh rafthub-backup:officiel -c 'rm -f /data/rafthub.db-wal /data/rafthub.db-shm && cp /restore.db /data/rafthub.db'
```

```bash
docker compose start rafthub
```

Pensez à copier aussi `backups/` **hors du serveur** (un autre disque, une autre
machine) : une sauvegarde sur le même disque ne protège pas d'une panne de disque.

### Base externe

Le service `backup` ne fait rien (il le dit dans son journal). Sauvegarder la base
avec l'outil du moteur, par exemple chaque nuit par une tâche planifiée :

- PostgreSQL : `pg_dump -Fc -h <hôte> -U rafthub rafthub > rafthub.dump`
- MySQL / MariaDB : `mysqldump --single-transaction -h <hôte> -u rafthub -p rafthub > rafthub.sql`

Garder aussi `rafthub.toml` du volume : il contient l'adresse de la base (et son
mot de passe : à garder secret).

## 7. Où sont les données ?

Dans le volume Docker `rafthub-data`, monté sur `/data` dans le conteneur
(variable `RAFTUP_HUB_DATA_DIR`) :

- `rafthub.toml` : la base choisie (secret si base externe) ;
- `rafthub.db` (+ `-wal`, `-shm`) : la base intégrée ;
- `cache/` : le launcher officiel en cache (page « Distribution »).

```bash
docker volume inspect rafthub-data
```

## 8. Sécurité

- Le conteneur tourne sans droits (uid 10001), système de fichiers en lecture
  seule sauf `/data`, sans privilège Linux.
- Le code d'installation ne se lit que dans le journal : celui qui le lit a accès
  au serveur. Installer le hub juste après l'avoir lancé.
- Le fichier `.env` peut contenir des secrets (SMTP, Discord) : ne jamais le
  commiter ni le partager. Le plus simple est de régler les secrets dans la
  console (Réglages), qui ne les réaffiche jamais.
