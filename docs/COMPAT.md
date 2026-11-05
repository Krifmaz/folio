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
