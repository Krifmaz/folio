# Tracks

Folio is a **port** of Audiobookshelf and Readarr (see [UPSTREAM.md](../UPSTREAM.md)). The work is the list in
[PARITY.md](PARITY.md): every row there names its track. Tracks run in parallel, each on its own branches and PRs,
and own their files: no two tracks edit the same file at the same time. The **core** track owns everything not
listed under another track, merges every PR in dependency order and cuts the releases.

No new features until every row is `done` or `n/a`. Then improvements go on top.

## Who owns what

| Track | PARITY rows | Owns |
| --- | --- | --- |
| **Core** | `P0` data model, `P2 users`, `P6 server admin` | `server/src/abs/{mod.rs,schema.sql,socket.rs,auth,users,me,admin,backups,notifications,logs,cache,tools,settings}*`, `server/src/abs/models/{mod,rows}.rs` and the core models' files (`user`, `setting`, `api_key`, `session`), `scripts/abs-rows.py`, `server/tests/parity/` (fixtures, harness), `server/tests/parity_abs_{users,admin}.rs`, `scripts/`, `UPSTREAM.*`, everything from phase 1 that no track owns |
| **A: web and playback** | `W web fork`, `P3 playback`, the ebook reader (`F own pages`) | `abs-web/`, `server/src/abs/{playback,hls,sessions,progress,bookmarks}*`, `server/tests/parity_abs_playback.rs`, the build step that serves `abs-web` at `/`, `server/src/playback/`, `web/src/{player,reader,offline_media}.rs` |
| **B: mobile** | `M mobile fork` | `abs-app/`, its CI job, and `android/` until the fork replaces it |
| **C: library** | `P1 library`, `P4 collections`, `P5 podcasts and feeds` | `server/src/abs/{library,items,scanner,authors,series,search,collections,playlists,podcasts,feeds,match}*`, `server/tests/parity_abs_{library,collections,podcasts}.rs`, `server/src/compat/` |
| **D: Readarr** | `R1`–`R5`, and the wanted and Lantern pages (`F own pages`) | `server/src/readarr/`, `server/tests/parity_readarr_*.rs`, `server/crates/migrate/`, `server/src/migration/`, `web/src/{wanted,downloads,migrate}.rs` |

The Review queue (verification) stays with core: `server/src/verify/`, `web/src/review.rs`.

### Shared files

- **Models.** `server/src/abs/models/rows.rs` (generated from `schema.sql`, core) has one row type per table and
  `models/mod.rs` the typed find, list, insert, update and delete. A model's behaviour (the JSON the API sends, its
  rules) goes in `server/src/abs/models/<model>.rs`, owned by the track of its domain: C has `library_item`,
  `book`, `podcast`, `podcast_episode`, `author`, `series`, `collection`, `playlist` and `feed`; A has
  `playback_session`, `media_progress` and `device`; core has the rest. Add the file and its `pub mod` line in
  `models/mod.rs`.
- **Socket.** Emit through `st.sockets` (`server/src/abs/socket.rs`): `emitter`, `client_emitter`,
  `admin_emitter`, `library_item_emitter`, `library_items_emitter`, named as in the original.
- **Fixtures.** `server/tests/parity/` holds the harness (`mod.rs`: `fixture`, `assert_like`) and the recordings;
  add requests to `abs/requests.json` (one entry each, any track) and copy the answers you need into
  `fixtures/abs/`.
- **`server/src/abs/mod.rs`** (core) declares every `abs` module and merges their routers. A track adds its own
  `pub mod x;` line and one `.merge(x::router())` line in its PR, and nothing else in that file.
- **`server/src/lib.rs`**: the same, one line per new top-level module (`readarr`).
- **docs/PARITY.md**: change only your own rows (status and proof). Rows are one line each, so parallel PRs merge.
- **Dependencies, settings, CHANGELOG.md, docs/DECISIONS.md, tables**: as in phase 1 below. The `abs` tables are
  core's migrations component `folio-abs`; a track that needs a column asks core.

## Order

1. **This plan**: PARITY.md, UPSTREAM.lock, the upstream script.
2. **Forks imported** (core): `abs-web/` and `abs-app/` at the pinned tags, merged with merge commits. A and B
   start from them.
3. **P0, the data model** (core): Audiobookshelf's database (`server/models/*.js`: libraries, folders, library
   items, books, podcasts, episodes, authors, series, collections, playlists, users, sessions, progress, feeds,
   settings) ported as SQLite tables in `folio-abs`, with the same ids (UUIDs), fields and JSON shapes, plus the
   recorded-fixture test harness in `server/tests/parity/`. C and A build on it; D builds on phase 1's wanted and
   library tables and does not wait.
4. Then all tracks in parallel. Core merges whatever is green, one at a time, oldest first.

## How a row is proven

- **Endpoints and socket events**: a test calls Folio and compares the answer with a fixture recorded from the
  pinned upstream (`ghcr.io/advplyr/audiobookshelf:2.37.2`, or Readarr's last image) for the same request and
  data: status, every field, ordering, errors. `server/tests/parity/record.md` says how to record one.
- **Settings**: a test that the setting exists with the upstream name and default, and changes the behaviour it
  changes upstream.
- **Jobs**: a test that the job runs on its schedule (or event) and does what upstream does.
- **Pages and actions**: a screenshot of Folio next to the original at the same size, in
  `docs/parity/screens/<ID>.png`, plus a Playwright check that the action calls the right endpoint.

## Phase 1 (done): the original pipeline

The tables and APIs below are still in use. The wanted lists, Lantern, verification and import keep working while
the port goes on, and the Readarr track ports onto them.

### The core schema

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

### Track A's tables (`folio-playback`)

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

### The playback API (Track A serves it; the web app, Android and C use it)

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

### Track C's routes

Track C owns these prefixes, so no other track adds routes under them: `/opds`, `/koreader`, `/kobo`, and the
Audiobookshelf paths under `/abs`. If an ABS app has to find the paths at the root (`/login`, `/status`, `/ping`,
`/api/libraries`, `/api/items`, `/api/me`, `/api/session`), Track C adds them there. Core never uses those names.
