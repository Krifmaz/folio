"""Writes the test library's EPUB ("All Systems Red" by Martha Wells) as a whole book for the phone app's reader.

The parity test library (server/tests/parity/abs/make_media.sh) makes its EPUB as small as it can, with pages
that have no <head>. That is enough for the server, but the app's reader (epub.js) cannot count locations in such
pages and throws an error, which a real book never causes. This writes the same book with whole pages over it.

Usage: python3 -I make_epub.py PATH
"""

import sys
import zipfile

OPF = """<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="id">urn:isbn:9780765397522</dc:identifier><dc:title>All Systems Red</dc:title>
<dc:creator>Martha Wells</dc:creator><dc:language>en</dc:language><dc:date>2017-05-02</dc:date>
<meta property="dcterms:modified">2017-05-02T00:00:00Z</meta></metadata>
<manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>
<spine><itemref idref="c1"/></spine></package>"""

CONTAINER = """<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
<rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"""


def page(title: str, body: str) -> str:
    """One whole XHTML page: the declaration, a head with its title, and [body]."""
    return (
        '<?xml version="1.0" encoding="utf-8"?><!DOCTYPE html>'
        '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">'
        f"<head><title>{title}</title></head><body>{body}</body></html>"
    )


def main() -> None:
    """Writes the book to the path given on the command line."""
    with zipfile.ZipFile(sys.argv[1], "w") as z:
        # the EPUB rule: "mimetype" first and stored as it is
        z.writestr(zipfile.ZipInfo("mimetype"), "application/epub+zip")
        z.writestr("META-INF/container.xml", CONTAINER, zipfile.ZIP_DEFLATED)
        z.writestr("content.opf", OPF, zipfile.ZIP_DEFLATED)
        nav = '<nav epub:type="toc"><ol><li><a href="c1.xhtml">One</a></li></ol></nav>'
        z.writestr("nav.xhtml", page("Contents", nav), zipfile.ZIP_DEFLATED)
        z.writestr("c1.xhtml", page("One", "<p>" + "word " * 6000 + "</p>"), zipfile.ZIP_DEFLATED)


if __name__ == "__main__":
    main()
