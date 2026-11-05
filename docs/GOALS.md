# Goals

The owner's goal for Folio, saved word for word (2026-11-05).

## Folio

Build Folio, one Rust app for audiobooks and ebooks that replaces Readarr/LazyLibrarian, Listenarr, Audiobookshelf and Calibre-Web, and gets them right every time (Docker, Encore look), started from the Krifmaz/kit template. Use the originals as reference only; write everything fresh in Rust (ffmpeg may be bundled for audio work). License GPL-3.0 and credit the original projects.

1. Exact editions: every book is tracked as works → editions. Audiobooks by Audible ASIN (narrator, runtime, abridged/unabridged, language, series position) via Audnexus; ebooks by ISBN via Open Library, Google Books and Hardcover. Follow authors and series; wanted lists per format with preferences (M4B > MP3; EPUB > AZW3 > PDF; unabridged only; language).
2. Downloads only through Lantern (URL + API key; no clients or indexers of its own). Search queries built from the edition (author, title, narrator, ASIN/ISBN), results scored on all of them.
3. Verify before import, never guess: audiobooks must match runtime within 3%, narrator, language and chapter count, and reject samples/abridged; ebooks must match their own OPF metadata (ISBN, title, author, language), be valid DRM-free EPUB, and have a plausible length. A confidence score decides: high = import, otherwise a Review queue with a side-by-side of what was requested vs what arrived, one-click accept/reject/re-search, and the reason in plain words with Copy.
4. Clean up on import: merge multi-file MP3 into one chaptered M4B with cover, chapters (Audnexus) and tags; rewrite EPUB metadata and cover; layout Author/Series/NN - Title (Year)/ with hardlinks from Lantern's download folder.
5. Listening and reading: audiobook player and EPUB reader in the web app and an Android app on kit-compose, with progress, bookmarks, sleep timer, speed, offline downloads, and progress synced across devices. Audiobookshelf-compatible API so existing ABS apps work; OPDS feed; KOReader sync server; Kobo sync.
6. Users and sharing: accounts from Marquee-style invites, per-user libraries, progress and wishlists; requests from users go through approval.
7. Migration: import an existing Audiobookshelf, Calibre library, Readarr/LazyLibrarian/Listenarr wanted lists and progress, with a dry run.

Build on Krifmaz/kit (pin the latest kit tag; read its docs/DESIGN.md and CLAUDE.md first). Use the kit for the look (Encore theme), What's new and changelog, self-updating web app, notice banners, smart toasts with Copy, confirm dialogs, bug reports and crash logs, diagnostics, settings from a schema, offline mode and the shared release workflows. Don't rebuild what the kit has; if something is missing or generic, add it to the kit instead of the app.
Work hands-off: make your own calls (log non-obvious ones in docs/DECISIONS.md), one PR per change, fix CI until green, squash-merge yourself, delete the branch. Every merge ships a normal vX.Y.Z release (no pre-releases). Never skip tests or force-push main. Only stop to ask about deleting user data, a needed secret or paid service. One short summary per finished goal.

## Build order (2026-11-05)

One PR each: works/editions metadata, wanted lists, Lantern search and grab, verification and Review queue, import
cleanup, player and reader, Android app, ABS/OPDS/KOReader/Kobo compatibility, migration. v0.1.0 ships when a
requested audiobook and ebook arrive verified and correctly filed end to end.

## Change of approach (2026-11-05): port, don't reimagine

Saved word for word. It replaces "Use the originals as reference only; write everything fresh in Rust" above.

Change of approach: port, don't reimagine. The app is missing basic features of the originals.
- Fork the originals as the starting point (Audiobookshelf, Readarr; record upstream versions in UPSTREAM.lock like Photovault). Port their backend logic module by module into Rust, keeping behaviour, settings and edge cases; GPL-3.0 with full credits.
- Web UI: use Audiobookshelf's own web frontend (and its mobile app), forked and rethemed to the Encore look (keep their layouts, mobile behaviour and every page), running against Folio's compatible APIs. Strip branding/links we don't want. Our own pages only where the merge needs them (wanted lists and Lantern downloads, verification and Review queue, ebook reader).
- Write docs/PARITY.md: every API endpoint, setting, page, action and background job of each original, with a status column; each row is done only with a test or a screenshot proving it works like the original. Work through it in parallel tracks.
- No new features until parity is complete; then layer our improvements on top.

v0.1.0 still ships when a requested audiobook and ebook arrive verified and correctly filed end to end.
