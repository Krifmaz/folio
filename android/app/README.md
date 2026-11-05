# app

Folio's Android app (`dev.krifmaz.folio`; debug builds install beside it as `dev.krifmaz.folio.debug`).
One activity in the kit's default theme; every page is a kit-compose part, wired to this repository's server.

| File | What it is |
| --- | --- |
| `App.kt` | The process: crash log, `KitApp.init`, where updates come from, the server and its calls |
| `MainActivity.kt` | The theme, the pages and the toasts; the daily update check |
| `AppShell.kt` | The pages as a stack (Back goes down), and the bug report screens' host |
| `ConnectPage.kt` | First start: the server's address |
| `SignInPage.kt` | Username and password (and the authenticator code), the kit's phone sign-in |
| `HomePage.kt` | Offline banner, What's new, the entries (Library, settings), bug reports, crash log, updates |
| `LibraryPage.kt` | Every book on the server, with search and All / Audiobooks / Ebooks; the shared `Cover` and error words |
| `WorkPage.kt` | One book: cover, authors, series, blurb and every exact edition with its facts and buttons |
| `Library.kt` | The library's models and `LibraryRules`: reading `/api/library/…` and the words shown (unit-tested in `src/test`) |
| `AdminPages.kt` | Server settings (forms from the server's schema) and Diagnostics (with the app's crash reports) |
| `Server.kt` | The address and token, the server's API (`KitApi`), What's new calls, the network check |
| `ServerAddress.kt` | What is tried for a typed address (unit-tested in `src/test`) |
| `SignInRules.kt` | The sign-in's body, its answer and words (unit-tested in `src/test`) |
| `EpubBook.kt` | An EPUB's reading order, table of contents and places (OPF, nav, NCX; unit-tested) |
| `ReaderView.kt` | The WebView that shows one part of the EPUB in pages, served from the file |
| `ReaderAssets.kt` | The reader's script and style, how they go into each page, the content security policy (unit-tested) |
| `ReaderPage.kt` | The reader: pages, controls, text size, contents, bookmarks; `ReaderPrefs` |
| `BookFiles.kt` | Fetching a file to the phone (`.part`, then renamed); an edition and its files, offline too |
| `Downloads.kt` | Books kept on the phone: start, stop, go on, remove, open offline |
| `DownloadRules.kt` | Which files a download keeps, and the button's words (unit-tested) |
| `DownloadsPage.kt` | The Download button and the Downloads page |
| `Playback.kt` | Files, progress, locators, bookmarks and `PlaybackRules`: places across files, chapters, speeds, the sleep timer (unit-tested) |
| `Sync.kt` | The place in each book and the bookmarks, kept on the phone and sent through the kit's outbox |
| `PlayerService.kt` | Media3's player in a media session; saves the place, runs the sleep timer; `NowPlaying` (the loaded book) |
| `PlayerConnection.kt` | The app's Media3 controller: loading a book, jumping across files; `PlayerPrefs` (speed) |
| `PlayerPage.kt` | The player: chapter slider, transport, speed, sleep timer, bookmarks, chapters |

| kit-compose part | Where | Server |
| --- | --- | --- |
| Theme, toasts, details | `MainActivity`, `HomePage` | — |
| What's new popup and changelog | `HomePage` (`WhatsNewOnStart`), `AppShell` (`ChangelogScreen`) | `/api/changelog…`, `PUT /api/auth/prefs` |
| In-app updates | `App` (`UpdateRules.source`), `MainActivity`, `HomePage` (`UpdatesSection`) | GitHub releases of Krifmaz/folio |
| Bug reports and crash logs | `App` (`CrashLog`), `AppShell`, `HomePage` | `/api/bug-reports…`, `/api/crash-reports` |
| Offline banner | `HomePage` (`probeHealth`, `OfflineBanner`) | `/api/health` |
| Settings forms | `AdminPages` (`SchemaPage`) | `/api/settings…`, `/api/diagnostics/…/test` |
| Diagnostics | `AdminPages` (`DiagnosticsScreen`, `AppCrashesSection`) | `/api/diagnostics` |

Signing in is the kit's phone sign-in: `POST /api/auth/login` with `{username, password, app: {device_id,
device_name}}` answers `{user, token}`; the token goes with every request as `Authorization: Bearer <token>` (only
to the server's own address), with the User-Agent `<app name>-Android/<version>`. A 401 from any later call goes
back to the sign-in. Admin pages show unless `GET /api/auth/me` says the account isn't one.

The library comes from `GET /api/library/works?q=&format=` and `GET /api/library/works/{id}` (see
[docs/TRACKS.md](../../docs/TRACKS.md) for the whole API the app uses).

Playing and progress use the playback API in docs/TRACKS.md: `GET /api/library/editions/{id}/files`, `GET
/api/files/{id}` (streamed with Range), `GET`/`PUT /api/progress/{edition_id}` (device `android:<device id>`) and the
bookmark routes.

The reader fetches the edition's EPUB (`kind` `epub`) once into the app's cache, or opens the downloaded copy, and
saves its place as a locator `{href, progression, total_progression}` where `href` is the spine item's path from
the EPUB's root.
