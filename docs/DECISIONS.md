# Decisions

Non-obvious calls, one line each, newest last.

- 2026-11-05 · Made from the kit template at v0.2.2 (the newest kit tag) with the Android app; the template's desktop app is left out, since the goal asks for the web and Android only.
- 2026-11-05 · Releases start at 0.0.1 (`first-version` in release.yml) so every merge ships a normal release before the goal's v0.1.0; the PR that gets a requested audiobook and ebook verified and filed end to end adds `docs/releases/0.1.0.md`, which makes that merge v0.1.0.
- 2026-11-05 · License GPL-3.0-only for Folio's own code (the kit stays MIT, which GPL allows); the original projects are credited in the README.
- 2026-11-05 · The first commit (goals and README) went to main directly, since a pull request needs a branch to merge into and the repository was empty.
- 2026-11-05 · Merged branches are deleted by the kit's cleanup-branches workflow (copied in), as the kit does, because this environment's proxy refuses ref deletes.
- 2026-11-05 · Works and editions: editions of one work are joined by Open Library's work key, else by the title before its subtitle (no leading article) and the first author's letters (`text::title_key`, `text::person_key`); Audnexus has no work key, so audiobooks join by title and author.
- 2026-11-05 · Each edition keeps the sources' whole answer as JSON (`editions.meta`) with the searched columns beside it, so new fields need no table change.
- 2026-11-05 · Ebook sources are merged in the order Open Library, Hardcover, Google Books (first non-empty field wins); the blurb takes the longest. Hardcover is asked only with a token; one failing source doesn't stop the others.
- 2026-11-05 · The metadata tests serve hand-written fixtures in each source's shape from a local server: this build environment can't reach Audnexus, Open Library or Hardcover.
- 2026-11-05 · Adding and removing editions is for admins until users and requests (goal 6) come; reading is for anyone signed in.
- 2026-11-05 · Wanting is per exact edition, under one preferences profile per format (made at the start, edited on the Wanted page); searching, verifying and importing read the same profile.
- 2026-11-05 · Followed authors' and series' audiobooks come from Audible's public catalogue (`api.audible.<store>/1.0/catalog`), since Audnexus has no author or series book lists; each ASIN is then read from Audnexus. Ebooks come from Open Library's author works and work editions (Open Library's own ebook editions first).
- 2026-11-05 · Every offer from a followed author or series is looked at once (`follow_seen`), so a book taken off the wanted list is not added back; an offer whose source couldn't be reached is asked again next time.
- 2026-11-05 · A language the source doesn't state is let through when wanting; the arriving files are checked for it.
- 2026-11-05 · Lantern search goes through its Prowlarr API (`GET /api/v1/search`, there today); grabs, progress and callbacks go through its download hub (`/api/hub/*`, Lantern's goal 9, not built yet), as written down in `docs/LANTERN.md`. Folio asks about grabs every minute, so callbacks are a speed-up, not a need.
- 2026-11-05 · A result is grabbed automatically only at 70 points or more (a setting) and never when refused; a person may grab a refused result, after a danger confirm, and it is still verified on arrival.
- 2026-11-05 · Lantern's callback address carries a random secret (kept in `folio_kv`) instead of an API key, since Lantern only knows Folio's address.
- 2026-11-05 · Folio is built in parallel tracks (core pipeline, player and reader, Android, compatibility, migration); `docs/TRACKS.md` pins who owns which files and the schema and playback API they share. Lengths are milliseconds everywhere (Audnexus gives chapters in ms), and progress is last-writer-wins by `updated_at`.
- 2026-11-05 · The library folder (`library_dir`, default `/library`) is the one place outside the data folder Folio writes to: books are filed there with hardlinks from Lantern's download folder, so both must be on the same disk.
- 2026-11-05 · `docs/GOALS.md` missed the first commit (a failed download stopped the command before it was written); it is added with TRACKS, word for word as saved then.
- 2026-11-05 · Library files are sent with `Content-Encoding: identity`, so the server's gzip layer leaves them alone: audio and EPUB are compressed already, and a gzipped answer would drop Range (jumping in a long book).
- 2026-11-05 · Migration reads each source's database file directly, copied with its `-wal` into a scratch folder first, so the source app can keep running and is never written to (SQLite writes beside a file even when opened read-only); no API keys are needed.
- 2026-11-05 · Migration reads Readarr, LazyLibrarian and Listenarr for their wanted lists only; books already had come from Audiobookshelf or Calibre, where the files are. Calibre-Web's progress points at `calibre:<id>`, the ids the Calibre reader gives.
- 2026-11-05 · A Readarr book is an audiobook when its edition's format says audio, or its author's quality profile allows only audio qualities; otherwise an ebook.
