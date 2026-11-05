#!/usr/bin/env python3
"""Build the rows of docs/PARITY.md from checkouts of the pinned upstreams (see UPSTREAM.lock).

Usage: scripts/parity-inventory.py ABS_DIR ABS_APP_DIR READARR_DIR > rows.md
Every row starts as todo (or n/a where GOALS replaces it). After an upstream bump, diff the new rows against
docs/PARITY.md and add what is new; never overwrite the status column.
"""
import json
import os
import re
import sys

abs_dir, app_dir, rd_dir = sys.argv[1:4]
rows = []  # (origin, kind, item, src, track)


def add(origin, kind, item, src, track):
    rows.append((origin, kind, item, src, track))


def rd(p):
    with open(p, encoding="utf-8", errors="replace") as f:
        return f.read()


# ---------- Audiobookshelf server ----------
ABS_TRACK = [
    (r"/play$|/play/|/progress|^/api/(session|sessions|playback)|listening-|bookmark|^/hls|^/public/session", "P3 playback"),
    (r"^/api/(libraries|filesystem|scan)", "P1 library"),
    (r"^/api/(items|library-items)", "P1 library"),
    (r"^/api/(authors|series|search|tags|genres)", "P1 library"),
    (r"^/api/(me|users|auth-settings|auth|sessions?)|^/(login|logout|init|status|ping|healthcheck)|^/auth", "P2 users"),
    (r"^/api/(session|playback|sessions)|^/hls|^/public/session", "P3 playback"),
    (r"^/api/(collections|playlists)", "P4 collections"),
    (r"^/api/(podcasts|episodes?)|episode-downloads|^/feed|^/api/feeds", "P5 podcasts and feeds"),
    (r"^/api/(backups|notifications|emails|cache|tools|logger-data|watcher|validate-cron|custom-metadata-providers|settings|sorting-prefixes|stats|upload|share)|^/public/share", "P6 server admin"),
    (r"^/api/(search/covers|search/books|search/podcast|search/authors|search/chapters|search/providers)", "P7 metadata match"),
]


def abs_track(path):
    for pat, t in ABS_TRACK:
        if re.search(pat, path):
            return t
    return "P6 server admin"


route_re = re.compile(r"this\.router\.(get|post|patch|put|delete)\(\s*'([^']+)'\s*,\s*(.*)\)")
api = os.path.join(abs_dir, "server/routers/ApiRouter.js")
for i, line in enumerate(rd(api).splitlines(), 1):
    m = route_re.search(line)
    if not m:
        continue
    meth, path, rest = m.groups()
    handler = re.findall(r"([A-Za-z]+Controller\.[A-Za-z]+|this\.[A-Za-z]+)", rest)
    h = handler[-1] if handler else ""
    full = "/api" + path
    add("ABS", "endpoint", f"`{meth.upper()} {full}` ({h})", f"server/routers/ApiRouter.js#L{i}", abs_track(full))
for fn, prefix in (("HlsRouter.js", "/hls"), ("PublicRouter.js", "/public")):
    p = os.path.join(abs_dir, "server/routers", fn)
    for i, line in enumerate(rd(p).splitlines(), 1):
        m = route_re.search(line)
        if m:
            meth, path, rest = m.groups()
            full = prefix + path
            add("ABS", "endpoint", f"`{meth.upper()} {full}`", f"server/routers/{fn}#L{i}", abs_track(full))
plain_re = re.compile(r"router\.(get|post|patch|put|delete)\(\s*'([^']+)'")
for fn in ("server/Server.js", "server/Auth.js"):
    for i, line in enumerate(rd(os.path.join(abs_dir, fn)).splitlines(), 1):
        if "this.router" in line:
            continue
        m = plain_re.search(line)
        if m:
            meth, path = m.groups()
            add("ABS", "endpoint", f"`{meth.upper()} {path}`", f"{fn}#L{i}", abs_track(path))
# socket.io
sa = rd(os.path.join(abs_dir, "server/SocketAuthority.js"))
for i, line in enumerate(sa.splitlines(), 1):
    m = re.search(r"socket\.on\('([a-z_]+)'", line)
    if m:
        add("ABS", "socket", f"client → server `{m.group(1)}`", f"server/SocketAuthority.js#L{i}", "P2 users")
emits = {}
for root, _, files in os.walk(os.path.join(abs_dir, "server")):
    for f in files:
        if not f.endswith(".js"):
            continue
        p = os.path.join(root, f)
        for i, line in enumerate(rd(p).splitlines(), 1):
            for m in re.finditer(r"(?:emitter|clientEmitter|userEmitter|adminEmitter|libraryItemEmitter|libraryItemsEmitter)\('([a-z_]+)'", line):
                emits.setdefault(m.group(1), f"{os.path.relpath(p, abs_dir)}#L{i}")
for ev in sorted(emits):
    add("ABS", "socket", f"server → client `{ev}`", emits[ev], "P2 users")

# settings
ss = rd(os.path.join(abs_dir, "server/objects/settings/ServerSettings.js"))
seen = set()
for i, line in enumerate(ss.splitlines(), 1):
    m = re.match(r"\s*this\.([a-zA-Z]+) = (.*)", line)
    if m and m.group(1) not in seen and m.group(1) not in ("id",):
        seen.add(m.group(1))
        add("ABS", "setting", f"server `{m.group(1)}` (default `{m.group(2).split('//')[0].strip()[:40]}`)", f"server/objects/settings/ServerSettings.js#L{i}", "P6 server admin")
    if "construct(" in line or "toJSON" in line:
        break
for fn, label, tr in (("EmailSettings.js", "email", "P6 server admin"), ("NotificationSettings.js", "notifications", "P6 server admin")):
    seen = set()
    for i, line in enumerate(rd(os.path.join(abs_dir, "server/objects/settings", fn)).splitlines(), 1):
        m = re.match(r"\s*this\.([a-zA-Z]+) = ", line)
        if m and m.group(1) not in seen and m.group(1) != "id":
            seen.add(m.group(1))
            add("ABS", "setting", f"{label} `{m.group(1)}`", f"server/objects/settings/{fn}#L{i}", tr)
lib = rd(os.path.join(abs_dir, "server/models/Library.js"))
m = re.search(r"getDefaultLibrarySettingsForMediaType\(mediaType\) \{(.*?)\n  \}", lib, re.S)
if m:
    start = lib[: m.start()].count("\n") + 1
    seen = set()
    for j, line in enumerate(m.group(1).splitlines()):
        k = re.match(r"\s*([a-zA-Z]+):", line)
        if k and k.group(1) not in seen:
            seen.add(k.group(1))
            add("ABS", "setting", f"library `{k.group(1)}`", f"server/models/Library.js#L{start + j}", "P1 library")
usr = rd(os.path.join(abs_dir, "server/models/User.js"))
m = re.search(r"getDefaultPermissionsForUserType\(type\) \{(.*?)\n  \}", usr, re.S)
if m:
    start = usr[: m.start()].count("\n") + 1
    seen = set()
    for j, line in enumerate(m.group(1).splitlines()):
        k = re.match(r"\s*([a-zA-Z]+):", line)
        if k and k.group(1) not in seen:
            seen.add(k.group(1))
            add("ABS", "setting", f"user permission `{k.group(1)}`", f"server/models/User.js#L{start + j}", "P2 users")
# notification events
ne = os.path.join(abs_dir, "server/utils/notifications.js")
if os.path.exists(ne):
    for i, line in enumerate(rd(ne).splitlines(), 1):
        m = re.search(r"name: '([a-zA-Z_]+)'", line)
        if m:
            add("ABS", "job", f"notification event `{m.group(1)}`", f"server/utils/notifications.js#L{i}", "P6 server admin")

# background jobs
jobs = [
    ("server/managers/CronManager.js", "library scan schedules (`cronExpression` per library)", "P1 library"),
    ("server/managers/CronManager.js", "podcast episode checks (`podcastEpisodeSchedule`, per podcast cron)", "P5 podcasts and feeds"),
    ("server/managers/BackupManager.js", "scheduled backups (`backupSchedule`, `backupsToKeep`, `maxBackupSize`)", "P6 server admin"),
    ("server/managers/LogManager.js", "daily log files and rotation (`loggerDailyLogsToKeep`, `loggerScannerLogsToKeep`)", "P6 server admin"),
    ("server/Watcher.js", "folder watcher: rescans changed items (`scannerDisableWatcher`)", "P1 library"),
    ("server/managers/PlaybackSessionManager.js", "stale open sessions closed (`removeOrphanStreams`, idle sessions)", "P3 playback"),
    ("server/managers/AudioMetadataManager.js", "embed-metadata queue (Quick Embed)", "P6 server admin"),
    ("server/managers/AbMergeManager.js", "M4B merge tasks", "P6 server admin"),
    ("server/managers/CacheManager.js", "cover and item cache purge", "P6 server admin"),
    ("server/managers/RssFeedManager.js", "open RSS feeds kept in step with items", "P5 podcasts and feeds"),
    ("server/managers/PodcastManager.js", "episode download queue", "P5 podcasts and feeds"),
    ("server/managers/MigrationManager.js", "database migrations on start", "P6 server admin"),
    ("server/managers/BinaryManager.js", "ffmpeg/ffprobe presence check and download", "P6 server admin"),
    ("server/Server.js", "server start: purge (`cleanUserData`, `purgeMetadata`)", "P6 server admin"),
]
for src, item, tr in jobs:
    add("ABS", "job", item, src, tr)
scanner = os.path.join(abs_dir, "server/scanner")
for f in sorted(os.listdir(scanner)):
    if f.endswith(".js"):
        add("ABS", "job", f"scanner: `{f[:-3]}`", f"server/scanner/{f}", "P1 library")

# client pages
pages = os.path.join(abs_dir, "client/pages")
for root, _, files in os.walk(pages):
    for f in sorted(files):
        if f.endswith(".vue"):
            rel = os.path.relpath(os.path.join(root, f), pages)
            route = "/" + rel[:-4].replace("index", "").replace("_", ":").rstrip("/")
            add("ABS web", "page", f"`{route or '/'}`", f"client/pages/{rel}", "W web fork")
# client context-menu actions
acts = {}
for root, _, files in os.walk(os.path.join(abs_dir, "client")):
    if "node_modules" in root:
        continue
    for f in files:
        if f.endswith(".vue"):
            p = os.path.join(root, f)
            for i, line in enumerate(rd(p).splitlines(), 1):
                for m in re.finditer(r"action: '([a-zA-Z-]+)'", line):
                    acts.setdefault(m.group(1), f"{os.path.relpath(p, abs_dir)}#L{i}")
for a in sorted(acts):
    add("ABS web", "action", f"menu action `{a}`", acts[a], "W web fork")

# ---------- Audiobookshelf app ----------
apages = os.path.join(app_dir, "pages")
for root, _, files in os.walk(apages):
    for f in sorted(files):
        if f.endswith(".vue"):
            rel = os.path.relpath(os.path.join(root, f), apages)
            route = "/" + rel[:-4].replace("index", "").replace("_", ":").rstrip("/")
            add("ABS app", "page", f"`{route or '/'}`", f"pages/{rel}", "M mobile fork")
st = rd(os.path.join(app_dir, "pages/settings.vue"))
for s in sorted(set(re.findall(r"settings\.([a-zA-Z]+)", st))):
    add("ABS app", "setting", f"device `{s}`", "pages/settings.vue", "M mobile fork")
for feat in ("offline downloads and local media", "Android Auto browse tree", "media notification and lock-screen controls",
             "sleep timer with shake to reset", "Chromecast", "local progress sync with the server", "ebook reader (EPUB, PDF, comic)"):
    add("ABS app", "action", feat, "android/, plugins/", "M mobile fork")

# ---------- Readarr ----------
RD_TRACK = [
    (r"/(author|book|edition|bookfile|series|bookshelf|manualimport|rename|retag|mediacover|search|parse)", "R1 authors and books"),
    (r"/(wanted|queue|history|blocklist|release|calendar|command)", "R2 wanted and downloads"),
    (r"/(qualityprofile|qualitydefinition|metadataprofile|delayprofile|releaseprofile|customformat|customfilter|language)", "R3 profiles"),
    (r"/(indexer|downloadclient|importlist|notification|metadata|remotepathmapping|rootfolder|tag)", "R4 providers"),
    (r"/(config|system|health|diskspace|update|log|backup|filesystem|localization|ping|autotagging)", "R5 system"),
]


def rd_track(path):
    p = path.lower()
    for pat, t in RD_TRACK:
        if re.search(pat, p):
            return t
    return "R5 system"


oa = json.loads(rd(os.path.join(rd_dir, "src/Readarr.Api.V1/openapi.json")))
for path, ops in oa["paths"].items():
    for meth, op in ops.items():
        tag = (op.get("tags") or [""])[0]
        add("Readarr", "endpoint", f"`{meth.upper()} {path}` ({tag})", "src/Readarr.Api.V1/openapi.json", rd_track(path))
cfg = os.path.join(rd_dir, "src/Readarr.Api.V1/Config")
for f in sorted(os.listdir(cfg)):
    if f.endswith("ConfigResource.cs"):
        txt = rd(os.path.join(cfg, f))
        cls = f[: -len("ConfigResource.cs")]
        body = txt.split("public static class")[0]
        for i, line in enumerate(body.splitlines(), 1):
            m = re.match(r"\s*public [A-Za-z<>?\[\]]+ ([A-Za-z]+) \{ get; set; \}", line)
            if m:
                add("Readarr", "setting", f"{cls} `{m.group(1)}`", f"src/Readarr.Api.V1/Config/{f}#L{i}", "R5 system")
tm = rd(os.path.join(rd_dir, "src/NzbDrone.Core/Jobs/TaskManager.cs"))
for m in re.finditer(r"Interval = ([^,]+),\s*TypeName = typeof\(([A-Za-z]+)\)", tm):
    add("Readarr", "job", f"scheduled `{m.group(2)}` (every {m.group(1).strip()})", "src/NzbDrone.Core/Jobs/TaskManager.cs", "R5 system")
for root, _, files in os.walk(os.path.join(rd_dir, "src/NzbDrone.Core")):
    for f in sorted(files):
        if f.endswith("Command.cs") and f != "Command.cs":
            add("Readarr", "action", f"command `{f[:-3]}`", os.path.relpath(os.path.join(root, f), rd_dir), "R2 wanted and downloads" if any(k in f for k in ("Search", "Download", "Rss", "Queue")) else "R5 system")
for kind, sub, tr in (("indexer", "Indexers", "R4 providers"), ("download client", "Download/Clients", "R4 providers"),
                      ("import list", "ImportLists", "R4 providers"), ("notification", "Notifications", "R4 providers"),
                      ("metadata consumer", "Extras/Metadata/Consumers", "R4 providers")):
    d = os.path.join(rd_dir, "src/NzbDrone.Core", sub)
    if os.path.isdir(d):
        for e in sorted(os.listdir(d)):
            if os.path.isdir(os.path.join(d, e)) and not e.startswith("."):
                add("Readarr", "provider", f"{kind} `{e}`", f"src/NzbDrone.Core/{sub}/{e}", tr)
routes = rd(os.path.join(rd_dir, "frontend/src/App/AppRoutes.js"))
for p in dict.fromkeys(re.findall(r'path="([^"*]+)"', routes)):
    add("Readarr", "page", f"`{p}`", "frontend/src/App/AppRoutes.js", "F own pages")

# ---------- output ----------
counter = {}
prefix = {"ABS": "A", "ABS web": "W", "ABS app": "M", "Readarr": "R"}
out = []
for origin, kind, item, src, track in rows:
    pre = prefix[origin]
    counter[pre] = counter.get(pre, 0) + 1
    out.append((origin, f"{pre}{counter[pre]:04d}", kind, item.replace("|", "\\|"), src, track))
summary = {}
for o, _, k, *_ in out:
    summary.setdefault(o, {}).setdefault(k, 0)
    summary[o][k] += 1
json.dump(summary, open(os.environ.get("SUMMARY", "/dev/null"), "w"))
cur = None
for origin, rid, kind, item, src, track in out:
    if origin != cur:
        cur = origin
        print(f"\n### {origin}\n")
        print("| ID | Kind | Item | Upstream source | Track | Status | Proof |")
        print("| --- | --- | --- | --- | --- | --- | --- |")
    na = origin == "Readarr" and (
        (kind == "provider" and ("indexer" in item or "download client" in item))
        or (kind == "endpoint" and re.search(r"/api/v1/(indexer|downloadclient)", item))
        or (kind == "setting" and (item.startswith("Indexer ") or item.startswith("DownloadClient ")))
    )
    status, proof = ("n/a", "Lantern replaces it (GOALS item 2)") if na else ("todo", "")
    print(f"| {rid} | {kind} | {item} | `{src}` | {track} | {status} | {proof} |")
