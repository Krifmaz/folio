# Changelog

## Unreleased

### Added
- Listening and reading on the server: each book's files are sent whole or in parts (so players can jump anywhere)
  with an ETag; each person's place in each book and their bookmarks are kept, and the newest place wins across
  devices (`/api/files`, `/api/progress`, `/api/editions/{id}/bookmarks`).
- Android: the EPUB reader. Read on a book page opens the book a page at a time (tap the sides or swipe, tap the
  middle for the controls) in the app's theme, with the text size, the table of contents, bookmarks and the place
  kept in step with the web app, KOReader and other phones. The book's own scripts and anything outside the book
  are blocked.
- Android: offline downloads. Download on a book page keeps the audiobook or EPUB on the phone (with the percent,
  Stop, and Try again); Downloads lists what is kept and the room it takes, with Remove. Kept books open and play
  without the server, and the places saved meanwhile go once it answers again.
- Android: the audiobook player. Listen on a book page opens it where you left off, on this phone or another
  device (it says so). Chapters across all the book's files, Back 15 s and Forward 30 s, previous and next chapter,
  speeds from 0.75× to 3×, a sleep timer (minutes or the end of the chapter, fading out), bookmarks, and the
  notification, lock screen, headphones and Bluetooth. The place is saved every 15 seconds, on pause and on every
  jump, and waits on the phone while the server can't be reached.
- Android: Library page (search titles and authors, All, Audiobooks or Ebooks) and a book page with every exact
  edition and its facts: narrator, runtime, abridged or not, language, pages, ASIN or ISBN.
- Import: checked downloads are filed in the library folder as `Author/Series/NN - Title (Year)/`. Audiobooks
  become one chaptered M4B with the edition's tags, Audnexus chapters (or one per file) and cover; several MP3s
  are joined into it. EPUBs get the edition's title, authors, language, ISBN, series and cover. With "Clean up
  files" off (Settings → Downloads) the downloads are hardlinked in unchanged. Lantern's copy is never touched,
  so it keeps seeding. A download that can't be filed says why and has File again under Downloads. Diagnostics
  checks the library folder.
- Verification before import: every download is checked against the edition you asked for. Audiobooks: runtime
  within 3%, narrator, language, chapter count, and nothing marking a sample or an abridged copy (read with
  ffprobe, which the image now carries). Ebooks: a valid EPUB without DRM whose own ISBN, title, author and
  language match, with a length that fits. A confidence score decides: sure enough (85% by default, Settings →
  Downloads) is checked and ready to file; anything else waits under **Review**, with what was requested next to
  what arrived, the reason with Copy, and Accept, Reject or Reject and search again. A rejected download is
  never grabbed again for that book.
- OPDS catalogue at `/opds` for e-readers and reading apps (KOReader, Moon+ Reader, Thorium …): new books,
  audiobooks, ebooks, authors, series and search, with covers and every file to download. Sign in with your
  Folio username and password. How to set each app up is in `docs/COMPAT.md`.
- `docs/TRACKS.md`: the parallel tracks, the files each one owns, and the schema and playback API they share. The
  library keeps each book's files (`edition_files`), and the library folder is a setting (Settings → Downloads).
- `docs/GOALS.md`: the goal, word for word.
- Downloads through Lantern only (Settings → Downloads: its address and API key). Searches are built from the
  edition (ASIN or ISBN, author and title, title and narrator) and every result is scored on all of them, the
  file format, language, size and seeders, with the reasons; samples, abridged copies, other languages and file
  formats the preferences don't take are refused. The best result is grabbed (Search now, or every 6 hours);
  Results shows every result with Grab; Downloads follows each grab until its files arrive. Diagnostics checks
  Lantern. The contract is in `docs/LANTERN.md`.
- Migration reader (`server/crates/migrate`): reads Audiobookshelf, Calibre, Calibre-Web, Readarr, LazyLibrarian
  and Listenarr databases into one plan (books and files, wanted lists, progress and bookmarks, and what was left out
  and why) without changing them; the base of the migration dry run.
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
