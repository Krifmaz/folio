//! Readarr's authors, books, editions, series and book files (PARITY rows R1): every request of
//! `parity/readarr/requests.json` is sent to Folio, loaded with the same test library as the recording
//! (`parity/readarr/seed.sql` and its files, `seed-files.json`), and must answer like Readarr 0.4.19.2811 did:
//! status, every field, order and errors. Only trace ids and the .NET stack traces in error descriptions may differ.
//!
//! The lookups ask a stand-in for Readarr's metadata server and Goodreads, served here from the same fixed answers
//! the recording used (`parity/readarr/stub/routes.json`, which `stub.py` served to Readarr).
//!
//! The deletes and adds are also checked by what they leave behind: at each `snapshot` entry (once the work Folio
//! does after answering is done) the rows of the tables they change and the files under the books folder must be
//! Readarr's (`parity/readarr/state/<name>.json`), except the times of the history rows and of the columns the
//! entry's `during_run` names, written during the run, and the layout of the JSON documents in the rows (compared
//! by what they hold). A request's `volatile` fields (times set during the run) only need the same type, and its
//! `save` keeps values of its answer for the `{placeholders}` of later requests, as the recorder does.

mod common;
mod parity;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Method, Request, Uri};
use axum::response::{IntoResponse, Response};
use kit_server_core::{Db, SqliteDb};
use parity::{fixture, Matcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tower::ServiceExt;

/// The rows Readarr's API made before the seed was written: the root folder `/books` and the tag `kids`.
const MADE_BY_THE_API: &str = "INSERT INTO RootFolders (Id, Path, Name, DefaultMetadataProfileId, DefaultQualityProfileId,
    DefaultMonitorOption, DefaultTags, IsCalibreLibrary, CalibreSettings, DefaultNewItemMonitorOption)
    VALUES (1, '/books/', 'Books', 1, 1, 0, '[]', 0, NULL, 0);
    INSERT INTO Tags (Id, Label) VALUES (1, 'kids');";

/// Where Folio answers differently on purpose (docs/DECISIONS.md): `(fixture, path)`. Readarr caches statistics
/// and its book editor forgets to clear the cache, so after `book_editor` it still counts Small Gods as
/// unmonitored; Folio counts what is stored.
const DIFFERENT_ON_PURPOSE: &[(&str, &str)] = &[("authors_after_changes", "$[0].statistics."), ("books_after_changes", "$[1].statistics.")];

/// The history rows the seed and the `sql` entries write; later ones are written during the run, at a time
/// that differs.
const SEEDED_HISTORY: i64 = 2;

/// The recorder's folder (requests, seed, states).
fn recorder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/readarr")
}

/// Makes each `[path under /books, size]` file under `books`, filled with zeros, as the recorder does.
fn write_files(books: &Path, files: &Value) {
    for f in files.as_array().into_iter().flatten() {
        let path = books.join(f[0].as_str().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0u8; f[1].as_u64().unwrap() as usize]).unwrap();
    }
}

/// The files and folders under `books` (folders end with `/`), sorted, as the recorder lists them.
fn listing(books: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if e.path().is_dir() {
                out.push(format!("{rel}/"));
                walk(root, &e.path(), out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(books, books, &mut out);
    out.sort();
    out
}

/// What the requests so far left behind, in the recorder's form: `{"tables": {name: rows}, "files": [...]}`.
fn state(db: &SqliteDb, books: &Path, tables: &[String]) -> Value {
    let mut t = serde_json::Map::new();
    for name in tables {
        let rows: Vec<Value> = db
            .query(&format!("SELECT * FROM \"{name}\" ORDER BY Id"), vec![])
            .unwrap()
            .into_iter()
            .map(|r| Value::Object(r.into_iter().collect()))
            .collect();
        t.insert(name.clone(), Value::Array(rows));
    }
    json!({ "tables": t, "files": listing(books) })
}

/// A row with its JSON documents read: Readarr writes them indented and Folio compactly, and both read either, so
/// they are compared by what they hold.
fn read_documents(rows: &mut Value) {
    for r in rows.as_array_mut().into_iter().flatten() {
        for v in r.as_object_mut().into_iter().flat_map(|o| o.values_mut()) {
            if let Some(doc) = v.as_str().filter(|s| s.starts_with(['{', '['])).and_then(|s| serde_json::from_str::<Value>(s).ok()) {
                *v = doc;
            }
        }
    }
}

/// The differences between Folio's state and Readarr's at a snapshot, leaving out the times of history rows
/// written during the run and the columns `during_run` names (times like `Added`, set when the row is written).
fn state_diffs(name: &str, ours: &Value, theirs: &Value, during_run: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if ours["files"] != theirs["files"] {
        out.push(format!("files {} where Readarr has {}", ours["files"], theirs["files"]));
    }
    for (table, want) in theirs["tables"].as_object().unwrap() {
        let mut want = want.clone();
        let mut got = ours["tables"][table].clone();
        read_documents(&mut want);
        read_documents(&mut got);
        for rows in [&mut want, &mut got] {
            for r in rows.as_array_mut().into_iter().flatten() {
                for col in during_run {
                    if r[col.as_str()].as_str().is_some_and(|t| !t.starts_with("0001-")) {
                        r[col.as_str()] = json!("(during the run)");
                    }
                }
            }
        }
        if table == "History" {
            for rows in [&mut want, &mut got] {
                for r in rows.as_array_mut().into_iter().flatten() {
                    if r["Id"].as_i64().unwrap_or(0) > SEEDED_HISTORY {
                        r["Date"] = json!("(during the run)");
                    }
                }
            }
        }
        if got != want {
            out.push(format!("{table}: {got} where Readarr has {want}"));
        }
    }
    out.into_iter().map(|d| format!("{name}: {d}")).collect()
}

/// Follows a dotted path like `0.id` through lists and objects (the recorder's `dig`).
fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, part| match v {
        Value::Array(a) => a.get(part.parse::<usize>().ok()?),
        _ => v.get(part),
    })
}

/// Fills in the `{placeholders}` saved from earlier answers.
fn fill(text: &str, saved: &HashMap<String, String>) -> String {
    saved.iter().fold(text.to_string(), |t, (k, v)| t.replace(&format!("{{{k}}}"), v))
}

/// The stand-in's answer to a request for `host` (`metadata` or `goodreads`): the route of routes.json with the
/// same method, path, query and (for a POST) JSON body, or 404.
async fn stub_answer(State((host, routes)): State<(&'static str, Vec<Value>)>, method: Method, uri: Uri, body: Bytes) -> Response {
    let mut query = serde_json::Map::new();
    for (k, v) in serde_urlencoded::from_str::<Vec<(String, String)>>(uri.query().unwrap_or_default()).unwrap_or_default() {
        query.entry(k).or_insert(json!(v));
    }
    let body: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap_or(Value::Null) };
    let route = routes.iter().find(|r| {
        r["host"] == json!(host)
            && r["method"] == json!(method.as_str())
            && r["path"] == json!(uri.path())
            && r.get("query").cloned().unwrap_or(json!({})) == Value::Object(query.clone())
            && r.get("request").is_none_or(|want| *want == body)
    });
    let Some(r) = route else { return (axum::http::StatusCode::NOT_FOUND, "{\"error\":\"no stub route\"}").into_response() };
    let status = axum::http::StatusCode::from_u16(r["status"].as_u64().unwrap() as u16).unwrap();
    let (kind, text) = match &r["body"] {
        Value::String(s) => ("text/plain", s.clone()),
        b => ("application/json", b.to_string()),
    };
    let mut res = (status, [("content-type", kind)], text).into_response();
    if let Some(l) = r["location"].as_str() {
        res.headers_mut().insert("location", l.parse().unwrap());
    }
    res
}

/// Serves the stand-ins on two local ports: (the metadata server's address, Goodreads' address).
async fn stub() -> (String, String) {
    let routes: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(recorder().join("stub/routes.json")).unwrap()).unwrap();
    let mut addrs = Vec::new();
    for host in ["metadata", "goodreads"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        addrs.push(format!("http://{}", listener.local_addr().unwrap()));
        let app = axum::Router::new().fallback(stub_answer).with_state((host, routes.clone()));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    }
    (addrs[0].clone(), addrs[1].clone())
}

#[tokio::test]
async fn every_recorded_answer_of_authors_books_and_files_matches() {
    let disk = std::env::temp_dir().join(format!("folio-readarr-disk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&disk);
    let books = disk.join("books");
    let (metadata, goodreads) = stub().await;
    let env = HashMap::from([
        ("READARR_DISK_ROOT".to_string(), disk.to_string_lossy().into_owned()),
        ("READARR_GOODREADS_URL".to_string(), goodreads),
    ]);
    let (app, st, dir) = common::server("parity-readarr-books", env).await;
    st.readarr.execute_batch(MADE_BY_THE_API).unwrap();
    st.readarr.execute("INSERT INTO Config (Key, Value) VALUES ('metadatasource', ?)", vec![json!(format!("{metadata}/"))]).unwrap();
    st.readarr.execute_batch(&std::fs::read_to_string(recorder().join("seed.sql")).unwrap()).unwrap();
    write_files(&books, &serde_json::from_str(&std::fs::read_to_string(recorder().join("seed-files.json")).unwrap()).unwrap());
    let requests: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(recorder().join("requests.json")).unwrap()).unwrap();
    let mut failed = Vec::new();
    let mut saved: HashMap<String, String> = HashMap::new();
    for r in &requests {
        let name = r["name"].as_str().unwrap();
        if let Some(sql) = r["sql"].as_str() {
            write_files(&books, &r["files"]);
            st.readarr.execute_batch(sql).unwrap();
            continue;
        }
        if r["snapshot"] == json!(true) {
            // Readarr finishes some work after it answers (an added author's books); so does Folio.
            st.readarr_jobs.idle().await;
            let during_run: Vec<String> =
                r["during_run"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            let theirs: Value =
                serde_json::from_str(&std::fs::read_to_string(recorder().join(format!("state/{name}.json"))).unwrap()).unwrap();
            let tables: Vec<String> = theirs["tables"].as_object().unwrap().keys().cloned().collect();
            failed.extend(state_diffs(name, &state(&st.readarr, &books, &tables), &theirs, &during_run));
            continue;
        }
        let fx = fixture("readarr", name);
        let path = fill(&fx.request["path"].as_str().unwrap().replace("{apikey}", "test-key"), &saved);
        let mut req = Request::builder().method(fx.request["method"].as_str().unwrap_or("GET")).uri(&path);
        if r.get("auth") != Some(&Value::Bool(false)) {
            req = req.header("x-api-key", "test-key");
        }
        let body = match &fx.request["body"] {
            Value::Null => Body::empty(),
            b => {
                req = req.header("content-type", "application/json");
                Body::from(b.to_string())
            }
        };
        let res = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 24).await.unwrap();
        let body = if fx.content_type.contains("json") {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        } else {
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        };
        for (key, at) in r["save"].as_object().into_iter().flatten() {
            if let Some(v) = dig(&body, at.as_str().unwrap()) {
                saved.insert(key.clone(), v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
            }
        }
        let mut m = Matcher::default();
        m.volatile = vec!["traceId".into(), "description".into()];
        m.volatile.extend(r["volatile"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
        let mut diffs = m.diff(&body, &fx.body);
        diffs.retain(|d| !DIFFERENT_ON_PURPOSE.iter().any(|(n, p)| *n == name && d.starts_with(p)));
        if status != fx.status {
            diffs.insert(0, format!("status {status} where Readarr answers {}", fx.status));
        }
        if !diffs.is_empty() {
            failed.push(format!("{name} ({path}):\n  {}", diffs.join("\n  ")));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(disk);
    assert!(failed.is_empty(), "Folio doesn't answer like Readarr:\n{}", failed.join("\n"));
}
