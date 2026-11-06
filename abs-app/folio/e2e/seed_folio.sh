#!/usr/bin/env bash
# Starts a Folio server for the phone app's tests, filled with the original Audiobookshelf's recorded library: the
# server runs once to make its database, the recorded rows (server/tests/parity/fixtures/abs/_db_end.json) are
# loaded into it by the web fork's seed.py (abs-web/folio/e2e/), with the books' /media paths pointed at MEDIA_DIR,
# where the small test library (server/tests/parity/abs/make_media.sh) must already be. It starts again after.
# Sign in with root / root.
# Usage: seed_folio.sh FOLIO_BINARY DATA_DIR PORT MEDIA_DIR
set -euo pipefail
bin="$1"
data="$2"
port="$3"
media="$(cd "$4" && pwd)"
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

python3 -I "$repo/abs-web/folio/e2e/seed.py" "$repo/server/tests/parity/fixtures/abs/_db_end.json" "$data" "$media"

nohup "$bin" > "$data/folio.log" 2>&1 &
wait_up
echo "Folio is up on port $port with the recorded library"
