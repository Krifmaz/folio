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
