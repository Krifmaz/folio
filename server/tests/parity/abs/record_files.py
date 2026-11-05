#!/usr/bin/env python3
"""Record what the original's scanner read and wrote besides its database, for the scanner's parity test.

Usage: record_files.py CONTAINER MEDIA_DIR METADATA_DIR OUT_DIR

Writes OUT_DIR/fixtures/_ffprobe.json: ffprobe's answer, from inside the original's container and with the
original's own arguments (`nodeFfprobe`), for every audio file of the test media, keyed by its path under /media.
Folio's test feeds these answers to its scanner, because ffprobe's numbers differ between ffmpeg versions.

Writes OUT_DIR/fixtures/_metadata_files.json: every `items/<id>/metadata.json` the scan wrote, keyed by item id.

Only the standard library is used; programs run with an argument list, never through a shell.
"""
import json
import os
import subprocess
import sys

container, media, metadata, out = sys.argv[1:5]
AUDIO = {"m4b", "mp3", "m4a", "flac", "opus", "ogg", "oga", "mp4", "aac", "wma", "aiff", "aif", "wav", "webm", "webma",
         "mka", "awb", "caf", "mpg", "mpeg"}
ARGS = ["-hide_banner", "-loglevel", "fatal", "-show_error", "-show_format", "-show_streams", "-show_programs",
        "-show_chapters", "-show_private_data", "-print_format", "json"]

env = subprocess.run(["docker", "exec", container, "printenv", "FFPROBE_PATH"], capture_output=True, text=True)
ffprobe = env.stdout.strip() or "ffprobe"
answers = {}
for root, _, names in os.walk(media):
    for name in sorted(names):
        if name.rsplit(".", 1)[-1].lower() not in AUDIO:
            continue
        rel = os.path.relpath(os.path.join(root, name), media)
        path = "/media/" + rel
        run = subprocess.run(["docker", "exec", container, ffprobe, *ARGS, path], capture_output=True, text=True, check=True)
        answers[path] = json.loads(run.stdout)
        print("probed", path)

files = {}
items = os.path.join(metadata, "items")
for item in sorted(os.listdir(items)) if os.path.isdir(items) else []:
    p = os.path.join(items, item, "metadata.json")
    if os.path.isfile(p):
        with open(p, encoding="utf-8") as f:
            files[item] = json.load(f)

os.makedirs(os.path.join(out, "fixtures"), exist_ok=True)
for name, data in (("_ffprobe", answers), ("_metadata_files", files)):
    with open(os.path.join(out, "fixtures", name + ".json"), "w", encoding="utf-8") as f:
        json.dump(data, f, indent=1, sort_keys=True, ensure_ascii=False)
        f.write("\n")
print(f"{len(answers)} ffprobe answers and {len(files)} metadata files written")
