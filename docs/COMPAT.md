# Other apps

Folio speaks the protocols of the apps people already read and listen with, so they keep working. Each one signs
in with a Folio account and sees the whole library (per-user libraries come with goal 6). Folio's address below
is written `https://folio.example`; use your own.

The protocols are written fresh from their public descriptions. Folio's code is not taken from the projects
that made them.

## OPDS (e-readers and reading apps)

For KOReader, Moon+ Reader, Thorium, Librera, Panels, Cantook and any other OPDS 1.2 reader.

- **Catalogue address:** `https://folio.example/opds`
- **Username and password:** your Folio ones.

The catalogue has New books, Audiobooks, Ebooks, Authors and Series, and a search (title, author, narrator,
series, ISBN or ASIN). Each book lists its cover and every file, so audiobooks can be downloaded too. Downloads
can resume (ranges).

Written from the [OPDS 1.2 specification](https://specs.opds.io/opds-1.2) and Atom (RFC 4287), with
[OpenSearch 1.1](https://github.com/dewitt/opensearch) for the search.

## KOReader progress sync

KOReader keeps its place in step with Folio: a page turned in KOReader is the book's place in Folio's web reader
and the Android app, and a place reached there sends KOReader to the start of the same chapter.

1. In Folio, open **Settings → Other apps** and set a **KOReader sync password** (at least 8 characters).
   KOReader only ever sends a scrambled copy of a password, which Folio's own password can't be checked
   against, so this one is separate.
2. In KOReader, open a book, then **Tools → Progress sync → Custom sync server** and enter
   `https://folio.example/koreader`.
3. **Login** with your Folio username and the KOReader sync password. (Register is not needed: Folio makes the
   accounts.)
4. Set **Document matching method** to **Binary** (the default). Folio also knows books by their file name.

Books KOReader opens that are not in Folio's library still sync between KOReader devices.

Written from the API of [koreader-sync-server](https://github.com/koreader/koreader-sync-server) (version 1).

## Audiobookshelf apps

The Audiobookshelf phone app, ShelfPlayer, Plappa, Lissen and other Audiobookshelf apps play Folio's audiobooks
and open its ebooks.

- **Server address:** `https://folio.example` (`https://folio.example/abs` works too).
- **Username and password:** your Folio ones.

The apps see two libraries, **Audiobooks** and **Ebooks**, with Continue Listening, Recently Added, series,
authors and search. A place reached in an app is the book's place in Folio's web player and Android app, and the
other way round; the newest place wins, also for books played offline and synced later. Bookmarks are Folio's
bookmarks.

Not there: podcasts, collections and playlists, uploading, editing books from the app, and live updates (the apps
ask again when you pull to refresh).

Written from Audiobookshelf's [API documentation](https://api.audiobookshelf.org) (server version 2.25).
