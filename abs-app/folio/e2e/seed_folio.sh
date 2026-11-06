#!/usr/bin/env bash
# Starts a Folio server for the phone app's tests, filled with the original Audiobookshelf's recorded library: the
# server runs once to make its database, the recorded rows (server/tests/parity/fixtures/abs/_db_end.json) are
# loaded into it, and it starts again. The recorded books point at /media, so the small test library
# (server/tests/parity/abs/make_media.sh) must already be there. Sign in with root / root.
# Usage: seed_folio.sh FOLIO_BINARY DATA_DIR PORT
set -euo pipefail
bin="$1"
data="$2"
port="$3"
repo="$(cd "$(dirname "$0")/../../.." && pwd)"
export APP_DATA_DIR="$data" APP_STATIC_DIR="$data/static" APP_PORT="$port"

wait_up() {
    for _ in $(seq 1 60); do
        curl -sf "http://localhost:$port/status" > /dev/null && return 0
        sleep 1
    done
    echo "Folio did not start; its log:" >&2
    cat "$data/folio.log" >&2
    return 1
}

rm -rf "$data"
mkdir -p "$data/static"
"$bin" > "$data/folio.log" 2>&1 &
wait_up
kill "$!"
wait "$!" 2> /dev/null || true

python3 -I - "$repo/server/tests/parity/fixtures/abs/_db_end.json" "$data/absdatabase.sqlite" <<'PY'
import json, sqlite3, sys
rows = json.load(open(sys.argv[1]))
db = sqlite3.connect(sys.argv[2])
db.execute("PRAGMA foreign_keys=OFF")
for table, records in sorted(rows.items()):
    # the server's own settings and the original's sessions and migrations stay as Folio made them
    if table in ("settings", "sessions", "migrationsMeta", "SequelizeMeta"):
        continue
    db.execute(f"DELETE FROM `{table}`")
    for r in records:
        values = [json.dumps(v) if isinstance(v, (dict, list)) else v for v in r.values()]
        cols = ",".join(f"`{k}`" for k in r)
        db.execute(f"INSERT INTO `{table}` ({cols}) VALUES ({','.join('?' * len(r))})", values)
db.commit()
PY

nohup "$bin" > "$data/folio.log" 2>&1 &
wait_up
echo "Folio is up on port $port with the recorded library"
