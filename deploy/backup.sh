#!/bin/sh
# Nightly SQLite backup from the running container. Example cron entry:
#   15 3 * * * /opt/elnino/deploy/backup.sh >> /var/log/elnino-backup.log 2>&1
set -eu
DEST="${BACKUP_DIR:-/var/backups/elnino}"
KEEP_DAYS="${KEEP_DAYS:-14}"
mkdir -p "$DEST"
STAMP="$(date +%Y%m%d)"
docker exec elnino sqlite3 /data/elnino.db ".backup '/data/backup-$STAMP.db'"
docker cp "elnino:/data/backup-$STAMP.db" "$DEST/elnino-$STAMP.db"
docker exec elnino rm -f "/data/backup-$STAMP.db"
gzip -f "$DEST/elnino-$STAMP.db"
find "$DEST" -name 'elnino-*.db.gz' -mtime +"$KEEP_DAYS" -delete
echo "backup ok: $DEST/elnino-$STAMP.db.gz"
