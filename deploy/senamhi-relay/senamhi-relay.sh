#!/bin/sh
# SENAMHI relay: fetch SENAMHI station data from a network SENAMHI accepts, then copy
# the export into the VPS inbox. Run daily by senamhi-relay.timer (systemd user unit).
# Settings come from ~/.config/elnino-relay.env (see senamhi-relay.env.example).
set -eu
. "${ELNINO_RELAY_ENV:-$HOME/.config/elnino-relay.env}"
export DATA_DIR CONTACT
mkdir -p "$DATA_DIR"

# Fails when the station list is unreachable or most stations fail; systemd retries.
"$ELNINO_BIN" run senamhi

"$ELNINO_BIN" export-senamhi "$DATA_DIR/senamhi.json"

# The VPS key only allows rrsync writes into the inbox, so the destination is a bare
# file name. rsync writes a temp file and renames it: the server never reads half a file.
rsync -z --timeout=120 \
  -e "ssh -i $SSH_KEY -o BatchMode=yes -o StrictHostKeyChecking=accept-new" \
  "$DATA_DIR/senamhi.json" "$VPS:senamhi.json"
echo "SENAMHI export uploaded to $VPS"
