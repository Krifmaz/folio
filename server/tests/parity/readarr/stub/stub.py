#!/usr/bin/env python3
"""A stand-in for Readarr's metadata server and Goodreads' search, for recording Readarr's lookups.

Usage: stub.py ROUTES METADATA_PORT GOODREADS_PORT CERT KEY LOG

Readarr's own metadata server is gone and the live services change, so the recordings use fixed answers:
ROUTES (routes.json) lists them, each with `host` (`metadata`, served over HTTP on METADATA_PORT, or `goodreads`,
served over HTTPS on GOODREADS_PORT with CERT and KEY), `method`, `path`, the `query` it must have, the JSON
`request` body a POST must send, and the answer: `status`, `location` (a redirect) and `body`. Folio's parity
test serves the same routes. Every request is written to LOG as one JSON line, with whether a route answered it.

Only the standard library is used, so it runs on any CI runner.
"""
import http.server
import json
import ssl
import sys
import threading
import urllib.parse

routes_path, metadata_port, goodreads_port, cert, key, log_path = sys.argv[1:7]
with open(routes_path, encoding="utf-8") as f:
    ROUTES = json.load(f)
log_lock = threading.Lock()


def find(host, method, path, query, body):
    """The route answering a request, if any."""
    for r in ROUTES:
        if r["host"] != host or r["method"] != method or r["path"] != path:
            continue
        if r.get("query", {}) != query:
            continue
        if "request" in r and r["request"] != body:
            continue
        return r
    return None


def handler(host):
    """A request handler for one of the two stand-ins."""

    class Handler(http.server.BaseHTTPRequestHandler):
        def answer(self, method):
            url = urllib.parse.urlsplit(self.path)
            query = {k: v[0] for k, v in urllib.parse.parse_qs(url.query, keep_blank_values=True).items()}
            raw = self.rfile.read(int(self.headers.get("Content-Length") or 0))
            try:
                body = json.loads(raw) if raw else None
            except ValueError:
                body = raw.decode("utf-8", "replace")
            r = find(host, method, url.path, query, body)
            with log_lock, open(log_path, "a", encoding="utf-8") as f:
                f.write(json.dumps({"host": host, "method": method, "path": url.path, "query": query, "body": body,
                                    "answered": r is not None}) + "\n")
            if r is None:
                self.send_response(404)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b'{"error":"no stub route"}')
                return
            payload = r["body"] if isinstance(r["body"], str) else json.dumps(r["body"])
            self.send_response(r["status"])
            if "location" in r:
                self.send_header("Location", r["location"])
            self.send_header("Content-Type", "application/json" if not isinstance(r["body"], str) else "text/plain")
            self.send_header("Content-Length", str(len(payload.encode())))
            self.end_headers()
            self.wfile.write(payload.encode())

        def do_GET(self):
            self.answer("GET")

        def do_POST(self):
            self.answer("POST")

        def log_message(self, *args):
            pass

    return Handler


metadata = http.server.ThreadingHTTPServer(("0.0.0.0", int(metadata_port)), handler("metadata"))
goodreads = http.server.ThreadingHTTPServer(("0.0.0.0", int(goodreads_port)), handler("goodreads"))
tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
tls.load_cert_chain(cert, key)
goodreads.socket = tls.wrap_socket(goodreads.socket, server_side=True)
threading.Thread(target=goodreads.serve_forever, daemon=True).start()
print("stub serving", flush=True)
metadata.serve_forever()
