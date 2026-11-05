# Changelog

## Unreleased

### Fixed
- The mobile app shows its saved language again after a restart; it came back in English before.

### Added
- The server log works like Audiobookshelf's: admins read today's messages on the log page and watch them
  live, messages are kept in one file per day, and the log level and the number of days kept follow the
  settings. Each answer is checked against the original's own.
- Backups work like Audiobookshelf's: admins make, download, upload, restore and remove backups of the database
  and the books' and authors' metadata, and move the backup folder. A schedule makes them on its own, only the
  newest ones you choose to keep stay, and a backup that grows past the size limit is stopped. Each answer is
  checked against the original's own.
- Notifications work like Audiobookshelf's: point Folio at an Apprise server, set up notifications for events
  with their own title and text, test them, and see when each last fired. One that keeps failing is turned off,
  and events that come while one is being sent wait their turn. Each answer is checked against the original's own.
- Server settings work like Audiobookshelf's: admins change the settings and the words ignored when sorting
  titles (sort titles are refreshed), check a schedule before saving it, add and remove their own metadata
  providers, and set the mail server and e-reader devices with who may use each. The task list the apps show is
  in place for the jobs that will use it. Each answer is checked against the original's own.
- The Readarr API can delete authors, books and book files, one or several, with Readarr's options (delete the
  files, add an import list exclusion). Files go to Readarr's recycling bin when one is set. What a delete leaves
  behind (rows and files on disk) is checked against Readarr's own, recorded at four points.
- The home page shelves of a book library (`/api/libraries/:id/personalized`), as Audiobookshelf's apps show them:
  Continue Listening and Reading, Continue Series, Recently Added, Recent Series, Discover, Listen and Read Again,
  and Newest Authors, each checked against the original's answer on the same data.
- Playback sessions work like Audiobookshelf's: starting one when a player opens a book, syncing and closing it,
  sessions phones played offline, the admins' list of open and past sessions (and deleting them), and each user's
  listening sessions and stats. Each answer is checked against the original's recorded one.
- Audiobookshelf's library scanner for book libraries: adding a library and scanning it finds audiobooks and
  ebooks in their folders, reads their tags, chapters, OPF, NFO, `desc.txt`, `reader.txt` and `metadata.json`
  files the original's way, and adds, updates or marks missing the books, authors and series. The parity recording
  now also keeps the original's ffprobe answers, so the test compares Folio's scan with the original's own rows.
- The mobile app's library list, book page, account page and log are checked against a Folio server holding the
  original's recorded library (PARITY M0001, M0007, M0019, M0023).
- Users, API keys and the sign-in settings work like Audiobookshelf's: admins add, change and remove users and
  their permissions, make API keys that sign in as a user (with an expiry, turned on or off), and set the sign-in
  message, sign-in methods and OpenID app addresses; everyone can change their own password, which signs them out
  on their other devices. Each answer is checked against the original's own.
- The Readarr API can change authors (folder, profiles, tags, monitoring, one at a time or several), books and
  their editions (monitoring, "any edition"), and book files' quality, with Readarr's checks and answers (21 more
  recorded answers).
- Audiobookshelf's library pages: the list of libraries, one library with its filter menus, and a library's items
  with every filter, sort and "collapse series", plus one item plain or expanded. Names sort "Last, First" the
  original's way. The parity recording now also keeps the original's database rows, so these answers are checked
  against the original's own on the same data. The recording is also pushed to a side branch `parity/<branch>`.
- Listening and reading progress and bookmarks work like Audiobookshelf's (`/api/me/progress`, `/api/me/bookmarks`,
  `/api/me/item/:id/bookmark`): set from any app, finished near the end, synced from phones with their own times,
  hidden from Continue Listening, and told to your other apps as they change.
- The web app looks like Folio: the Encore colours and font, the name Folio and its book icon, and links to
  Folio's own repository instead of the upstream project's; its fork changes are listed in `abs-web/FOLIO.md`.
- The web app is now Audiobookshelf's (built from `abs-web/` for the root of the server, served at `/`). Folio's
  own pages (Wanted, Downloads, Review, Move in) move to `/folio/` and are linked from its sidebar.
- The mobile app's settings page is checked in CI: each of its 24 device settings is changed on the page and
  must be saved and kept after a restart.
- The mobile app signs in to Folio: its connect page names a Folio server, and connecting and signing in were
  checked against a running Folio server.
- The base of the Readarr port: its database (`<data>/readarr.db`) with the original's exact schema and first-start
  rows, and its API v1 for reading authors, books, editions, series and book files, answering exactly like Readarr
  0.4.19.2811 (checked against 34 recorded answers, errors included). Readarr's API key is Folio's.
- Sign-in works like Audiobookshelf's: setting up the server, signing in with a name and password (the mobile app
  gets its tokens in the answer, the web app a cookie), refreshing and signing out, old tokens, API keys, the limit
  on sign-in attempts, and the apps' live connection. Each answer is checked against the original's own.
- The mobile app (`abs-app/`) looks like Folio: the Encore colours and font in its default, Black and Light
  themes and in the reader, the name Folio with its own app id (it installs beside the Audiobookshelf app), a
  book icon, and no links out to the upstream project.
- The mobile app (`abs-app/`, the Audiobookshelf app fork) is built for Android in CI on every change to it;
  its fork changes are listed in `abs-app/FOLIO.md`.
- The base of the Audiobookshelf port: its database with the original's exact schema, a row type for each of its 26
  tables, its socket.io (apps connect, ping, and are told when a token is refused), and recordings of 65 of the
  original's answers that parity tests compare Folio with.
- Audiobookshelf's web app (v2.37.2) and mobile app (v0.14.2-beta) are imported as light forks in `abs-web/` and
  `abs-app/`, unchanged for now; the next steps retheme them and point them at Folio.
- The port plan: Folio now ports Audiobookshelf and Readarr instead of reimagining them. `docs/PARITY.md` lists
  every endpoint, setting, page, action and job of the originals (989 rows) with its status and proof;
  `UPSTREAM.lock` pins the exact upstream versions, and `scripts/upstream-merge.sh` keeps the forked apps up to
  date (see `UPSTREAM.md`).
- Move in hands migrated books' files to the import: each is checked like a download, then filed in the library
  with hardlinks (a copy across disks) or put in the Review queue; the old app's files stay where they are. Files
  Folio can't see are listed with what to do (mount the book folder read-only) and tried again on the next run.
- Move in (`#/migrate`, admins; `/api/migrate/*`): check Audiobookshelf, Calibre, Calibre-Web, Readarr,
  LazyLibrarian or Listenarr with a dry run (what is in Folio already, what would be
  added, whose places have an account to go to, what needs you and why), then bring it in. Exact editions by ASIN or
  ISBN only, never guessed; the wanted list; everyone's places and bookmarks (an older place never replaces a newer
  one); migrated books' files wait for the import. Running it again adds nothing twice.
- Download a book to the browser (Download on the player and the reader): it then opens, plays and reads with no
  connection, at the place you were; places made offline go to the server when the connection is back.
- Listen and Read in the web app. The audiobook player plays a book's files as one book with its chapters, speed,
  a sleep timer (minutes or the end of the chapter), bookmarks with notes, media keys and the lock screen. The EPUB
  reader shows the book in pages in the app's colours, with its contents, text size, bookmarks and page turns by
  buttons, keys or a tap on the page's edge; the book's own scripts never run. Both keep your place on the server
  (every device carries on from it) and keep it in the browser until the server can be reached again.
- Other apps page (in the sidebar): the addresses for OPDS, Audiobookshelf apps, KOReader and Kobo, each with
  Copy, the KOReader sync password and the Kobo sync address.
- Kobo sync: a Kobo e-reader syncs Folio's ebooks over Wi-Fi with covers, series and places kept in step
  with Folio. Its address is on the Other apps page.
- Audiobookshelf apps work with Folio (the official app, ShelfPlayer, Plappa …): sign in with your Folio
  account, browse Audiobooks and Ebooks, play with chapters, and keep your place and bookmarks in step with Folio,
  offline listening included.
- KOReader progress sync at `/koreader`: KOReader's place is the book's place in Folio and back (to the
  chapter), with a KOReader sync password of its own; books outside the library sync between KOReaders too.
- OPDS catalogue at `/opds` for e-readers and reading apps (KOReader, Moon+ Reader, Thorium …): new books,
  audiobooks, ebooks, authors, series and search, with covers and every file to download. Sign in with your
  Folio username and password. How to set each app up is in `docs/COMPAT.md`.
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

### Removed
- The first Audiobookshelf compatibility layer (it answered as server 2.25 from Folio's own library): the port of
  Audiobookshelf answers the apps now, with Audiobookshelf's own users.
