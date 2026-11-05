# Folio

One self-hosted app for audiobooks and ebooks: exact editions, wanted lists, downloads through
[Lantern](https://github.com/Krifmaz/lantern), verification before import, a tidy library, and listening and
reading on the web and Android. Goals: [docs/GOALS.md](docs/GOALS.md). Decisions: [docs/DECISIONS.md](docs/DECISIONS.md).

Made from the [Krifmaz kit](https://github.com/Krifmaz/kit) template (v0.2.2): the kit's look and its
ease-of-use features from the first commit.

| Folder | What it is |
| --- | --- |
| `server/` | Axum server with the kit server crates: health, changelog (What's new), notices, bug reports, diagnostics, settings |
| `web/` | Leptos web app with the kit components and the kit's default theme, served at `/` by the server |
| `android/` | Compose app with kit-compose, a Baseline Profile and a benchmark module |
| `docs/releases/` | one Markdown file per release: the release notes and What's new |

## First steps

1. Make the lock files and commit them (CI and the Docker build use `--locked`):
   `(cd server && cargo generate-lockfile) && (cd web && cargo generate-lockfile)`.
2. Push to GitHub. Every pull request runs the checks; every merge to main is a release.
3. The first start makes the owner account: set `APP_ADMIN_PASSWORD` (and `APP_ADMIN_USER`, default `admin`),
   or read the password it prints (also in `initial_admin_password.txt` in the data folder).

## Run it

```
cd web && trunk build && cd ..
cd server && APP_DATA_DIR=../data APP_STATIC_DIR=../web/dist cargo run
```

Then open <http://localhost:8080>. With Docker:

```
docker build -t folio .
docker run --read-only -p 8080:8080 -v folio-data:/data folio
```

| Variable | Meaning | Default |
| --- | --- | --- |
| `APP_PORT` | the port the server listens on | `8080` |
| `APP_DATA_DIR` | where the database and uploads live (the only folder it writes to) | `/data` |
| `APP_STATIC_DIR` | the built web app | `/app/static` |
| `APP_ADMIN_USER`, `APP_ADMIN_PASSWORD` | the owner made at the first start | `admin`, a printed password |
| `APP_NAME`, `APP_SESSION_DAYS`, `APP_API_KEY` | settings (settings.toml); set here they are locked in Settings | |

## Checks

In `server/` and `web/`: `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
Every merge to main is released as `vX.Y.Z` with the Docker image `ghcr.io/krifmaz/folio:<version>` and `:latest`.
The signed Android APK builds with Actions → release → "apps" (secrets `ANDROID_KEYSTORE_BASE64`,
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`).

## License and credits

Folio is free software under the [GNU General Public License v3.0](LICENSE). It is a port of Audiobookshelf and
Readarr: their backends are being ported to Rust module by module, and Audiobookshelf's web and mobile apps are
forked in `abs-web/` and `abs-app/` (see [UPSTREAM.md](UPSTREAM.md) for the exact versions). Their copyright
notices stay with their code. Thank you to their authors:

- [Audiobookshelf](https://github.com/advplyr/audiobookshelf) and its
  [mobile app](https://github.com/advplyr/audiobookshelf-app) (GPL-3.0), by advplyr and contributors: the
  listening server, web app and mobile app that Folio ports and forks.
- [Calibre-Web](https://github.com/janeczku/calibre-web) (GPL-3.0) and [Calibre](https://calibre-ebook.com):
  reading in the browser, OPDS and Kobo sync.
- [Readarr](https://github.com/Readarr/Readarr) (GPL-3.0), [LazyLibrarian](https://gitlab.com/LazyLibrarian/LazyLibrarian)
  (GPL-3.0) and [Listenarr](https://github.com/therobbiedavis/Listenarr): following authors, wanted lists and
  importing.
- [Audnexus](https://github.com/laxamentumtech/audnexus) for audiobook metadata, [Open Library](https://openlibrary.org),
  Google Books and [Hardcover](https://hardcover.app) for book metadata, and [KOReader](https://koreader.rocks) for its
  sync protocol.
- [FFmpeg](https://ffmpeg.org) (LGPL/GPL) for audio work.
- The [Krifmaz kit](https://github.com/Krifmaz/kit) (MIT) for the look and the shared features.
