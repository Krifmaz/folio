//! Each source read from a small database in its own shape (the tables and columns the real apps make), and the
//! promise that reading never changes the source.

mod common;

use common::{database, fingerprint, folder, is_empty};
use migrate::{detect, read, Format, Source};
use rusqlite::Connection;
use std::path::PathBuf;

/// An Audiobookshelf database: a book with audio files and an ebook, an audiobook with a bookmark, a missing item
/// and a podcast; progress for two users.
const ABS: &str = r#"
CREATE TABLE libraries (id TEXT PRIMARY KEY, name TEXT, mediaType TEXT);
CREATE TABLE libraryItems (id TEXT PRIMARY KEY, ino TEXT, path TEXT, relPath TEXT, mediaId TEXT, mediaType TEXT, isFile INTEGER,
  isMissing INTEGER, isInvalid INTEGER, libraryId TEXT, title TEXT, createdAt DATETIME, updatedAt DATETIME);
CREATE TABLE books (id TEXT PRIMARY KEY, title TEXT, titleIgnorePrefix TEXT, subtitle TEXT, publishedYear TEXT, publishedDate TEXT,
  publisher TEXT, description TEXT, isbn TEXT, asin TEXT, language TEXT, explicit INTEGER, abridged INTEGER, coverPath TEXT,
  duration FLOAT, narrators JSON, audioFiles JSON, ebookFile JSON, chapters JSON, tags JSON, genres JSON);
CREATE TABLE authors (id TEXT PRIMARY KEY, name TEXT, lastFirst TEXT, asin TEXT);
CREATE TABLE bookAuthors (id TEXT PRIMARY KEY, bookId TEXT, authorId TEXT, createdAt DATETIME);
CREATE TABLE series (id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE bookSeries (id TEXT PRIMARY KEY, sequence TEXT, bookId TEXT, seriesId TEXT);
CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT, type TEXT, bookmarks JSON);
CREATE TABLE mediaProgresses (id TEXT PRIMARY KEY, userId TEXT, mediaItemId TEXT, mediaItemType TEXT, duration FLOAT,
  currentTime FLOAT, isFinished INTEGER, hideFromContinueListening INTEGER, ebookLocation TEXT, ebookProgress FLOAT,
  finishedAt DATETIME, extraData JSON, podcastId TEXT, createdAt DATETIME, updatedAt DATETIME);

INSERT INTO libraryItems VALUES
 ('li1','1','/audiobooks/Andy Weir/Project Hail Mary','Andy Weir/Project Hail Mary','b1','book',0,0,0,'lib','Project Hail Mary',NULL,NULL),
 ('li2','2','/audiobooks/Martha Wells/All Systems Red','x','b2','book',0,0,0,'lib','All Systems Red',NULL,NULL),
 ('li3','3','/audiobooks/Gone','x','b3','book',0,1,0,'lib','Gone Book',NULL,NULL),
 ('li4','4','/podcasts/Show','x','p1','podcast',0,0,0,'pod','Show',NULL,NULL);
INSERT INTO books VALUES
 ('b1','Project Hail Mary','Project Hail Mary','A Novel','2021',NULL,NULL,NULL,'9780593135204','B08G9PRS1K','English',0,0,
  '/metadata/items/li1/cover.jpg',58200.5,'["Ray Porter"]',
  '[{"index":2,"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/02.mp3"}},{"index":1,"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/01.mp3"}},{"index":3,"exclude":true,"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/sample.mp3"}}]',
  '{"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/phm.epub"}}',NULL,NULL,NULL),
 ('b2','All Systems Red',NULL,NULL,'2017',NULL,NULL,NULL,NULL,'B06XWJ7ZR8','English',0,0,NULL,23040,'["Kevin R. Free"]',
  '[{"index":1,"metadata":{"path":"/audiobooks/Martha Wells/All Systems Red/asr.m4b"}}]',NULL,NULL,NULL,NULL),
 ('b3','Gone Book',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,0,0,NULL,0,'[]','[]',NULL,NULL,NULL,NULL);
INSERT INTO authors VALUES ('a1','Andy Weir','Weir, Andy',NULL),('a2','Martha Wells','Wells, Martha',NULL);
INSERT INTO bookAuthors VALUES ('ba1','b1','a1','2024-01-01'),('ba2','b2','a2','2024-01-01');
INSERT INTO series VALUES ('s1','The Murderbot Diaries');
INSERT INTO bookSeries VALUES ('bs1','1','b2','s1');
INSERT INTO users VALUES ('u1','azule','admin','[{"libraryItemId":"li2","title":"Good bit","time":125.5,"createdAt":1}]'),
                         ('u2','sam','user','[]');
INSERT INTO mediaProgresses VALUES
 ('p1','u1','b1','book',58200.5,29100.25,0,0,'epubcfi(/6/14!/4/2/1:0)',0.42,NULL,NULL,NULL,NULL,'2024-03-01 10:20:30.000 +00:00'),
 ('p2','u2','b2','book',23040,23040,1,0,NULL,0,NULL,NULL,NULL,NULL,'2024-03-02 10:00:00.000 +00:00'),
 ('p3','u1','e1','podcastEpisode',100,50,0,0,NULL,0,NULL,NULL,'p1',NULL,NULL);
"#;

#[test]
fn audiobookshelf_gives_books_files_progress_and_bookmarks() {
    let dir = folder("abs");
    let src = dir.join("config");
    database(&src.join("absdatabase.sqlite"), ABS);
    let before = fingerprint(&src);
    let scratch = dir.join("scratch");
    // the folder is enough: the file is found in it
    let plan = read(Source::Audiobookshelf, &src, &scratch).unwrap();

    let s = plan.summary();
    assert_eq!((s.audiobooks, s.ebooks, s.progress, s.bookmarks, s.users), (2, 1, 3, 1, 2), "{plan:#?}");
    let phm = plan.books.iter().find(|b| b.source_id == "abs:li1").unwrap();
    assert_eq!(phm.format, Format::Audiobook);
    assert_eq!(
        (phm.title.as_str(), phm.subtitle.as_str(), phm.asin.as_str(), phm.language.as_str()),
        ("Project Hail Mary", "A Novel", "B08G9PRS1K", "English")
    );
    assert_eq!(
        (phm.authors.clone(), phm.narrators.clone(), phm.year, phm.duration_secs),
        (vec!["Andy Weir".into()], vec!["Ray Porter".into()], Some(2021), Some(58200.5))
    );
    // in playing order, without the excluded sample
    assert_eq!(
        phm.files,
        vec![
            PathBuf::from("/audiobooks/Andy Weir/Project Hail Mary/01.mp3"),
            PathBuf::from("/audiobooks/Andy Weir/Project Hail Mary/02.mp3")
        ]
    );
    let ebook = plan.books.iter().find(|b| b.source_id == "abs:li1:ebook").unwrap();
    assert_eq!((ebook.format, ebook.isbn.as_str()), (Format::Ebook, "9780593135204"));
    assert_eq!(ebook.files, vec![PathBuf::from("/audiobooks/Andy Weir/Project Hail Mary/phm.epub")]);
    let asr = plan.books.iter().find(|b| b.source_id == "abs:li2").unwrap();
    assert_eq!((asr.series[0].name.as_str(), asr.series[0].position.as_str()), ("The Murderbot Diaries", "1"));

    let listen = plan.progress.iter().find(|p| p.book == "abs:li1").unwrap();
    assert_eq!((listen.user.as_str(), listen.position_ms, listen.finished), ("azule", Some(29_100_250), false));
    assert!((listen.fraction.unwrap() - 0.5).abs() < 1e-6);
    assert_eq!(listen.updated_at, Some(1709288430.0));
    let read_p = plan.progress.iter().find(|p| p.book == "abs:li1:ebook").unwrap();
    assert_eq!((read_p.cfi.as_str(), read_p.fraction), ("epubcfi(/6/14!/4/2/1:0)", Some(0.42)));
    let done = plan.progress.iter().find(|p| p.book == "abs:li2").unwrap();
    assert_eq!((done.user.as_str(), done.finished, done.fraction), ("sam", true, Some(1.0)));
    assert_eq!(
        (plan.bookmarks[0].book.as_str(), plan.bookmarks[0].position_ms, plan.bookmarks[0].title.as_str()),
        ("abs:li2", 125_500, "Good bit")
    );

    // the missing item and the podcast are left out, with the reason
    assert!(plan.skipped.iter().any(|s| s.item == "abs:li3" && s.reason.contains("missing")), "{:?}", plan.skipped);
    assert!(plan.skipped.iter().any(|s| s.reason.contains("1 podcast is left out")), "{:?}", plan.skipped);

    assert_eq!(fingerprint(&src), before, "the source must not change");
    assert!(is_empty(&scratch), "the scratch copy is removed");
}

/// A Calibre library: one book with EPUB, PDF and an M4B, one without files.
const CALIBRE: &str = r#"
CREATE TABLE books (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL DEFAULT 'Unknown', sort TEXT, timestamp TIMESTAMP,
  pubdate TIMESTAMP, series_index REAL NOT NULL DEFAULT 1.0, author_sort TEXT, isbn TEXT DEFAULT '', lccn TEXT DEFAULT '',
  path TEXT NOT NULL DEFAULT '', flags INTEGER NOT NULL DEFAULT 1, uuid TEXT, has_cover BOOL DEFAULT 0, last_modified TIMESTAMP);
CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT, link TEXT NOT NULL DEFAULT '');
CREATE TABLE books_authors_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, author INTEGER NOT NULL);
CREATE TABLE series (id INTEGER PRIMARY KEY, name TEXT NOT NULL, sort TEXT);
CREATE TABLE books_series_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, series INTEGER NOT NULL);
CREATE TABLE identifiers (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, type TEXT NOT NULL DEFAULT 'isbn', val TEXT NOT NULL);
CREATE TABLE languages (id INTEGER PRIMARY KEY, lang_code TEXT NOT NULL);
CREATE TABLE books_languages_link (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, lang_code INTEGER NOT NULL, item_order INTEGER NOT NULL DEFAULT 0);
CREATE TABLE data (id INTEGER PRIMARY KEY, book INTEGER NOT NULL, format TEXT NOT NULL, uncompressed_size INTEGER NOT NULL, name TEXT NOT NULL);
INSERT INTO books (id, title, pubdate, series_index, path, has_cover) VALUES
  (1, 'Small Gods', '1992-01-01 00:00:00+00:00', 13.0, 'Terry Pratchett/Small Gods (1)', 1),
  (2, 'Empty', '0101-01-01 00:00:00+00:00', 1.0, 'Nobody/Empty (2)', 0);
INSERT INTO authors VALUES (1, 'Terry Pratchett', 'Pratchett, Terry', '');
INSERT INTO books_authors_link VALUES (1, 1, 1);
INSERT INTO series VALUES (1, 'Discworld', 'Discworld');
INSERT INTO books_series_link VALUES (1, 1, 1);
INSERT INTO identifiers VALUES (1, 1, 'isbn', '9780552152976'), (2, 1, 'audible', 'B0036I54I6'), (3, 1, 'goodreads', '34484');
INSERT INTO languages VALUES (1, 'eng');
INSERT INTO books_languages_link VALUES (1, 1, 1, 0);
INSERT INTO data VALUES (1, 1, 'PDF', 10, 'Small Gods - Terry Pratchett'), (2, 1, 'EPUB', 10, 'Small Gods - Terry Pratchett'),
  (3, 1, 'M4B', 10, 'Small Gods - Terry Pratchett');
"#;

#[test]
fn calibre_gives_ebooks_best_format_first_and_audiobooks_apart() {
    let dir = folder("calibre");
    let lib = dir.join("Calibre Library");
    database(&lib.join("metadata.db"), CALIBRE);
    let before = fingerprint(&lib);
    let plan = read(Source::Calibre, &lib, &dir.join("scratch")).unwrap();

    let s = plan.summary();
    assert_eq!((s.ebooks, s.audiobooks, s.skipped), (1, 1, 1), "{plan:#?}");
    let e = plan.books.iter().find(|b| b.source_id == "calibre:1").unwrap();
    let folder = lib.join("Terry Pratchett/Small Gods (1)");
    assert_eq!(e.files, vec![folder.join("Small Gods - Terry Pratchett.epub"), folder.join("Small Gods - Terry Pratchett.pdf")]);
    assert_eq!((e.isbn.as_str(), e.asin.as_str(), e.language.as_str(), e.year), ("9780552152976", "", "eng", Some(1992)));
    assert_eq!((e.series[0].name.as_str(), e.series[0].position.as_str()), ("Discworld", "13"));
    assert_eq!(e.cover, Some(folder.join("cover.jpg")));
    let a = plan.books.iter().find(|b| b.source_id == "calibre:1:audio").unwrap();
    assert_eq!((a.format, a.asin.as_str(), a.isbn.as_str()), (Format::Audiobook, "B0036I54I6", ""));
    assert_eq!(a.files, vec![folder.join("Small Gods - Terry Pratchett.m4b")]);
    assert!(plan.skipped[0].reason.contains("no book files"));
    assert_eq!(fingerprint(&lib), before);
}

/// Calibre-Web's app.db: one finished book, one being read in the web reader and on a Kobo.
const CALIBRE_WEB: &str = r#"
CREATE TABLE user (id INTEGER PRIMARY KEY AUTOINCREMENT, name VARCHAR(64) UNIQUE, email VARCHAR(120), role SMALLINT, password VARCHAR);
CREATE TABLE book_read_link (id INTEGER PRIMARY KEY, book_id INTEGER, user_id INTEGER, read_status INTEGER NOT NULL,
  last_modified DATETIME, last_time_started_reading DATETIME, times_started_reading INTEGER NOT NULL DEFAULT 0);
CREATE TABLE bookmark (id INTEGER PRIMARY KEY, user_id INTEGER, book_id INTEGER, format VARCHAR, bookmark_key VARCHAR);
CREATE TABLE kobo_reading_state (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER, book_id INTEGER, last_modified DATETIME, priority_timestamp DATETIME);
CREATE TABLE kobo_bookmark (id INTEGER PRIMARY KEY, kobo_reading_state_id INTEGER, last_modified DATETIME, location_source VARCHAR,
  location_type VARCHAR, location_value VARCHAR, progress_percent FLOAT, content_source_progress_percent FLOAT);
INSERT INTO user (id, name) VALUES (1, 'admin'), (2, 'Guest');
INSERT INTO book_read_link (book_id, user_id, read_status, last_modified) VALUES (1, 1, 1, '2024-03-01 10:20:30.000000'),
  (2, 1, 2, '2024-03-01 10:00:00.000000'), (3, 1, 0, NULL);
INSERT INTO bookmark (user_id, book_id, format, bookmark_key) VALUES (1, 2, 'EPUB', 'epubcfi(/6/8!/4/2/10/1:0)');
INSERT INTO kobo_reading_state (id, user_id, book_id) VALUES (1, 1, 2);
INSERT INTO kobo_bookmark (kobo_reading_state_id, last_modified, progress_percent) VALUES (1, '2024-03-02 10:00:00.000000', 37.0);
"#;

#[test]
fn calibre_web_gives_read_marks_and_places_for_calibre_books() {
    let dir = folder("cweb");
    let db = dir.join("app.db");
    database(&db, CALIBRE_WEB);
    let before = fingerprint(&dir);
    let plan = read(Source::CalibreWeb, &db, &dir.join("scratch")).unwrap();
    assert_eq!(plan.progress.len(), 2, "unread books give no progress: {plan:#?}");
    let done = plan.progress.iter().find(|p| p.book == "calibre:1").unwrap();
    assert_eq!((done.user.as_str(), done.finished, done.fraction, done.updated_at), ("admin", true, Some(1.0), Some(1709288430.0)));
    let reading = plan.progress.iter().find(|p| p.book == "calibre:2").unwrap();
    assert_eq!((reading.finished, reading.fraction, reading.cfi.as_str()), (false, Some(0.37), "epubcfi(/6/8!/4/2/10/1:0)"));
    assert_eq!(reading.updated_at, Some(1709373600.0), "the newest change wins");
    assert!(dir.join("app.db").is_file());
    let mut after = fingerprint(&dir);
    after.retain(|p, _| !p.starts_with(dir.join("scratch")));
    assert_eq!(after, before);
}

/// Readarr: a missing ebook, a missing audiobook (by its edition's format), one by an audio-only profile, one with
/// a file, one unmonitored and one by an unmonitored author.
const READARR: &str = r#"
CREATE TABLE QualityProfiles (Id INTEGER PRIMARY KEY, Name TEXT, Cutoff INTEGER, Items TEXT NOT NULL);
CREATE TABLE Authors (Id INTEGER PRIMARY KEY, CleanName TEXT, Path TEXT, Monitored INTEGER, QualityProfileId INTEGER, AuthorMetadataId INTEGER UNIQUE);
CREATE TABLE AuthorMetadata (Id INTEGER PRIMARY KEY, ForeignAuthorId TEXT, TitleSlug TEXT, Name TEXT);
CREATE TABLE Books (Id INTEGER PRIMARY KEY, AuthorMetadataId INTEGER, ForeignBookId TEXT, TitleSlug TEXT, Title TEXT, CleanTitle TEXT, Monitored INTEGER, AnyEditionOk INTEGER);
CREATE TABLE Editions (Id INTEGER PRIMARY KEY, BookId INTEGER, ForeignEditionId TEXT, Isbn13 TEXT, Asin TEXT, Title TEXT, TitleSlug TEXT,
  Language TEXT, Format TEXT, IsEbook INTEGER, Monitored INTEGER);
CREATE TABLE BookFiles (Id INTEGER PRIMARY KEY, EditionId INTEGER, Path TEXT);
INSERT INTO QualityProfiles VALUES (1, 'eBook', 3, '[{"quality":1,"items":[],"allowed":true},{"quality":3,"items":[],"allowed":true},{"quality":10,"items":[],"allowed":false}]'),
  (2, 'Spoken', 12, '[{"name":"Audio","items":[{"quality":10,"items":[]},{"quality":12,"items":[]}],"allowed":true},{"quality":3,"items":[],"allowed":false}]');
INSERT INTO AuthorMetadata VALUES (1, 'a1', 'andy-weir', 'Andy Weir'), (2, 'a2', 'martha-wells', 'Martha Wells'), (3, 'a3', 'nobody', 'Not Followed');
INSERT INTO Authors VALUES (1, 'andyweir', '/b/Andy Weir', 1, 1, 1), (2, 'marthawells', '/a/Martha Wells', 1, 2, 2), (3, 'nobody', '/b/x', 0, 1, 3);
INSERT INTO Books VALUES (1, 1, 'gb1', 'phm', 'Project Hail Mary', 'phm', 1, 0), (2, 1, 'gb2', 'martian', 'The Martian', 'martian', 1, 0),
  (3, 2, 'gb3', 'asr', 'All Systems Red', 'asr', 1, 0), (4, 1, 'gb4', 'artemis', 'Artemis', 'artemis', 1, 0),
  (5, 1, 'gb5', 'cheshire', 'Cheshire Crossing', 'cc', 0, 0), (6, 3, 'gb6', 'x', 'Unfollowed Book', 'x', 1, 0);
INSERT INTO Editions VALUES (1, 1, 'e1', '9780593135204', NULL, 'Project Hail Mary', 'phm', 'eng', 'Kindle Edition', 1, 1),
  (2, 2, 'e2', NULL, 'B00B5HZGUG', 'The Martian', 'martian', 'eng', 'Audible Audio', 0, 1),
  (3, 3, 'e3', NULL, 'B06XWJ7ZR8', 'All Systems Red', 'asr', 'eng', 'Hardcover', 0, 1),
  (4, 4, 'e4', '9780553448122', NULL, 'Artemis', 'artemis', 'eng', 'Paperback', 0, 1),
  (5, 5, 'e5', NULL, NULL, 'Cheshire Crossing', 'cc', 'eng', NULL, 0, 1), (6, 6, 'e6', NULL, NULL, 'Unfollowed Book', 'x', 'eng', NULL, 0, 1);
INSERT INTO BookFiles VALUES (1, 4, '/b/Andy Weir/Artemis.epub');
"#;

#[test]
fn readarr_gives_missing_books_with_their_exact_edition_and_format() {
    let dir = folder("readarr");
    let db = dir.join("readarr.db");
    database(&db, READARR);
    let plan = read(Source::Readarr, &db, &dir.join("scratch")).unwrap();
    let got: Vec<(&str, Format, &str, &str)> =
        plan.wanted.iter().map(|w| (w.title.as_str(), w.format, w.isbn.as_str(), w.asin.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("Project Hail Mary", Format::Ebook, "9780593135204", ""),
            ("The Martian", Format::Audiobook, "", "B00B5HZGUG"),
            ("All Systems Red", Format::Audiobook, "", "B06XWJ7ZR8"),
        ],
        "a book with a file, an unmonitored book and an unmonitored author's book are not wanted"
    );
    assert_eq!(
        (plan.wanted[0].source_id.as_str(), plan.wanted[0].authors.clone(), plan.wanted[0].language.as_str()),
        ("readarr:1", vec!["Andy Weir".to_string()], "eng")
    );
}

/// LazyLibrarian: a book wanted as an ebook and an audiobook, one had, one skipped.
const LAZYLIBRARIAN: &str = r#"
CREATE TABLE authors (AuthorID TEXT UNIQUE, AuthorName TEXT UNIQUE, Status TEXT);
CREATE TABLE books (AuthorID TEXT, BookName TEXT, BookSub TEXT, BookIsbn TEXT, BookID TEXT UNIQUE, BookFile TEXT, BookLang TEXT,
  Status TEXT DEFAULT "Skipped", AudioFile TEXT, AudioStatus TEXT DEFAULT "Skipped", Narrator TEXT);
CREATE TABLE series (SeriesID INTEGER UNIQUE, SeriesName TEXT, Status TEXT);
CREATE TABLE member (SeriesID INTEGER, BookID TEXT, WorkID TEXT, SeriesNum TEXT);
CREATE TABLE seriesauthors (SeriesID INTEGER, AuthorID TEXT);
INSERT INTO authors VALUES ('ol1', 'Becky Chambers', 'Active');
INSERT INTO books VALUES ('ol1', 'The Long Way to a Small, Angry Planet', NULL, '9780062444134', 'b1', NULL, 'eng', 'Wanted', NULL, 'Wanted', 'Rachel Dulude'),
  ('ol1', 'Record of a Spaceborn Few', NULL, '9780062699220', 'b2', '/books/x.epub', 'eng', 'Open', NULL, 'Skipped', NULL),
  ('ol1', 'A Closed and Common Orbit', NULL, NULL, 'b3', NULL, 'eng', 'Skipped', NULL, 'Skipped', NULL);
"#;

#[test]
fn lazylibrarian_gives_wanted_ebooks_and_audiobooks() {
    let dir = folder("ll");
    let db = dir.join("lazylibrarian.db");
    database(&db, LAZYLIBRARIAN);
    let plan = read(Source::LazyLibrarian, &db, &dir.join("scratch")).unwrap();
    assert_eq!(plan.wanted.len(), 2, "{plan:#?}");
    let (e, a) = (&plan.wanted[0], &plan.wanted[1]);
    assert_eq!((e.source_id.as_str(), e.format, e.isbn.as_str()), ("lazylibrarian:b1", Format::Ebook, "9780062444134"));
    assert_eq!((a.source_id.as_str(), a.format, a.isbn.as_str()), ("lazylibrarian:b1:audio", Format::Audiobook, ""));
    assert_eq!((a.authors.clone(), a.narrators.clone()), (vec!["Becky Chambers".to_string()], vec!["Rachel Dulude".to_string()]));
}

/// Listenarr: one wanted audiobook, one with a tracked file, one with an old-style file path, one unmonitored.
const LISTENARR: &str = r#"
CREATE TABLE "Audiobooks" ("Id" INTEGER PRIMARY KEY AUTOINCREMENT, "Title" TEXT NULL, "Subtitle" TEXT NULL, "Authors" TEXT NULL,
  "Narrators" TEXT NULL, "Isbn" TEXT NULL, "Asin" TEXT NULL, "Language" TEXT NULL, "Runtime" INTEGER NULL, "Abridged" INTEGER NOT NULL,
  "Monitored" INTEGER NOT NULL, "FilePath" TEXT NULL);
CREATE TABLE "AudiobookFiles" ("Id" INTEGER PRIMARY KEY AUTOINCREMENT, "AudiobookId" INTEGER NOT NULL, "Path" TEXT NULL);
INSERT INTO Audiobooks VALUES (1, 'Dungeon Crawler Carl', NULL, '["Matt Dinniman"]', '["Jeff Hays"]', '[]', 'B08BX57TZW', 'English', 820, 0, 1, NULL),
  (2, 'Had', NULL, '["X"]', '[]', NULL, 'B000000001', 'English', 1, 0, 1, NULL),
  (3, 'Old Path', NULL, '"Y"', NULL, NULL, 'B000000002', 'English', 1, 0, 1, '/audiobooks/Y/Old Path'),
  (4, 'Not Monitored', NULL, NULL, NULL, NULL, 'B000000003', 'English', 1, 0, 0, NULL);
INSERT INTO AudiobookFiles VALUES (1, 2, '/audiobooks/X/Had.m4b');
"#;

#[test]
fn listenarr_gives_wanted_audiobooks_by_asin() {
    let dir = folder("listenarr");
    let db = dir.join("listenarr.db");
    database(&db, LISTENARR);
    let plan = read(Source::Listenarr, &db, &dir.join("scratch")).unwrap();
    assert_eq!(plan.wanted.len(), 1, "{plan:#?}");
    let w = &plan.wanted[0];
    assert_eq!((w.title.as_str(), w.asin.as_str(), w.format), ("Dungeon Crawler Carl", "B08BX57TZW", Format::Audiobook));
    assert_eq!((w.authors.clone(), w.narrators.clone()), (vec!["Matt Dinniman".to_string()], vec!["Jeff Hays".to_string()]));
}

#[test]
fn a_database_still_being_written_is_read_whole_and_left_alone() {
    let dir = folder("wal");
    let db = dir.join("listenarr.db");
    // the app is running: its newest rows are only in the write-ahead log
    let live = Connection::open(&db).unwrap();
    live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;").unwrap();
    live.execute_batch(LISTENARR).unwrap();
    assert!(dir.join("listenarr.db-wal").is_file());
    let before = fingerprint(&dir);
    let plan = read(Source::Listenarr, &db, &dir.join("scratch")).unwrap();
    assert_eq!(plan.wanted.len(), 1, "rows still in the log are read");
    let mut after = fingerprint(&dir);
    after.retain(|p, _| !p.starts_with(dir.join("scratch")));
    let mut before = before;
    // the live app's shared-memory file is its own business
    before.retain(|p, _| !p.to_string_lossy().ends_with("-shm"));
    after.retain(|p, _| !p.to_string_lossy().ends_with("-shm"));
    assert_eq!(after, before, "the database and its log are unchanged");
    drop(live);
}

#[test]
fn the_wrong_file_is_named_in_plain_words() {
    let dir = folder("wrong");
    let db = dir.join("readarr.db");
    database(&db, READARR);
    let e = read(Source::Calibre, &db, &dir.join("scratch")).unwrap_err();
    assert_eq!(e.0, "This looks like a Readarr database, not Calibre. Choose Readarr as the source, or Calibre's metadata.db file.");
    let e = read(Source::Calibre, &dir.join("nowhere"), &dir.join("scratch")).unwrap_err();
    assert!(e.0.contains("can't be found"), "{e}");
    let e = read(Source::Calibre, &dir, &dir.join("scratch")).unwrap_err();
    assert!(e.0.starts_with("There is no metadata.db in"), "{e}");
    std::fs::write(dir.join("notes.txt"), "not a database at all, just some words to make it long enough").unwrap();
    let e = read(Source::Calibre, &dir.join("notes.txt"), &dir.join("scratch")).unwrap_err();
    assert!(e.0.contains("is not a database Folio can read"), "{e}");
    assert!(is_empty(&dir.join("scratch")), "failed reads clean up too");
}

#[test]
fn each_source_is_recognised_by_its_tables() {
    let dir = folder("detect");
    for (name, sql, source) in [
        ("abs.sqlite", ABS, Source::Audiobookshelf),
        ("metadata.db", CALIBRE, Source::Calibre),
        ("app.db", CALIBRE_WEB, Source::CalibreWeb),
        ("readarr.db", READARR, Source::Readarr),
        ("lazylibrarian.db", LAZYLIBRARIAN, Source::LazyLibrarian),
        ("listenarr.db", LISTENARR, Source::Listenarr),
    ] {
        let f = dir.join(name);
        database(&f, sql);
        assert_eq!(detect(&f, &dir.join("scratch")).unwrap(), Some(source), "{name}");
    }
    database(&dir.join("other.db"), "CREATE TABLE things (id INTEGER);");
    assert_eq!(detect(&dir.join("other.db"), &dir.join("scratch")).unwrap(), None);
}
