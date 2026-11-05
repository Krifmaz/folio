# Changelog

## Unreleased

### Added
- Wanted lists: want an exact edition; Wanted page per format with each book's status. Preferences per format
  (file formats best first: M4B before MP3, EPUB before AZW3 before PDF; languages; unabridged only) refuse
  editions that won't do, in plain words. Follow authors and series in audiobooks, ebooks or both: new books
  (Audible's catalogue, Open Library) are added to the wanted list every 12 hours or with Check followed now.
- Exact editions: every book is a work with its editions. Add an audiobook by its Audible ASIN (Audnexus: narrators,
  runtime, abridged or not, language, series and place, chapters) or an ebook by its ISBN (Open Library, Google
  Books and Hardcover, merged). An audiobook and an ebook of one book share its work. Library, book, authors and
  series pages; Settings → Metadata for the Audible store, the Hardcover token and the Google Books key.
- Folio made from the Krifmaz kit template (v0.2.2).
- GPL-3.0 license and credits for the projects Folio learns from; the goal in `docs/GOALS.md`; `docs/DECISIONS.md`.
- CI and releases through the kit's shared workflows (server, web app, Docker image, Android APK); merged
  branches are deleted by `cleanup-branches`.
