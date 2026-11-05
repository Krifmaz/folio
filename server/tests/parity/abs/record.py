#!/usr/bin/env python3
"""Record how the pinned Audiobookshelf answers, for Folio's parity tests.

Usage: record.py BASE_URL OUT_DIR DB_PATH

Placeholders always known: {library}, {token} (root's access token) and {legacy_token} (root's old token).

1. Waits for the server, creates the root user (root / root), signs in, adds a book library on /media and scans it.
2. Replays requests.json. Each request may use {placeholders} filled from earlier answers (see `save`).
3. Writes one JSON file per request to OUT_DIR/fixtures/<name>.json:
   {"request": {...}, "status": 200, "content_type": "...", "body": <JSON or text>, "cookies": [...]}
   (`cookies`: the cookies the answer set, as `name; attributes`, without their values)
   Also `lib_create.json` (the library made on /media) and `_db_scan.json` (every row right after the first scan).
4. Copies the database schema (sqlite_master) to OUT_DIR/schema.sql.

Only the standard library is used, so it runs on any CI runner.
"""
import json
import os
import sqlite3
import sys
import time
import urllib.error
import urllib.request

base, out, db_path = sys.argv[1].rstrip("/"), sys.argv[2], sys.argv[3]
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(os.path.join(out, "fixtures"), exist_ok=True)
ctx = {}
token = None


def cookies_of(headers):
    """The cookies an answer sets, without their values: `name; attributes` (values are tokens)."""
    out = []
    for c in headers.get_all("Set-Cookie") or []:
        name, _, rest = c.partition("=")
        attrs = [a.strip() for a in rest.split(";")[1:] if not a.strip().lower().startswith("expires=")]
        out.append("; ".join([name.strip()] + attrs))
    return sorted(out)


last_cookies = []


def call(method, path, body=None, auth=True, headers=None):
    """Send one request; answer (status, content type, parsed body). The cookies it set go to `last_cookies`."""
    global last_cookies
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method)
    if data is not None:
        req.add_header("Content-Type", "application/json")
    if auth and token:
        req.add_header("Authorization", "Bearer " + token)
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            status, ctype, raw = r.status, r.headers.get("Content-Type", ""), r.read()
            last_cookies = cookies_of(r.headers)
    except urllib.error.HTTPError as e:
        status, ctype, raw = e.code, e.headers.get("Content-Type", ""), e.read()
        last_cookies = cookies_of(e.headers)
    if "json" in ctype:
        try:
            return status, ctype, json.loads(raw)
        except ValueError:
            pass
    if ctype.startswith(("image/", "audio/", "application/octet", "application/epub", "application/zip")):
        return status, ctype, {"bytes": len(raw)}
    return status, ctype, raw.decode("utf-8", "replace")


def dig(value, path):
    """Follow a dotted path like `libraries.0.id` through lists and objects."""
    for part in path.split("."):
        value = value[int(part)] if isinstance(value, list) else value[part]
    return value


def ctx_needed(r):
    """The {placeholders} a request uses."""
    import re
    return set(re.findall(r"\{(?:b64:)?([a-z0-9_]+)\}", json.dumps(r.get("body")) + json.dumps(r.get("headers")) + r["path"]))


def fill(text):
    """Fill {key} with a saved value, and {b64:key} with it base64-encoded for a URL (library page filters)."""
    import base64
    import urllib.parse
    for k, v in ctx.items():
        text = text.replace("{" + k + "}", str(v))
        text = text.replace("{b64:" + k + "}", urllib.parse.quote(base64.b64encode(str(v).encode()).decode(), safe=""))
    return text


for _ in range(120):
    try:
        if call("GET", "/status", auth=False)[0] == 200:
            break
    except OSError:
        pass
    time.sleep(1)
else:
    sys.exit("the server did not start")

print("init:", call("POST", "/init", {"newRoot": {"username": "root", "password": "root"}}, auth=False)[0])
status, _, login = call("POST", "/login", {"username": "root", "password": "root"}, auth=False)
if status != 200:
    sys.exit(f"login failed: {status} {login}")
token = login["user"].get("accessToken") or login["user"]["token"]
ctx["token"] = token
ctx["legacy_token"] = login["user"]["token"]
create_body = {"name": "Books", "folders": [{"fullPath": "/media"}], "mediaType": "book"}
status, ctype, lib = call("POST", "/api/libraries", create_body)
print("library:", status)
with open(os.path.join(out, "fixtures", "lib_create.json"), "w", encoding="utf-8") as f:
    json.dump({"request": {"method": "POST", "path": "/api/libraries", "body": create_body}, "status": status,
               "content_type": ctype.split(";")[0], "body": lib}, f, indent=1, sort_keys=True, ensure_ascii=False)
    f.write("\n")
ctx["library"] = lib["id"]
call("POST", f"/api/libraries/{ctx['library']}/scan")
for _ in range(120):
    items = call("GET", f"/api/libraries/{ctx['library']}/items")[2]
    if isinstance(items, dict) and items.get("total", 0) >= 3:
        break
    time.sleep(1)
else:
    sys.exit("the scan did not find the three test books")
time.sleep(3)  # let the scan finish writing authors and series


def snapshot(name):
    """Every row of every table, written to OUT_DIR/fixtures/<name>.json."""
    snap = sqlite3.connect(":memory:")
    sqlite3.connect(f"file:{db_path}?mode=ro", uri=True).backup(snap)
    rows = {}
    for (t,) in snap.execute("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        cur = snap.execute(f'SELECT * FROM "{t}" ORDER BY rowid')
        cols = [d[0] for d in cur.description]
        rows[t] = [dict(zip(cols, r)) for r in cur]
    with open(os.path.join(out, "fixtures", name + ".json"), "w", encoding="utf-8") as f:
        json.dump(rows, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")


# The rows right after the first scan, for the scanner's parity test (before any request changes them).
snapshot("_db_scan")

with open(os.path.join(here, "requests.json"), encoding="utf-8") as f:
    requests = json.load(f)
for r in requests:
    missing = [k for k in ctx_needed(r) if k not in ctx]
    if missing:
        print(f"skipped {r['name']}: no {', '.join(missing)} from an earlier answer")
        continue
    path = fill(r["path"])
    body = json.loads(fill(json.dumps(r["body"]))) if "body" in r else None
    headers = json.loads(fill(json.dumps(r["headers"]))) if "headers" in r else None
    status, ctype, answer = call(r.get("method", "GET"), path, body, auth=r.get("auth", True), headers=headers)
    for key, where in r.get("save", {}).items():
        try:
            ctx[key] = dig(answer, where)
        except (KeyError, IndexError, TypeError, ValueError):
            print(f"warning: {r['name']} has no {where}; requests that need {{{key}}} are skipped")
    record = {"request": {"method": r.get("method", "GET"), "path": r["path"], "body": r.get("body")},
              "status": status, "content_type": ctype.split(";")[0], "body": answer}
    if "headers" in r:
        record["request"]["headers"] = r["headers"]
    if last_cookies:
        record["cookies"] = last_cookies
    with open(os.path.join(out, "fixtures", r["name"] + ".json"), "w", encoding="utf-8") as f:
        json.dump(record, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")
    print(f"{status} {r.get('method', 'GET')} {path} -> {r['name']}")

with open(os.path.join(out, "context.json"), "w", encoding="utf-8") as f:
    json.dump(ctx, f, indent=1, sort_keys=True)

# Every row of every table once all requests ran, so a test can load the original's own data into Folio and
# replay the last requests (the read-only ones at the end of requests.json) against the same state.
time.sleep(2)
snapshot("_db_end")
print("database rows written")
con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
with open(os.path.join(out, "schema.sql"), "w", encoding="utf-8") as f:
    for typ, name, sql in con.execute("SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type DESC, name"):
        f.write(sql.strip() + ";\n\n")
    tables = [n for (n,) in con.execute("SELECT name FROM sqlite_master WHERE type = 'table' AND lower(name) LIKE '%meta%'")]
    for t in tables:  # the migrations bookkeeping, so Folio can write the same rows
        f.write(f"-- rows of {t}\n")
        for row in con.execute(f'SELECT * FROM "{t}"'):
            f.write(f"-- {json.dumps(row)}\n")
print("schema written")
