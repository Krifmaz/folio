# Changelog

## Unreleased

### Added
- Exact editions: every book is a work with its editions. Add an audiobook by its Audible ASIN (Audnexus: narrators,
  runtime, abridged or not, language, series and place, chapters) or an ebook by its ISBN (Open Library, Google
  Books and Hardcover, merged). An audiobook and an ebook of one book share its work. Library, book, authors and
  series pages; Settings → Metadata for the Audible store, the Hardcover token and the Google Books key.
- Folio made from the Krifmaz kit template (v0.2.2).
- GPL-3.0 license and credits for the projects Folio learns from; the goal in `docs/GOALS.md`; `docs/DECISIONS.md`.
- CI and releases through the kit's shared workflows (server, web app, Docker image, Android APK); merged
  branches are deleted by `cleanup-branches`.
