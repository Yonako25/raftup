# ADR 0040 — Rafthub officiel : console d'admin invisible, hub public derrière Cloudflare

- **Statut** : accepté le 20 novembre 2026 (demande de l'utilisatrice, proposition validée)
- Complète l'ADR 0035 (adresse du hub dans le launcher), l'ADR 0036 (console d'admin), l'ADR 0038 (Rafthub officiel) et l'ADR 0039 (HTTPS intégré)

## Contexte

Le Rafthub officiel sera public et doit être simple d'accès : un joueur ouvre le launcher et se connecte, sans rien régler. L'utilisatrice veut que la console d'admin soit « full indisponible, voire inconnue » des joueurs, et ne pas exposer son serveur.

On ne peut pas cacher aux launchers l'adresse d'un hub public : un client doit savoir où se connecter, l'adresse se voit sur le réseau, et le code de Raftup est public. Ce qu'on protège, c'est le serveur (son IP) et la console.

## Décision

### Console servie à part (`--admin-listen`)
- Nouvelle option `--admin-listen <adresse>` (ou `RAFTUP_HUB_ADMIN_LISTEN`). Avec elle, le hub ouvre deux écoutes :
  - `--listen` : l'API `/v1` et les pages publiques (`/telecharger`, fichiers sous `/static/`), **sans la console** : `/admin` et tout ce qui est dessous répondent 404 sans corps, exactement comme une adresse qui n'existe pas ;
  - `--admin-listen` : la console `/admin` seule, en HTTP clair (pas l'API).
- Sur le serveur officiel, `--admin-listen` est publié sur l'IP WireGuard de l'hôte (`10.10.0.1:8788`) : la console n'est joignable que par le VPN, qui chiffre. Ses cookies n'ont donc pas l'attribut `Secure` (`HubConfig::admin_separate`), qu'un navigateur refuserait en HTTP.
- L'assistant d'installation (ADR 0037) passe aussi par l'adresse de la console.
- Les pages publiques ne mentionnent plus la console : leurs fichiers statiques sont servis sous `/static/` (les mêmes que `/admin/static/`).
- Sans l'option (Rafthub auto-hébergé), rien ne change : console et API sur la même adresse.
- Code : `hub::public_router`, `hub::admin_router` ; `hub::router` = les deux réunis.

### Déploiement (`deploy/rafthub/`)
- Port 8788 publié sur `RAFTHUB_ADMIN_BIND` (IP du VPN) seulement ; Traefik ne joint que le port 8787 et sa règle exclut `/admin` (seconde barrière).
- Cloudflare en proxy devant `hub.<domaine>` : l'IP du serveur reste cachée et les attaques sont filtrées. Middleware Traefik `ipAllowList` réglé par `RAFTHUB_ALLOWED_SOURCES` (les plages de Cloudflare) : personne ne contourne Cloudflare en visant l'IP. Seul le routeur du hub est concerné, pas les autres services du serveur.

### Adresse du hub officiel dans le launcher
- `DEFAULT_HUB_URL` vient de la variable `RAFTUP_OFFICIAL_HUB_URL` **à la compilation** (`option_env!`) ; sans elle, `http://127.0.0.1:8787` comme avant.
- Le workflow de release la lit dans les variables du dépôt GitHub (`vars.RAFTUP_OFFICIAL_HUB_URL`). Rien n'est écrit en dur dans le code tant que le domaine n'est pas choisi, et seul le nom de domaine est écrit, jamais l'IP.
- Un `rafthub.toml` à côté de l'exe (hub auto-hébergé, ADR 0036) passe toujours avant.

## Conséquences

- Le Rafthub officiel s'administre VPN allumé, à `http://10.10.0.1:8788/admin`.
- Quelqu'un qui scanne le domaine ne trouve aucune trace de la console.
- Il faut tenir à jour la liste des plages Cloudflare (elles changent rarement).
- À faire par l'utilisatrice : acheter le domaine, le confier à Cloudflare, puis créer la variable `RAFTUP_OFFICIAL_HUB_URL` (`https://hub.<domaine>`) dans GitHub.

## Mise à jour du 22 novembre 2026

Le domaine est choisi et le hub officiel est en service (`https://hub.raftup-games.com`). `RAFTUP_OFFICIAL_HUB_URL` est maintenant posée dans `.cargo/config.toml` (section `[env]`) : toutes les compilations ont le hub officiel par défaut, pas seulement la release. Raison : les versions compilées en local visaient `http://127.0.0.1:8787` et n'affichaient aucune illustration (vu en lançant le launcher dans un conteneur). Le workflow de release peut toujours la remplacer par la variable GitHub ; « Par défaut » dans Paramètres revient au hub officiel, et un hub local se choisit à la main.
