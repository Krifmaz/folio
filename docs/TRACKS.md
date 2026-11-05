# Tracks

Folio is built in parallel tracks. Each track has its own branch and PRs and owns its files. No two tracks edit the
same file at the same time. The **core** track (the server pipeline) owns everything not listed under another
track. It also merges every PR, in dependency order, and cuts the releases.

## Who owns what

| Track | Owns | Builds |
| --- | --- | --- |
| **Core** (pipeline, merges) | `server/src/{lib,main,auth,checks}.rs`, `server/src/{library,metadata,wanted,lantern,verify,import}/`, `server/tests/common/`, `server/tests/{api,library,wanted,lantern,verify,import}.rs`, `web/src/{main,library,wanted,downloads,review,pages}.rs`, `web/static/css/app.css`, `web/index.html`, `Dockerfile`, `.github/`, `docs/{GOALS,LANTERN,TRACKS}.md`, `docs/releases/` | metadata, wanted lists, Lantern, verification and the Review queue, import and cleanup, the `edition_files` table |
| **A: player and reader** | `server/src/playback/`, `server/tests/playback*.rs`, `web/src/{player,reader,offline_media}.rs`, `web/static/css/player.css` | streaming, progress, bookmarks, the web player and EPUB reader, offline media in the web app |
| **B: Android** | `android/` | the Android app on kit-compose against the API below |
| **C: compatibility** | `server/src/compat/`, `server/tests/compat_*.rs`, `docs/COMPAT.md` | Audiobookshelf API, OPDS, KOReader sync, Kobo sync |
| **D: migration** | `server/crates/migrate/` (package `folio-migrate`), `server/src/migration/`, `server/tests/migration*.rs`, `web/src/migrate.rs` | reading Audiobookshelf, Calibre (and Calibre-Web), Readarr, LazyLibrarian and Listenarr, a dry run, then applying it |

### Shared files

Each of these is shared. Change them only as described.

- **Wiring** is already in place. `lib.rs` calls `playback::`, `compat::` and `migration::` `migrate(&db)` and
  merges their `router()`. `web/src/main.rs` routes `#/play?id=` to `player::Player` and `#/read?id=` to
  `reader::Reader`. Every edition row on the book page calls `player::edition_actions(&edition)` for its Listen
  and Read buttons. Track D adds `mod migrate;`, the `#/migrate` route and a Settings link to `web/src/main.rs` in
  its own PR. That PR adds only those lines to the file.
- **Dependencies.** Add your crates to `server/Cargo.toml` or `web/Cargo.toml` under a comment naming your track,
  at the end of `[dependencies]`. Commit `Cargo.lock`. Track D adds `members = ["crates/migrate"]` to
  `[workspace]`.
- **Settings.** Add a `[sections.<track>]` block of your own at the end of `server/settings.toml`. Core owns the
  rest.
- **CHANGELOG.md and docs/DECISIONS.md.** Add your own lines under `## Unreleased`, or one line at the end of
  DECISIONS.
- **Tables.** Each track has its own migrations component: `folio-playback`, `folio-compat` or
  `folio-migration`. A track never alters another track's tables. If you need a column there, ask its owner.

When two PRs touch the same shared file, the later one rebases. The core track resolves the conflicts when it
merges.

## Merge order

1. TRACKS (this file and the placeholders).
2. Core: Lantern (#4), verification and Review, then import. Import fills `edition_files`.
3. A's playback store and API. B and C build against the contract below from the start, and merge after A.
4. B, C and D merge in any order once A is in. D's dry-run crate can merge at any time.
5. v0.1.0 ships with the core track once a requested audiobook and ebook arrive verified and filed.

## The core schema

All tables live in one SQLite database. The column types below are SQLite types. Times are Unix seconds
(`REAL`), and lengths are milliseconds.

- **`users`**: `id`, `username`, `role` (`owner`, `admin`, `user`) and `disabled`. A request's account comes from
  `kit_server_core::SignedIn(account)` or `Admin(account)`, whose `account.id` is the `users.id`. Sign-in works
  three ways: a cookie session for the web app, `Authorization: Bearer <app token>` for apps (`POST
  /api/auth/login` with `app: {device_id, device_name}`), and `X-Api-Key` (as the owner). Cookie requests that
  change something send `X-Requested-With: Folio`. **There are no per-user libraries yet.** Every signed-in
  account sees the whole library. Per-user libraries come with goal 6 and will be a filter on `works` or
  `editions`.
- **`works`** has `id`, `title`, `description`, `cover_url` and `year`. Authors and series hang off `work_authors`
  (`position`) and `work_series` (`position` as text, like `"2"` or `"2.5"`).
- **`editions`** has `id`, `work_id`, `format` (`audiobook` or `ebook`), `asin`, `isbn13`, `title`, `language`
  and `meta`. `meta` is `EditionMeta` as JSON: authors, narrators, series, `runtime_min`, `pages`, `chapters`
  ([{title, start_ms, length_ms}] from Audnexus), `cover_url`, `description` and more. Read it with
  `library::store::edition(db, id)`, which returns `(row, EditionMeta)`. `GET /api/library/works/{id}` returns a
  work with its editions.
- **`edition_files`** (`folio-library` step 3) holds the files of a book that is in the library. **The import
  writes it, and every other track only reads it.**

  | column | meaning |
  | --- | --- |
  | `id` | the file's id (`/api/files/{id}`) |
  | `edition_id` | the edition it belongs to |
  | `path` | relative to the library folder (setting `library_dir`, default `/library`), e.g. `Andy Weir/Project Hail Mary (2021)/Project Hail Mary.m4b` |
  | `name` | the file name shown to people |
  | `kind` | `audio`, `epub`, `pdf`, `azw3` or `cover` |
  | `mime` | e.g. `audio/mp4`, `audio/mpeg`, `application/epub+zip`, `image/jpeg` |
  | `size` | bytes |
  | `position` | the play order of an edition's audio files, from 0 |
  | `duration_ms` | the length of an audio file (0 otherwise) |
  | `chapters` | the chapters of an audio file as JSON `[{title, start_ms, end_ms}]`, counted from the start of this file |

  An imported audiobook is usually one chaptered M4B, but multi-file books stay possible. Every audio file is
  ordered by `position`. An edition's position in the book is the sum of the durations of its earlier files,
  plus the position in the current file. Until the import lands, tests insert rows themselves.
- **`wanted`** and **`grabs`** belong to core. Other tracks read the wanted list through `wanted::store`. When
  migration imports wanted entries, it calls `wanted::store::want(db, edition_id, profile_id, user_id,
  "migration")`.

## Track A's tables (`folio-playback`)

Track A creates these tables, and every other track reads and writes them through `playback::store`.

- **`progress`**: one row per account and edition, with primary key `(user_id, edition_id)`.

  | column | meaning |
  | --- | --- |
  | `position_ms` | audio: where in the whole book (across files) |
  | `locator` | ebook: JSON `{href, progression, total_progression, cfi?}`, where `href` is the spine item, `progression` how far through it (0–1), `total_progression` how far through the book (0–1) and `cfi` an EPUB CFI when known |
  | `fraction` | how far through the book (0–1), for both formats |
  | `finished` | 0 or 1 |
  | `device` | which app or device wrote it, e.g. `web`, `android:<device_id>`, `koreader:<name>`, `kobo:<id>`, `abs:<client>` |
  | `updated_at` | when the app says it was read or listened |

  **The last writer wins.** A save whose `updated_at` is older than the stored one changes nothing, and returns
  the stored row.
- **`bookmarks`**: `id`, `user_id`, `edition_id`, `position_ms`, `locator`, `note` and `created_at`.

Track A exports the store functions that Track C uses for KOReader, Kobo and ABS. The names are fixed now:
`playback::store::{progress, all_progress, save_progress, bookmarks, add_bookmark, delete_bookmark}`, with
`(db, user_id, edition_id, …)` arguments and `Progress` and `Bookmark` structs whose fields match the columns.

## The playback API (Track A serves it; the web app, Android and C use it)

Every route needs a signed-in account (cookie or bearer).

```text
GET    /api/library/editions/{id}/files   {files: [{id, name, kind, mime, size, duration_ms, chapters: [{title, start_ms, end_ms}]}]}
GET    /api/files/{id}                    the file's bytes, with Range (206), ETag and the right Content-Type
GET    /api/progress                      {progress: [...]} for this account, every edition (?since=<unix seconds> for only newer)
GET    /api/progress/{edition_id}         {edition_id, position_ms, locator, fraction, finished, device, updated_at}; 404 when never opened
PUT    /api/progress/{edition_id}         the same fields; last writer wins by updated_at; answers the row that stands
GET    /api/editions/{id}/bookmarks       {bookmarks: [{id, position_ms, locator, note, created_at}]}
POST   /api/editions/{id}/bookmarks       {position_ms?, locator?, note}
DELETE /api/bookmarks/{id}
```

Errors are `{"detail": "<plain sentence>"}` with the right status, through `kit_server_core::err`.

## Track C's routes

Track C owns these prefixes, so no other track adds routes under them: `/opds`, `/koreader`, `/kobo`, and the
Audiobookshelf paths under `/abs`. If an ABS app has to find the paths at the root (`/login`, `/status`, `/ping`,
`/api/libraries`, `/api/items`, `/api/me`, `/api/session`), Track C adds them there. Core never uses those names.
