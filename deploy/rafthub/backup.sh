#!/bin/sh
# Sauvegarde de la base intégrée du Rafthub (SQLite en mode WAL).
#
#   rafthub-backup --now    une sauvegarde tout de suite, puis s'arrête
#   rafthub-backup --loop   une sauvegarde chaque nuit à BACKUP_HOUR (service Docker)
#
# Copie à chaud sûre : `sqlite3 … ".backup …"` passe par l'API de sauvegarde de
# SQLite, qui lit une image cohérente de la base même pendant que le hub écrit
# (une simple copie du fichier pourrait oublier ce qui est encore dans le -wal).
# La copie est ensuite vérifiée (`PRAGMA integrity_check`), compressée, et les
# sauvegardes de plus de BACKUP_KEEP_DAYS jours sont effacées.
#
# Base externe (PostgreSQL, MySQL, MariaDB) : ce script ne fait rien ; utiliser
# pg_dump ou mysqldump (voir README.md).
set -eu

DATA_DIR="${RAFTUP_HUB_DATA_DIR:-/data}"
BACKUP_DIR="${BACKUP_DIR:-/backups}"
BACKUP_HOUR="${BACKUP_HOUR:-3}"
BACKUP_KEEP_DAYS="${BACKUP_KEEP_DAYS:-14}"
DB="$DATA_DIR/rafthub.db"

log() {
    echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') rafthub-backup : $*"
}

backup_once() {
    if [ ! -f "$DB" ]; then
        log "pas de $DB (hub pas encore installé, ou base externe) : rien à sauvegarder"
        return 0
    fi
    mkdir -p "$BACKUP_DIR"
    stamp="$(date -u '+%Y%m%d-%H%M%S')"
    target="$BACKUP_DIR/rafthub-$stamp.db"
    tmp="$target.tmp"
    rm -f "$tmp"
    # `.timeout` : attend jusqu'à 30 s si le hub tient un verrou d'écriture.
    sqlite3 "$DB" ".timeout 30000" ".backup '$tmp'"
    check="$(sqlite3 "$tmp" 'PRAGMA integrity_check;')"
    if [ "$check" != "ok" ]; then
        log "ÉCHEC : la copie est abîmée ($check), gardée sous $tmp"
        return 1
    fi
    mv "$tmp" "$target"
    gzip -f "$target"
    # rafthub.toml dit quelle base ouvrir : on le garde avec la sauvegarde.
    if [ -f "$DATA_DIR/rafthub.toml" ]; then
        cp "$DATA_DIR/rafthub.toml" "$BACKUP_DIR/rafthub-$stamp.toml"
    fi
    log "sauvegarde écrite : $target.gz"
    # Rotation : on efface les sauvegardes plus vieilles que BACKUP_KEEP_DAYS jours.
    find "$BACKUP_DIR" -maxdepth 1 -type f \
        \( -name 'rafthub-*.db.gz' -o -name 'rafthub-*.toml' \) \
        -mtime +"$BACKUP_KEEP_DAYS" -print -delete | while read -r old; do
        log "ancienne sauvegarde effacée : $old"
    done
}

# Secondes à attendre jusqu'à la prochaine BACKUP_HOUR:00 (heure du conteneur, TZ).
seconds_until_next_run() {
    now="$(date '+%s')"
    next="$(date -d "today $BACKUP_HOUR:00" '+%s')"
    if [ "$next" -le "$now" ]; then
        next="$(date -d "tomorrow $BACKUP_HOUR:00" '+%s')"
    fi
    echo $((next - now))
}

case "${1:---now}" in
    --now)
        backup_once
        ;;
    --loop)
        log "sauvegarde chaque jour à ${BACKUP_HOUR}h, gardée ${BACKUP_KEEP_DAYS} jours, dans $BACKUP_DIR"
        # `trap` : `docker stop` (SIGTERM) arrête l'attente tout de suite.
        trap 'log "arrêt"; exit 0' TERM INT
        while true; do
            wait_s="$(seconds_until_next_run)"
            log "prochaine sauvegarde dans ${wait_s} s"
            # `sleep` en arrière-plan + `wait` : le trap peut couper l'attente.
            sleep "$wait_s" &
            wait $!
            # Une sauvegarde ratée ne doit pas arrêter le service : on réessaiera demain.
            backup_once || log "sauvegarde ratée, nouvel essai à la prochaine échéance"
        done
        ;;
    *)
        echo "usage : rafthub-backup [--now | --loop]" >&2
        exit 2
        ;;
esac
