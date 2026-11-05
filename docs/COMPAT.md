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

1. In Folio, open **Other apps** in the sidebar and set a **KOReader sync password** (at least 8 characters).
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

Folio is a port of the Audiobookshelf server (v2.37.2, see `docs/PARITY.md`), so the Audiobookshelf phone app,
ShelfPlayer, Plappa, Lissen and other Audiobookshelf apps talk to it as they would to Audiobookshelf.

- **Server address:** `https://folio.example`.
- **Username and password:** the Audiobookshelf ones. The first time, the server asks for a root user to be set
  up, like a new Audiobookshelf; an existing Audiobookshelf database (`absdatabase.sqlite` in the data folder)
  keeps its users, passwords and tokens.

The port is in progress: sign-in, sign-out and the apps' live connection work like the original now; the library,
playback, collections, podcasts and the rest follow track by track (each row of `docs/PARITY.md` says what works).

## Kobo e-readers

A Kobo syncs Folio's ebooks into its library over Wi-Fi, downloads them, and keeps its place in step with Folio
(the newest place wins). Books removed on the Kobo are not synced to it again.

1. In Folio, open **Other apps** in the sidebar and copy your **Kobo sync address**
   (`https://folio.example/kobo/<token>`).
2. Connect the Kobo to a computer by USB and open `.kobo/Kobo/Kobo eReader.conf` (the `.kobo` folder may be
   hidden).
3. Under `[OneStoreServices]`, set `api_endpoint=` to the address. Save, eject the Kobo, and tap **Sync**.

Make a new address in Folio if the old one was shared by mistake; the old one stops working at once. To go back to
the Kobo store, set `api_endpoint=https://storeapi.kobo.com` again. While the Kobo syncs with Folio, books bought
in the Kobo store don't sync to it.

Written from the public descriptions of the Kobo store's sync protocol (as Kobo e-readers speak it). Folio doesn't
pass anything on to the Kobo store.
