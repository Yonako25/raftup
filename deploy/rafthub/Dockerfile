# Image Docker du Rafthub officiel (binaire `rafthub`, AGPL-3.0-or-later, ADR 0003).
#
# À construire depuis la RACINE du dépôt (le hub a besoin du workspace entier et des
# polices de `crates/launcher-ui/fonts/`) :
#
#   docker build -f deploy/rafthub/Dockerfile -t rafthub:officiel .
#
# Trois étapes :
#   1. `build`   : compile `rafthub` en release avec l'option `officiel` (ADR 0038) ;
#   2. `runtime` : image finale, Debian slim + le binaire, utilisateur non root ;
#   3. `backup`  : petite image de sauvegarde (sqlite3 + script), voir backup.sh.
# La cible par défaut est `runtime` (dernière étape nommée en `--target` sinon).

# Version de Rust et de Debian. Debian 13 (trixie) partout : même glibc à la
# compilation et à l'exécution.
ARG RUST_VERSION=1.91
ARG DEBIAN_VERSION=trixie

# ---------------------------------------------------------------------------
# 1. Compilation
# ---------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-slim-${DEBIAN_VERSION} AS build
WORKDIR /src
COPY . .
# Les « cache mounts » gardent le registre cargo et `target/` entre deux builds
# (beaucoup plus rapide), mais ne vont pas dans l'image : le binaire est donc
# copié hors de `target/` dans la même commande.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release -p hub --bin rafthub --features officiel \
 && install -D -m 0755 target/release/rafthub /out/rafthub

# ---------------------------------------------------------------------------
# 2. Image finale du hub
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_VERSION}-slim AS runtime
# Certificats racine : SMTP en TLS et appels HTTPS à Discord.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# Utilisateur sans droits, uid/gid fixes (10001) : le volume de données et le
# dossier des sauvegardes lui appartiennent.
RUN groupadd --system --gid 10001 rafthub \
 && useradd --system --uid 10001 --gid rafthub --home-dir /data --shell /usr/sbin/nologin rafthub \
 && mkdir -p /data \
 && chown rafthub:rafthub /data
COPY --from=build /out/rafthub /usr/local/bin/rafthub
COPY crates/hub/LICENSE-AGPL /usr/share/doc/rafthub/LICENSE-AGPL

# Dossier de données sur le volume (ADR 0037) : rafthub.toml, rafthub.db, cache/.
ENV RAFTUP_HUB_DATA_DIR=/data \
    RUST_LOG=info
VOLUME ["/data"]
USER rafthub:rafthub
# 8787 : API et pages publiques ; 8788 : console d'admin avec --admin-listen (ADR 0040).
EXPOSE 8787 8788
# La sonde est le binaire lui-même (pas de curl dans l'image) ; elle répond « sain »
# aussi pendant l'assistant de premier lancement.
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD ["/usr/local/bin/rafthub", "healthcheck", "--listen", "127.0.0.1:8787"]
ENTRYPOINT ["/usr/local/bin/rafthub"]
# Dans le conteneur, le hub écoute sur toutes les interfaces : seul Traefik le joint,
# par le réseau Docker (aucun port publié sur l'hôte).
CMD ["--listen", "0.0.0.0:8787"]

# ---------------------------------------------------------------------------
# 3. Image de sauvegarde (service `backup` du docker-compose.yml)
# ---------------------------------------------------------------------------
FROM debian:${DEBIAN_VERSION}-slim AS backup
RUN apt-get update \
 && apt-get install -y --no-install-recommends sqlite3 gzip \
 && rm -rf /var/lib/apt/lists/*
# Même uid/gid que le hub : les fichiers -wal / -shm de SQLite restent à lui.
RUN groupadd --system --gid 10001 rafthub \
 && useradd --system --uid 10001 --gid rafthub --no-create-home --shell /usr/sbin/nologin rafthub \
 && mkdir -p /backups \
 && chown rafthub:rafthub /backups
COPY deploy/rafthub/backup.sh /usr/local/bin/rafthub-backup
RUN chmod 0755 /usr/local/bin/rafthub-backup
USER rafthub:rafthub
ENTRYPOINT ["/usr/local/bin/rafthub-backup"]
CMD ["--loop"]

# Cible par défaut : l'image du hub.
FROM runtime
