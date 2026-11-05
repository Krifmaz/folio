//! The library scanner of the Audiobookshelf port (PARITY P1, track C) against the pinned original. The test makes
//! the same test media the recording used (`parity/abs/make_media.sh`), adds a library on it like the recording
//! did, scans it, and compares Folio's rows with the original's rows right after its first scan
//! (`fixtures/abs/_db_scan.json`), and the `metadata.json` files with the ones the original wrote.
//!
//! ffprobe's numbers (durations, bit rates) differ between ffmpeg versions, so Folio's scanner is given the
//! original's own ffprobe answers (`_ffprobe.json`). Ids are matched up across the tables; times, inode numbers and
//! file sizes (the media is made again, so its bytes may differ) are compared by type only.

mod common;
mod parity;

use axum::body::Body;
use axum::http::{header, Request};
use kit_server_core::Db;
use parity::{fixture, Matcher};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

/// The tables the scan writes, in the order they are compared (ids are first met in an item, then its book).
const SCANNED: &[&str] = &["libraries", "libraryFolders", "libraryItems", "books", "authors", "series", "bookAuthors", "bookSeries"];

/// Tables not loaded from the recording: the scanned ones (Folio makes them), the server settings (Folio keeps its
/// own token secret) and the recording's sign-in sessions.
const NOT_LOADED: &[&str] = &["settings", "sessions", "migrationsMeta", "SequelizeMeta"];

/// One recorded table: its rows as column → value.
type Rows = Vec<Map<String, Value>>;

/// The original's rows right after its first scan.
fn recorded_rows() -> HashMap<String, Rows> {
    serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_db_scan.json")).unwrap()).unwrap()
}

/// Loads the recording's rows of the tables the scan doesn't write (the users above all) into Folio.
fn load_other_rows(db: &dyn Db, all: &HashMap<String, Rows>) {
    db.execute("PRAGMA foreign_keys = OFF", vec![]).ok();
    for (table, rows) in all.iter().filter(|(t, _)| !NOT_LOADED.contains(&t.as_str()) && !SCANNED.contains(&t.as_str())) {
        db.execute(&format!("DELETE FROM `{table}`"), vec![]).unwrap();
        for row in rows {
            let cols: Vec<String> = row.keys().map(|k| format!("`{k}`")).collect();
            let marks = vec!["?"; cols.len()].join(", ");
            db.execute(&format!("INSERT INTO `{table}` ({}) VALUES ({marks})", cols.join(", ")), row.values().cloned().collect()).unwrap();
        }
    }
}

/// Makes the recording's test media in `dir` with the recording's own script (needs ffmpeg, as in CI).
fn make_media(dir: &Path) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/abs/make_media.sh");
    let status = std::process::Command::new(script).arg(dir).status().expect("make_media.sh could not be started");
    assert!(status.success(), "make_media.sh failed; it needs ffmpeg and python3");
}

/// A row with its JSON text columns read as JSON and the test's media folder written as the recording's `/media`.
fn tidy(row: &Map<String, Value>, media: &str) -> Value {
    let mut out = Map::new();
    for (k, v) in row {
        let v = match v {
            Value::String(s) if s.starts_with('{') || s.starts_with('[') => serde_json::from_str(s).unwrap_or_else(|_| v.clone()),
            _ => v.clone(),
        };
        out.insert(k.clone(), v);
    }
    serde_json::from_str(&serde_json::to_string(&Value::Object(out)).unwrap().replace(media, "/media")).unwrap()
}

/// A table's rows in a stable order: by a natural key, the same on both sides whatever the ids.
fn ordered(table: &str, rows: &[Value], all: &HashMap<&str, Vec<Value>>) -> Vec<Value> {
    let name_of = |t: &str, id: &Value, field: &str| -> String {
        all.get(t).and_then(|r| r.iter().find(|r| r["id"] == *id)).and_then(|r| r[field].as_str()).unwrap_or_default().to_string()
    };
    let key = |r: &Value| -> String {
        match table {
            "libraryItems" => r["relPath"].as_str().unwrap_or_default().to_string(),
            "books" => r["title"].as_str().unwrap_or_default().to_string(),
            "authors" | "series" => r["name"].as_str().unwrap_or_default().to_string(),
            "bookAuthors" => format!("{}/{}", name_of("books", &r["bookId"], "title"), name_of("authors", &r["authorId"], "name")),
            "bookSeries" => format!("{}/{}", name_of("books", &r["bookId"], "title"), name_of("series", &r["seriesId"], "name")),
            _ => r["path"].as_str().or(r["name"].as_str()).unwrap_or_default().to_string(),
        }
    };
    let mut out = rows.to_vec();
    out.sort_by_key(key);
    out
}

/// Every table the scan wrote, tidied, from Folio's database or from the recording.
fn tables(read: impl Fn(&str) -> Rows, media: &str) -> HashMap<&'static str, Vec<Value>> {
    SCANNED.iter().map(|t| (*t, read(t).iter().map(|r| tidy(r, media)).collect())).collect()
}

#[tokio::test]
async fn a_scan_writes_the_originals_rows() {
    let (app, st, dir) = common::server("abs-scanner", HashMap::new()).await;
    let recorded = recorded_rows();
    load_other_rows(&st.abs, &recorded);

    // The media, and the original's ffprobe answers for it.
    let media_dir = dir.join("media");
    make_media(&media_dir);
    let media = media_dir.display().to_string();
    let probes: HashMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_ffprobe.json")).unwrap()).unwrap();
    assert!(!probes.is_empty(), "the recording has no ffprobe answers");
    server::abs::scanner::probe::use_recorded(probes.into_iter().map(|(p, v)| (p.replacen("/media", &media, 1), v)).collect());

    // Sign in as the recording's root user.
    let login = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"username": "root", "password": "root"}).to_string()))
        .unwrap();
    let r = app.clone().oneshot(login).await.unwrap();
    let v: Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap()).unwrap();
    let token = v["user"]["accessToken"].as_str().or(v["user"]["token"].as_str()).unwrap().to_string();

    // Add the library like the recording did, and compare the answer.
    let mut m = Matcher::default();
    m.volatile.extend(["size", "mtime", "ctime", "birthtime", "lastScanVersion"].map(String::from));
    let fx = fixture("abs", "lib_create");
    let mut body = fx.request["body"].clone();
    body["folders"] = json!([{"fullPath": media}]);
    let req = Request::builder()
        .method("POST")
        .uri("/api/libraries")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    let answer: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes).replace(&media, "/media")).unwrap();
    parity::assert_like_with(&mut m, status, &answer, &fx);

    // Scan it, then compare every table the scan wrote.
    let library_id = answer["id"].as_str().unwrap().to_string();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/libraries/{library_id}/scan"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.clone().oneshot(req).await.unwrap().status().as_u16(), 200, "the scan was not started");
    // The route answers at once and scans in the background: wait until the three items are in and it has ended.
    let count = || st.abs.one("SELECT count(*) AS n FROM libraryItems", vec![]).unwrap().and_then(|r| r["n"].as_i64()).unwrap_or(0);
    for _ in 0..600 {
        if count() >= 3 && !server::abs::scanner::is_scanning(&library_id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    while server::abs::scanner::is_scanning(&library_id) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let folio = tables(|t| st.abs.query(&format!("SELECT * FROM `{t}`"), vec![]).unwrap(), &media);
    let original = tables(|t| recorded.get(t).cloned().unwrap_or_default(), &media);
    let mut diffs = Vec::new();
    let mut item_ids = Vec::new();
    for t in SCANNED {
        let a = Value::Array(ordered(t, &folio[t], &folio));
        let e = Value::Array(ordered(t, &original[t], &original));
        if *t == "libraryItems" {
            let ids = |v: &Value| v.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
            item_ids = ids(&e).into_iter().zip(ids(&a)).collect();
        }
        diffs.extend(m.diff(&a, &e).into_iter().map(|d| format!("{t}{}", d.trim_start_matches('$'))));
    }

    // The metadata.json files the scan wrote for each item.
    let files: HashMap<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_metadata_files.json")).unwrap()).unwrap();
    for (original_id, folio_id) in &item_ids {
        let Some(expected) = files.get(original_id) else { continue };
        let path = st.abs_server.metadata_path.join("items").join(folio_id).join("metadata.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let actual: Value = serde_json::from_str(&text.replace(&media, "/media")).unwrap();
                diffs.extend(
                    m.diff(&actual, expected).into_iter().map(|d| format!("metadata.json of {original_id}{}", d.trim_start_matches('$'))),
                );
            }
            Err(_) => diffs.push(format!("metadata.json of {original_id}: not written")),
        }
    }
    assert!(diffs.is_empty(), "Folio's scan differs from the original's:\n{}", diffs.join("\n"));
}
