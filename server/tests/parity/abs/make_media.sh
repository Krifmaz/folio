#!/usr/bin/env bash
# Make the small, fixed test library the parity recordings run on: two audiobooks (one M4B with chapters and
# tags, one in three MP3 parts) and one EPUB, laid out as Audiobookshelf expects (Author/Series/Title/).
# Usage: make_media.sh DIR
set -euo pipefail
out="$1"
mkdir -p "$out"
tone() { # tone SECONDS FILE [ffmpeg metadata args...]
    local secs="$1" file="$2"
    shift 2
    ffmpeg -nostdin -loglevel error -y -f lavfi -i "sine=frequency=440:duration=$secs" "$@" -ac 1 "$file"
}

# 1. One M4B with three chapters, tags and a narrator.
d="$out/Ann Leckie/Imperial Radch/1 - Ancillary Justice"
mkdir -p "$d"
meta="$(mktemp)"
cat >"$meta" <<'META'
;FFMETADATA1
title=Ancillary Justice
artist=Ann Leckie
album=Ancillary Justice
composer=Adjoa Andoh
genre=Science Fiction
date=2013
[CHAPTER]
TIMEBASE=1/1000
START=0
END=20000
title=Chapter 1
[CHAPTER]
TIMEBASE=1/1000
START=20000
END=40000
title=Chapter 2
[CHAPTER]
TIMEBASE=1/1000
START=40000
END=60000
title=Chapter 3
META
tone 60 "$d/tmp.m4a" -c:a aac -b:a 32k
ffmpeg -nostdin -loglevel error -y -i "$d/tmp.m4a" -i "$meta" -map_metadata 1 -map_chapters 1 -c copy "$d/Ancillary Justice.m4b"
rm -f "$d/tmp.m4a" "$meta"

# 2. Three MP3 parts with track numbers, no series.
d="$out/Becky Chambers/A Psalm for the Wild-Built"
mkdir -p "$d"
for i in 1 2 3; do
    tone 15 "$d/Part 0$i.mp3" -c:a libmp3lame -b:a 32k -metadata "title=Part $i" -metadata "track=$i/3" \
        -metadata "artist=Becky Chambers" -metadata "album=A Psalm for the Wild-Built"
done

# 3. One EPUB.
d="$out/Martha Wells/The Murderbot Diaries/1 - All Systems Red"
mkdir -p "$d"
python3 -I - "$d/All Systems Red.epub" <<'PY'
import sys, zipfile
p = sys.argv[1]
opf = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:isbn:9780765397522</dc:identifier><dc:title>All Systems Red</dc:title>
<dc:creator>Martha Wells</dc:creator><dc:language>en</dc:language><dc:date>2017-05-02</dc:date>
<meta property="dcterms:modified">2017-05-02T00:00:00Z</meta></metadata>
<manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>
<spine><itemref idref="c1"/></spine></package>"""
with zipfile.ZipFile(p, "w") as z:
    z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip")
    z.writestr("META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>', zipfile.ZIP_DEFLATED)
    z.writestr("content.opf", opf, zipfile.ZIP_DEFLATED)
    z.writestr("nav.xhtml", '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="c1.xhtml">One</a></li></ol></nav></body></html>', zipfile.ZIP_DEFLATED)
    z.writestr("c1.xhtml", '<html xmlns="http://www.w3.org/1999/xhtml"><body>' + "<p>" + "word " * 6000 + "</p></body></html>", zipfile.ZIP_DEFLATED)
PY
echo "Test media in $out"
