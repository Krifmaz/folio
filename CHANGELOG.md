# Changelog

## Unreleased

### Added
- Verification before import: every download is checked against the edition you asked for. Audiobooks: runtime
  within 3%, narrator, language, chapter count, and nothing marking a sample or an abridged copy (read with
  ffprobe, which the image now carries). Ebooks: a valid EPUB without DRM whose own ISBN, title, author and
  language match, with a length that fits. A confidence score decides: sure enough (85% by default, Settings →
  Downloads) is checked and ready to file; anything else waits under **Review**, with what was requested next to
  what arrived, the reason with Copy, and Accept, Reject or Reject and search again. A rejected download is
  never grabbed again for that book.
- `docs/TRACKS.md`: the parallel tracks, the files each one owns, and the schema and playback API they share. The
  library keeps each book's files (`edition_files`), and the library folder is a setting (Settings → Downloads).
- `docs/GOALS.md`: the goal, word for word.
- Downloads through Lantern only (Settings → Downloads: its address and API key). Searches are built from the
  edition (ASIN or ISBN, author and title, title and narrator) and every result is scored on all of them, the
  file format, language, size and seeders, with the reasons; samples, abridged copies, other languages and file
  formats the preferences don't take are refused. The best result is grabbed (Search now, or every 6 hours);
  Results shows every result with Grab; Downloads follows each grab until its files arrive. Diagnostics checks
  Lantern. The contract is in `docs/LANTERN.md`.
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
