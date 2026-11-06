#!/usr/bin/env python3
"""Record how the pinned Readarr answers, for Folio's parity tests.

Usage: record.py BASE_URL OUT_DIR CONFIG_DIR BOOKS_DIR [METADATA_URL]

BASE_URL is the running Readarr (its last image, see UPSTREAM.lock), CONFIG_DIR the folder mounted at /config and
BOOKS_DIR the one mounted at /books, both as this machine sees them.

1. Waits for Readarr, reads its API key from config.xml, adds the root folder /books and the tag `kids`. With
   METADATA_URL, points Readarr's metadata source at it and turns certificate checks off, so that the stand-ins
   of `stub/stub.py` answer its lookups (the workflow sends www.goodreads.com to the Goodreads stand-in).
2. Writes the test library (`seed.sql`) into Readarr's database and its files (`seed-files.json`) into
   BOOKS_DIR. Readarr's metadata server is gone, so authors and books can't be added through its API.
3. Replays requests.json. Each request may use {placeholders} filled from earlier answers (see `save`), and
   `{apikey}`. An entry with `sql` instead of a path writes more rows straight into the database (and its `files`
   into BOOKS_DIR), at that point of the replay.
4. Writes one JSON file per request to OUT_DIR/fixtures/<name>.json:
   {"request": {...}, "status": 200, "content_type": "...", "body": <JSON or text>}
   An entry with `snapshot` writes what the requests so far left behind to OUT_DIR/state/<name>.json: the rows of
   the tables they change (or of its `tables`) and the files and folders under BOOKS_DIR, after waiting its `wait`
   seconds (3 by default) for the work Readarr does after it answers.
5. Copies the database schema (sqlite_master) and the rows Readarr makes on first start to OUT_DIR/schema.sql.

Only the standard library is used, so it runs on any CI runner.
"""
import json
import os
import re
import sqlite3
import sys
import time
import urllib.error
import urllib.request

base, out, config, books = sys.argv[1].rstrip("/"), sys.argv[2], sys.argv[3], sys.argv[4]
metadata_url = sys.argv[5] if len(sys.argv) > 5 else None
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(os.path.join(out, "fixtures"), exist_ok=True)
ctx = {}

# Tables whose rows Readarr writes on first start (profiles, definitions, ...): kept in schema.sql so Folio
# starts with the same ones.
# Tables the requests change, written by `snapshot` entries.
STATE_TABLES = ["Authors", "AuthorMetadata", "Books", "Editions", "BookFiles", "Series", "SeriesBookLink", "History",
                "ImportListExclusions", "ExtraFiles", "MetadataFiles"]
DEFAULT_ROWS = ["QualityProfiles", "MetadataProfiles", "QualityDefinitions", "DelayProfiles", "VersionInfo"]


def call(method, path, body=None, auth=True):
    """Send one request; answer (status, content type, parsed body)."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if auth:
        req.add_header("X-Api-Key", ctx["apikey"])
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            status, ctype, raw = r.status, r.headers.get("Content-Type", ""), r.read()
    except urllib.error.HTTPError as e:
        status, ctype, raw = e.code, e.headers.get("Content-Type", ""), e.read()
    if "json" in ctype:
        try:
            return status, ctype, json.loads(raw)
        except ValueError:
            pass
    if ctype.startswith(("image/", "audio/", "application/octet", "application/epub", "application/zip")):
        return status, ctype, {"bytes": len(raw)}
    return status, ctype, raw.decode("utf-8", "replace")


def dig(value, path):
    """Follow a dotted path like `0.id` through lists and objects."""
    for part in path.split("."):
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def needed(r):
    """The {placeholders} a request uses."""
    return set(re.findall(r"\{([a-z0-9_]+)\}", json.dumps(r.get("body")) + r["path"]))


def fill(text):
    for k, v in ctx.items():
        text = text.replace("{" + k + "}", str(v))
    return text


for _ in range(120):
    try:
        with open(os.path.join(config, "config.xml"), encoding="utf-8") as f:
            ctx["apikey"] = re.search(r"<ApiKey>(\w+)</ApiKey>", f.read()).group(1)
        if call("GET", "/api/v1/system/status")[0] == 200:
            break
    except (OSError, AttributeError):
        pass
    time.sleep(1)
else:
    sys.exit("Readarr did not start")

print("root folder:", call("POST", "/api/v1/rootfolder", {
    "name": "Books", "path": "/books/", "defaultQualityProfileId": 1, "defaultMetadataProfileId": 1,
    "defaultMonitorOption": "all", "defaultNewItemMonitorOption": "all", "defaultTags": [], "isCalibreLibrary": False,
})[0])
print("tag:", call("POST", "/api/v1/tag", {"label": "kids"})[0])
if metadata_url:
    for section, change in (("development", {"metadataSource": metadata_url}), ("host", {"certificateValidation": "disabled"})):
        settings = call("GET", f"/api/v1/config/{section}")[2]
        settings.update(change)
        print(f"{section} settings:", call("PUT", f"/api/v1/config/{section}/{settings['id']}", settings)[0])


def write_files(files):
    """Make each (path under /books, size) file in BOOKS_DIR, filled with zeros."""
    for rel, size in files:
        path = os.path.join(books, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(b"\0" * size)


def snapshot(name, tables=None, wait=3):
    """Write the rows of the tables the requests change (or of `tables`), and the files and folders under
    BOOKS_DIR, to OUT_DIR/state/<name>.json. Readarr finishes some of its work after it answers (an author's
    refresh after it is added), so this waits `wait` seconds first."""
    time.sleep(wait)
    con = sqlite3.connect(f"file:{os.path.join(config, 'readarr.db')}?mode=ro", uri=True, timeout=30)
    con.row_factory = sqlite3.Row
    state = {"tables": {t: [dict(row) for row in con.execute(f'SELECT * FROM "{t}" ORDER BY Id')]
                        for t in tables or STATE_TABLES},
             "files": []}
    con.close()
    for root, dirs, files in os.walk(books):
        state["files"] += [os.path.relpath(os.path.join(root, n), books) + "/" for n in dirs]
        state["files"] += [os.path.relpath(os.path.join(root, n), books) for n in files]
    state["files"].sort()
    os.makedirs(os.path.join(out, "state"), exist_ok=True)
    with open(os.path.join(out, "state", name + ".json"), "w", encoding="utf-8") as f:
        json.dump(state, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")


def run_sql(sql):
    """Write rows straight into Readarr's database."""
    con = sqlite3.connect(os.path.join(config, "readarr.db"), timeout=30)
    con.executescript(sql)
    con.commit()
    con.close()


# The test library's files ([path under /books, size]); seed.sql's BookFiles point at them.
with open(os.path.join(here, "seed-files.json"), encoding="utf-8") as f:
    write_files(json.load(f))
with open(os.path.join(here, "seed.sql"), encoding="utf-8") as f:
    run_sql(f.read())
print("library seeded")

with open(os.path.join(here, "requests.json"), encoding="utf-8") as f:
    requests = json.load(f)
for r in requests:
    if "sql" in r:
        write_files(r.get("files", []))
        run_sql(r["sql"])
        print(f"wrote {r['name']}")
        continue
    if r.get("snapshot"):
        snapshot(r["name"], r.get("tables"), r.get("wait", 3))
        print(f"state {r['name']}")
        continue
    missing = [k for k in needed(r) if k not in ctx]
    if missing:
        print(f"skipped {r['name']}: no {', '.join(missing)} from an earlier answer")
        continue
    path = fill(r["path"])
    body = json.loads(fill(json.dumps(r["body"]))) if "body" in r else None
    status, ctype, answer = call(r.get("method", "GET"), path, body, auth=r.get("auth", True))
    for key, where in r.get("save", {}).items():
        try:
            ctx[key] = dig(answer, where)
        except (KeyError, IndexError, TypeError, ValueError):
            print(f"warning: {r['name']} has no {where}; requests that need {{{key}}} are skipped")
    record = {"request": {"method": r.get("method", "GET"), "path": r["path"], "body": r.get("body")},
              "status": status, "content_type": ctype.split(";")[0], "body": answer}
    with open(os.path.join(out, "fixtures", r["name"] + ".json"), "w", encoding="utf-8") as f:
        json.dump(record, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")
    print(f"{status} {r.get('method', 'GET')} {path} -> {r['name']}")

con = sqlite3.connect(f"file:{os.path.join(config, 'readarr.db')}?mode=ro", uri=True)
with open(os.path.join(out, "schema.sql"), "w", encoding="utf-8") as f:
    for (sql,) in con.execute("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' "
                              "ORDER BY type = 'index', rowid"):
        f.write(sql.strip() + ";\n")
    for t in DEFAULT_ROWS:
        cols = [c[1] for c in con.execute(f'PRAGMA table_info("{t}")')]
        for row in con.execute(f'SELECT * FROM "{t}" ORDER BY rowid'):
            vals = ", ".join("NULL" if v is None else str(v) if isinstance(v, (int, float)) else
                             "'" + str(v).replace("'", "''") + "'" for v in row)
            f.write(f'INSERT INTO "{t}" ({", ".join(chr(34) + c + chr(34) for c in cols)}) VALUES ({vals});\n')
print("schema written")
