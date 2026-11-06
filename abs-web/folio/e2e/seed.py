#!/usr/bin/env python3
"""Fill a Folio data folder with the library the parity recorder made in the original (its `_db_end.json`), so the
web fork's pages can be checked against Folio with real books: the rows of every table but the server settings and
sign-in sessions, with the original's `/media` paths pointed at MEDIA_DIR (made by
`server/tests/parity/abs/make_media.sh`). The user `root` signs in with the password `root`.

Run it on a data folder Folio has opened once (so the database has its tables) while the server is stopped:
  python3 -I abs-web/folio/e2e/seed.py DB_END_JSON DATA_DIR MEDIA_DIR
"""
import json
import os
import sqlite3
import sys

db_end, data, media = sys.argv[1], sys.argv[2], os.path.abspath(sys.argv[3])
SKIP = {"settings", "sessions", "migrationsMeta", "SequelizeMeta"}


def local(value):
    """The value with the original's media folder replaced by the local one (also inside JSON text)."""
    if isinstance(value, str):
        return value.replace('"/media/', '"' + media + "/").replace('"/media"', '"' + media + '"') if value[:1] in "[{" else (
            media + value[len("/media"):] if value == "/media" or value.startswith("/media/") else value)
    return value


tables = json.load(open(db_end, encoding="utf-8"))
con = sqlite3.connect(os.path.join(data, "absdatabase.sqlite"))
con.execute("PRAGMA foreign_keys = OFF")
for table, rows in tables.items():
    if table in SKIP:
        continue
    con.execute(f'DELETE FROM "{table}"')
    for row in rows:
        cols = ", ".join(f'"{c}"' for c in row)
        marks = ", ".join("?" for _ in row)
        con.execute(f'INSERT INTO "{table}" ({cols}) VALUES ({marks})', [local(v) for v in row.values()])
con.commit()
print(f"seeded {sum(len(r) for t, r in tables.items() if t not in SKIP)} rows from {db_end}")
