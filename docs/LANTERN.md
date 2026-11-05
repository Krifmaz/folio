# Folio ↔ Lantern

Folio downloads only through [Lantern](https://github.com/Krifmaz/lantern). It keeps no download clients and no
indexers; it needs Lantern's address and API key (Settings → Downloads) and, optionally, its own address so
Lantern can call back. Every call sends `X-Api-Key: <Lantern's API key>`.

## Search (Lantern's Prowlarr API)

```
GET /api/v1/search?query=<words>&type=search&categories=3030&limit=100
GET /api/v1/search?query=<words>&type=book&author=<author>&title=<title>&categories=7020,7000&limit=100
→ [ {guid, title, indexerId, indexer, size, seeders, protocol, downloadUrl, infoUrl, publishDate, …} ]
```

Audiobooks search category 3030 (Audio/Audiobook); ebooks 7020 (Books/EBook) and 7000 (Books). Folio sends, most
exact first: the ASIN or ISBN, author + title (as a book search), title + narrator, title. Every result is scored
by Folio itself (`server/src/lantern/score.rs`).

## Download hub (Lantern's goal 9)

```
POST /api/hub/apps   {name: "Folio", media_types: ["audiobook", "ebook"], callback_url}
→ {id, categories: {audiobook, ebook}, …}          sign-up; sent again before each grab (harmless)

POST /api/hub/grabs  {app: "Folio", media_type, guid, indexer_id, download_url, title, protocol, reference}
→ {id, status, progress}                           reference is Folio's own ("wanted-12"), echoed back

GET  /api/hub/grabs/{id}
→ {id, status, progress, path, files: [{path, size}], error, reference}
```

`status` is `queued`, `downloading`, `completed` or `failed`; `progress` 0 to 1. `path` and `files[].path` are the
finished files' full paths as Folio sees them: Lantern's download folder is mounted into Folio at the same path,
on the same file system as Folio's library, so Folio can hardlink instead of copying.

## Callback

When `callback_url` is set (Folio's `public_url` + `/api/lantern/callback/<secret>`), Lantern `POST`s the grab
object above to it whenever a grab changes. Without it Folio asks `GET /api/hub/grabs/{id}` every minute; it also
does so as a safety net when callbacks are on.

## Errors

Any non-2xx answer with `{"detail": "<sentence>"}` is shown to the person as Lantern's own words. 401/403 means the
key is wrong; 404 without a `detail` means the Lantern version has no download hub yet.
