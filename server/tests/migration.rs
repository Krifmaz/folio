//! Moving in through the HTTP API: a dry run that looks and writes nothing, then the real run that keeps the exact
//! editions, the wanted list, everyone's places and bookmarks, and queues the files for the import, never twice.

mod common;

use axum::http::StatusCode;
use common::{admin, call, server, sources};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

/// An Audiobookshelf database: Project Hail Mary as an audiobook and an ebook, an ASIN no source knows, an
/// audiobook without an ASIN; places for `Admin` (the owner, in another case) and a stranger; a bookmark.
const ABS: &str = r#"
CREATE TABLE libraryItems (id TEXT PRIMARY KEY, path TEXT, mediaId TEXT, mediaType TEXT, isMissing INTEGER);
CREATE TABLE books (id TEXT PRIMARY KEY, title TEXT, subtitle TEXT, publishedYear TEXT, isbn TEXT, asin TEXT, language TEXT,
  abridged INTEGER, coverPath TEXT, duration FLOAT, narrators JSON, audioFiles JSON, ebookFile JSON);
CREATE TABLE authors (id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE bookAuthors (id TEXT PRIMARY KEY, bookId TEXT, authorId TEXT, createdAt DATETIME);
CREATE TABLE series (id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE bookSeries (id TEXT PRIMARY KEY, sequence TEXT, bookId TEXT, seriesId TEXT);
CREATE TABLE users (id TEXT PRIMARY KEY, username TEXT, bookmarks JSON);
CREATE TABLE mediaProgresses (id TEXT PRIMARY KEY, userId TEXT, mediaItemId TEXT, mediaItemType TEXT, duration FLOAT,
  currentTime FLOAT, isFinished INTEGER, ebookLocation TEXT, ebookProgress FLOAT, updatedAt DATETIME);
INSERT INTO libraryItems VALUES ('li1','/audiobooks/Andy Weir/Project Hail Mary','b1','book',0),
  ('li2','/audiobooks/Nobody/Unknown','b2','book',0), ('li3','/audiobooks/Nobody/No Asin','b3','book',0);
INSERT INTO books VALUES
 ('b1','Project Hail Mary',NULL,'2021','9780593135204','B08G9PRS1K','English',0,NULL,58200,'["Ray Porter"]',
  '[{"index":1,"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/phm.m4b"}}]',
  '{"metadata":{"path":"/audiobooks/Andy Weir/Project Hail Mary/phm.epub"}}'),
 ('b2','Unknown Book',NULL,NULL,NULL,'B0UNKNOWN1','English',0,NULL,100,'[]','[{"index":1,"metadata":{"path":"/audiobooks/Nobody/Unknown/u.mp3"}}]',NULL),
 ('b3','No Asin Book',NULL,NULL,NULL,NULL,'English',0,NULL,100,'[]','[{"index":1,"metadata":{"path":"/audiobooks/Nobody/No Asin/n.mp3"}}]',NULL);
INSERT INTO authors VALUES ('a1','Andy Weir');
INSERT INTO bookAuthors VALUES ('ba1','b1','a1','2024-01-01');
INSERT INTO users VALUES ('u1','Admin','[{"libraryItemId":"li1","title":"The beetles","time":600}]'), ('u2','stranger','[]');
INSERT INTO mediaProgresses VALUES
 ('p1','u1','b1','book',58200,29100,0,'epubcfi(/6/14!/4/2/1:0)',0.25,'2024-03-01 10:20:30.000 +00:00'),
 ('p2','u2','b1','book',58200,100,0,NULL,0,'2024-03-01 10:20:30.000 +00:00');
"#;

/// Listenarr: a wanted audiobook the sources know, and an abridged one the preferences refuse.
const LISTENARR: &str = r#"
CREATE TABLE Audiobooks (Id INTEGER PRIMARY KEY, Title TEXT, Authors TEXT, Narrators TEXT, Asin TEXT, Language TEXT, Monitored INTEGER, FilePath TEXT);
CREATE TABLE AudiobookFiles (Id INTEGER PRIMARY KEY, AudiobookId INTEGER, Path TEXT);
INSERT INTO Audiobooks VALUES (1, 'The Second', '["Andy Weir"]', '[]', 'B0SECOND01', 'English', 1, NULL),
  (2, 'Short Version', '["Andy Weir"]', '[]', 'B0ABRIDGE1', 'English', 1, NULL);
"#;

/// Makes the database `file` from `sql`.
fn database(file: &Path, sql: &str) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    Connection::open(file).unwrap().execute_batch(sql).unwrap();
}

/// Starts a run and waits for it to end; the finished run.
async fn run(app: &axum::Router, body: Value) -> Value {
    let (s, v) = call(app, admin("POST", "/api/migrate/runs", Some(body))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["id"].as_i64().unwrap();
    for _ in 0..200 {
        let (_, r) = call(app, admin("GET", &format!("/api/migrate/runs/{id}"), None)).await;
        if r["status"] != "running" {
            assert_eq!(r["status"], "done", "{r}");
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the run didn't end");
}

/// One item of a report by its source id.
fn item<'a>(report: &'a Value, source_id: &str) -> &'a Value {
    report["report"]["items"].as_array().unwrap().iter().find(|i| i["source_id"] == source_id).unwrap_or_else(|| panic!("{source_id}"))
}

/// How many rows `table` has.
fn count(st: &server::AppState, table: &str) -> i64 {
    use kit_server_core::{p, Db, RowExt};
    st.db.one(&format!("SELECT count(*) AS n FROM {table}"), p![]).unwrap().unwrap().i("n")
}

#[tokio::test]
async fn audiobookshelf_is_checked_first_then_brought_in_once() {
    let (app, st, dir) = server("migrate-abs", sources().await).await;
    let db = dir.join("abs/absdatabase.sqlite");
    database(&db, ABS);
    let before = std::fs::read(&db).unwrap();

    // the dry run looks and writes nothing
    let dry = run(&app, json!({ "source": "abs", "path": dir.join("abs") })).await;
    assert_eq!(dry["dry_run"], true);
    let c = &dry["report"]["counts"];
    assert_eq!(
        (c["items"].as_u64(), c["added"].as_u64(), c["progress"].as_u64(), c["bookmarks"].as_u64()),
        (Some(4), Some(3), Some(2), Some(1)),
        "{dry}"
    );
    assert_eq!(item(&dry, "abs:li1")["outcome"], "add");
    assert_eq!(item(&dry, "abs:li3")["outcome"], "no_id");
    assert!(item(&dry, "abs:li3")["reason"].as_str().unwrap().contains("no Audible ASIN"));
    let users = dry["report"]["users"].as_array().unwrap();
    assert_eq!(users.iter().find(|u| u["name"] == "Admin").unwrap()["username"], "admin", "names match in any case");
    assert!(users.iter().find(|u| u["name"] == "stranger").unwrap()["user_id"].is_null());
    assert!(dry["report"]["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["reason"].as_str().unwrap().starts_with("stranger's place is left out")));
    assert_eq!((count(&st, "editions"), count(&st, "progress"), count(&st, "bookmarks"), count(&st, "migration_files")), (0, 0, 0, 0));

    // the real run keeps the exact editions, the places and the bookmark, and queues the files
    let real = run(&app, json!({ "source": "Audiobookshelf", "path": db, "dry_run": false })).await;
    let c = &real["report"]["counts"];
    assert_eq!(
        (c["added"].as_u64(), c["files_queued"].as_u64(), c["progress"].as_u64(), c["bookmarks"].as_u64()),
        (Some(2), Some(2), Some(2), Some(1)),
        "{real}"
    );
    let audio = item(&real, "abs:li1");
    assert_eq!((audio["outcome"].as_str(), audio["key"].as_str()), (Some("added"), Some("B08G9PRS1K")));
    assert_eq!(item(&real, "abs:li1:ebook")["key"], "9780593135204");
    assert_eq!(item(&real, "abs:li2")["outcome"], "not_found");
    let audio_id = audio["edition_id"].as_i64().unwrap();
    let ebook_id = item(&real, "abs:li1:ebook")["edition_id"].as_i64().unwrap();

    let (_, place) = call(&app, admin("GET", &format!("/api/progress/{audio_id}"), None)).await;
    assert_eq!((place["position_ms"].as_i64(), place["device"].as_str()), (Some(29_100_000), Some("migration:abs")), "{place}");
    assert_eq!(place["updated_at"].as_f64(), Some(1709288430.0));
    let (_, read) = call(&app, admin("GET", &format!("/api/progress/{ebook_id}"), None)).await;
    assert_eq!((read["locator"]["cfi"].as_str(), read["fraction"].as_f64()), (Some("epubcfi(/6/14!/4/2/1:0)"), Some(0.25)));
    let (_, marks) = call(&app, admin("GET", &format!("/api/editions/{audio_id}/bookmarks"), None)).await;
    assert_eq!(
        (marks["bookmarks"][0]["position_ms"].as_i64(), marks["bookmarks"][0]["note"].as_str()),
        (Some(600_000), Some("The beetles"))
    );

    let (_, waiting) = call(&app, admin("GET", "/api/migrate/waiting", None)).await;
    let books = waiting["books"].as_array().unwrap();
    assert_eq!(books.len(), 2, "files Folio can't see stay behind");
    let phm = books.iter().find(|b| b["source_id"] == "abs:li1").unwrap();
    assert_eq!(phm["files"], json!(["/audiobooks/Andy Weir/Project Hail Mary/phm.m4b"]));
    assert_eq!(phm["status"], "missing");
    assert!(phm["note"].as_str().unwrap().starts_with("Folio can't see 1 of this book's files"), "{phm}");
    assert_eq!(real["report"]["counts"]["files_handed"], 0);
    assert!(real["report"]["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["item"] == "abs:li1" && s["reason"].as_str().unwrap().contains("Mount the old app's book folder")));

    // again: everything is in Folio already and nothing is added twice
    let again = run(&app, json!({ "source": "abs", "path": db, "dry_run": false })).await;
    assert_eq!(item(&again, "abs:li1")["outcome"], "in_folio");
    assert_eq!(again["report"]["counts"]["bookmarks"], 0);
    assert_eq!((count(&st, "editions"), count(&st, "bookmarks"), count(&st, "migration_files")), (2, 1, 2));

    // a place a person made in Folio since is newer, so the old app's place doesn't replace it
    let body = json!({ "position_ms": 5, "fraction": 0.0, "device": "web", "updated_at": 1800000000.0 });
    call(&app, admin("PUT", &format!("/api/progress/{audio_id}"), Some(body))).await;
    run(&app, json!({ "source": "abs", "path": db, "dry_run": false })).await;
    let (_, place) = call(&app, admin("GET", &format!("/api/progress/{audio_id}"), None)).await;
    assert_eq!(place["position_ms"], 5);

    assert_eq!(std::fs::read(&db).unwrap(), before, "the source is never changed");
    let (_, runs) = call(&app, admin("GET", "/api/migrate/runs", None)).await;
    assert_eq!(runs["runs"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn a_wanted_list_is_brought_in_and_refusals_say_why() {
    let (app, st, dir) = server("migrate-listenarr", sources().await).await;
    let db = dir.join("listenarr.db");
    database(&db, LISTENARR);
    let real = run(&app, json!({ "source": "listenarr", "path": db, "dry_run": false })).await;
    assert_eq!(real["report"]["counts"]["wanted"], 1, "{real}");
    assert_eq!(item(&real, "listenarr:1")["outcome"], "added");
    let refused = item(&real, "listenarr:2");
    assert_eq!(refused["outcome"], "refused");
    assert!(!refused["reason"].as_str().unwrap().is_empty());
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    let list = w["wanted"].as_array().unwrap();
    assert_eq!((list.len(), list[0]["source"].as_str(), list[0]["edition"]["asin"].as_str()), (1, Some("migration"), Some("B0SECOND01")));
    assert_eq!(count(&st, "migration_files"), 0, "wanted books have no files to import");
}

#[tokio::test]
async fn wrong_places_and_calibre_web_alone_are_explained() {
    let (app, _, dir) = server("migrate-errors", sources().await).await;
    let (s, v) = call(&app, admin("POST", "/api/migrate/runs", Some(json!({ "source": "plex", "path": "/x" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().starts_with("Choose the app"));
    let (s, v) = call(&app, admin("POST", "/api/migrate/runs", Some(json!({ "source": "calibre", "path": dir.join("nowhere") })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("can't be found"), "{v}");

    let app_db = dir.join("cw/app.db");
    database(
        &app_db,
        "CREATE TABLE user (id INTEGER PRIMARY KEY, name TEXT); CREATE TABLE book_read_link (id INTEGER PRIMARY KEY, book_id INTEGER,
         user_id INTEGER, read_status INTEGER, last_modified DATETIME); INSERT INTO user VALUES (1, 'admin');
         INSERT INTO book_read_link VALUES (1, 7, 1, 1, NULL);",
    );
    let r = run(&app, json!({ "source": "calibre-web", "path": app_db })).await;
    let skipped = r["report"]["skipped"].as_array().unwrap();
    assert!(skipped[0]["reason"].as_str().unwrap().contains("bring in the Calibre library first"), "{r}");
}

#[tokio::test]
async fn files_folio_can_see_go_to_the_import_and_stay_where_they_are() {
    let (app, st, dir) = server("migrate-handover", sources().await).await;
    let books = dir.join("books/Andy Weir");
    std::fs::create_dir_all(&books).unwrap();
    let epub = books.join("phm.epub");
    std::fs::write(&epub, b"an ebook").unwrap();
    let sql = ABS.replace("/audiobooks/Andy Weir/Project Hail Mary/phm.epub", &epub.to_string_lossy());
    let db = dir.join("abs/absdatabase.sqlite");
    database(&db, &sql);
    let real = run(&app, json!({ "source": "abs", "path": db, "dry_run": false })).await;
    assert_eq!(real["report"]["counts"]["files_handed"], 1, "{real}");

    // the import has it as an arrived download of the edition, to check and file
    use kit_server_core::{p, Db, RowExt};
    let ebook = item(&real, "abs:li1:ebook")["edition_id"].as_i64().unwrap();
    let g = st
        .db
        .one("SELECT g.status, g.indexer FROM grabs g JOIN wanted w ON w.id = g.wanted_id WHERE w.edition_id=?", p![ebook])
        .unwrap()
        .unwrap();
    assert_eq!((g.s("status").as_str(), g.s("indexer").as_str()), ("completed", "Audiobookshelf"));
    let (_, waiting) = call(&app, admin("GET", "/api/migrate/waiting", None)).await;
    assert!(waiting["books"].as_array().unwrap().iter().all(|b| b["source_id"] != "abs:li1:ebook"), "{waiting}");

    // running it again doesn't hand it over twice
    run(&app, json!({ "source": "abs", "path": db, "dry_run": false })).await;
    assert_eq!(count(&st, "grabs"), 1);
    assert_eq!(std::fs::read(&epub).unwrap(), b"an ebook", "the source file stays where it was, unchanged");
}
