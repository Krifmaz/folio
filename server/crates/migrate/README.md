# folio-migrate

Reads what another app knows about your books into one plan, which Folio shows as a dry run and then brings in.
It only reads: every database is copied into a scratch folder first and read there, so the source app can keep
running and nothing of it is ever changed.

| Source | Point it at | What it gives |
| --- | --- | --- |
| Audiobookshelf (2.3+) | `absdatabase.sqlite` or its `/config` folder | audiobooks and ebooks with their files, listening and reading progress, bookmarks |
| Calibre | the library folder or `metadata.db` | ebooks (best format first) and any audiobooks, with their files |
| Calibre-Web | `app.db` | finished and reading marks, web reader places and Kobo progress for the Calibre books |
| Readarr | `readarr.db` | wanted books: monitored, no file, with the exact edition (ISBN or ASIN) |
| LazyLibrarian | `lazylibrarian.db` | wanted ebooks and audiobooks |
| Listenarr | `listenarr.db` | wanted audiobooks by ASIN, with narrators |

```rust
let plan = migrate::read(migrate::Source::Calibre, Path::new("/import/Calibre Library"), &data_dir.join("tmp"))?;
println!("{:?}", plan.summary());
```

`migrate::detect` names the source of a database file by its tables. Errors are plain sentences that say what to
do next. Books, wanted entries and progress point at each other by source id (`abs:<item>`, `calibre:<id>`), so
Calibre-Web's progress lands on the books read from the Calibre library.

Matching the plan to Folio's editions and writing it (the real run) is in the server's `migration` module.
The tests build each source's database in its real shape and check the source is unchanged after reading.
