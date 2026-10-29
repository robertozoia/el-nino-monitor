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

# INBOX is the inbox path on the VPS (default /opt/el-nino/inbox). Set it empty when the
# key is restricted with rrsync, which already roots paths at the inbox. SSH_KEY is
# optional: without it ssh uses your usual keys (which must not need a passphrase
# prompt, since the timer runs without a terminal or agent).
INBOX="${INBOX-/opt/el-nino/inbox}"
DEST="${INBOX:+$INBOX/}senamhi.json"
SSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new"
if [ -n "${SSH_KEY:-}" ]; then
  SSH="$SSH -i $SSH_KEY"
fi
# rsync writes a temp file and renames it: the server never reads half a file.
rsync -z --timeout=120 -e "$SSH" "$DATA_DIR/senamhi.json" "$VPS:$DEST"
echo "SENAMHI export uploaded to $VPS:$DEST"
