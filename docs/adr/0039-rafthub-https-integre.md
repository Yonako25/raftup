# ADR 0039 — Rafthub : HTTPS intégré, en option

- **Statut** : accepté en autonomie le 20 novembre 2026, à relire par l'utilisatrice
- Complète l'ADR 0033 (le hub parle HTTP en clair, derrière un proxy) et l'ADR 0037 (assistant d'installation) ; même pile TLS que l'ADR 0009

## Contexte

Jusqu'ici, Rafthub ne parlait qu'HTTP : pour du HTTPS, il fallait un reverse proxy (Traefik, Caddy, nginx). Beaucoup de gens qui auto-hébergent n'en ont pas. Comme Forgejo, le hub doit pouvoir servir lui-même en HTTPS, le proxy restant possible.

## Décision

Trois façons de lancer le hub, au choix :

| Mode | Options | Variables |
| --- | --- | --- |
| HTTP en clair (défaut, inchangé) : derrière un proxy | aucune | — |
| HTTPS, certificat fourni | `--tls-cert fullchain.pem --tls-key privkey.pem` | `RAFTUP_HUB_TLS_CERT`, `RAFTUP_HUB_TLS_KEY` |
| HTTPS, Let's Encrypt automatique | `--tls-acme hub.exemple.org [--tls-acme-email moi@exemple.org] [--tls-acme-staging]` | `RAFTUP_HUB_TLS_ACME`, `RAFTUP_HUB_TLS_ACME_EMAIL` |

- **Pile TLS** : `rustls` + `tokio-rustls` avec le moteur `ring`, comme le serveur Archipelago de Raftup (ADR 0009). Pas d'OpenSSL ni d'aws-lc. HTTP/1.1 seulement (ALPN `http/1.1`), comme le hub en clair.
- **Module `hub::tls`** : `TlsSetup` (configuration prête), `serve` (sert un routeur axum en HTTP ou en HTTPS). L'assistant d'installation et le vrai hub passent tous les deux par `serve` : l'installation se fait donc aussi en HTTPS.
- **Poignées de main** dans des tâches à part (un client lent ne bloque pas les autres), 10 s au plus chacune, 256 en même temps au plus. Quand le serveur s'arrête, l'écoute rend le port tout de suite (l'assistant s'arrête puis le vrai hub réécoute sur le même port).
- **Fichiers PEM** : la chaîne entière est gardée ; erreurs claires qui nomment le fichier : fichier illisible (« impossible de lire le certificat … »), plus d'1 Mio, pas de certificat, pas de clé, PEM mal formé, clé qui ne va pas avec le certificat. Le hub refuse de démarrer dans tous ces cas. Pas de rechargement à chaud : un certificat renouvelé (certbot) demande un redémarrage.
- **Let's Encrypt** : crate **`rustls-acme` 0.15** (Apache-2.0 OR MIT), fonctions `ring`, `tokio`, `webpki-roots`, `tls12` (sans aws-lc). Défi **TLS-ALPN-01** : la vérification passe par le port HTTPS lui-même, pas besoin du port 80, mais Let's Encrypt doit joindre le hub sur le **port 443** (`--listen 0.0.0.0:443`, ou une redirection de port). Compte et certificats rangés dans `rafthub-data/acme/`. Une tâche de fond obtient le certificat puis le renouvelle avant son expiration, sans redémarrer (le certificat est choisi à chaque poignée de main). `--tls-acme-staging` utilise le serveur d'essai de Let's Encrypt (certificats non reconnus, limites larges) pour essayer son installation. L'adresse publique par défaut devient `https://<domaine>`.
- **Cookies `Secure`** : nouveau champ `HubConfig::https`. En HTTPS intégré, les cookies de la console et de l'assistant ont toujours l'attribut `Secure`, même quand le hub est joint en `https://localhost`. Sans HTTPS intégré, règle d'avant (pas de `Secure` seulement en `http://127.0.0.1` ou `localhost`).
- **Choix exclusifs** : `--tls-cert` sans `--tls-key` (ou l'inverse), ou fichiers **et** Let's Encrypt ensemble : refus au démarrage avec un message. Un avertissement est écrit si le HTTPS est actif mais l'adresse publique est en `http://`.

Écarté : `axum-server` (un serveur HTTP de plus, alors qu'un petit `Listener` axum suffit) ; `instant-acme` (il faudrait écrire soi-même la commande, le défi et le renouvellement) ; le défi HTTP-01 (il faudrait en plus écouter le port 80).

## Licences

Nouvelles crates, toutes vérifiées le 20 novembre 2026 (`cargo metadata`) : MIT, Apache-2.0 ou les deux (`rustls-acme`, `async-web-client`, `futures-rustls`, `x509-parser`, `asn1-rs`, `der-parser`, `oid-registry`, `nom`, `async-io`, `blocking`, `futures-lite`, `tokio-util`…). Aucune n'est sous une licence plus forte ; le hub reste la seule crate AGPL.

## Conséquences

- Tests (`crates/hub/tests/https.rs`) : certificat auto-signé créé pendant le test avec `rcgen`, client `ureq` qui ne fait confiance qu'à lui ; client normal et HTTP en clair refusés ; port libéré à l'arrêt ; messages d'erreur des fichiers. Cookie `Secure` en HTTPS intégré (`tests/admin.rs`).
- **Pas testé** : la partie Let's Encrypt (aucun appel réseau dans les tests) ; jamais essayée contre le vrai Let's Encrypt, ni même le serveur d'essai. À faire au premier déploiement, avec `--tls-acme-staging` d'abord.
- `rustls-acme` tire une petite pile asynchrone à part (`async-io`, `futures-lite`) en plus de tokio : une trentaine de crates de plus à compiler pour le hub seulement.
- Pas de redirection HTTP → HTTPS sur le port 80 : à ajouter si on le demande (petit serveur qui répond 308).
- Pas de rechargement à chaud des fichiers PEM : à revoir si on le demande (relire les fichiers sur `SIGHUP` ou toutes les 12 h).
