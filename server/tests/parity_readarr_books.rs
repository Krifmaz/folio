//! Readarr's authors, books, editions, series and book files (PARITY rows R1): every request of
//! `parity/readarr/requests.json` is sent to Folio, loaded with the same test library as the recording
//! (`parity/readarr/seed.sql`), and must answer like Readarr 0.4.19.2811 did: status, every field, order and
//! errors. Only trace ids and the .NET stack traces in error descriptions may differ.

mod common;
mod parity;

use axum::body::Body;
use axum::http::Request;
use kit_server_core::Db;
use parity::{fixture, Matcher};
use serde_json::Value;
use std::path::PathBuf;
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

/// The recorder's folder (requests and seed).
fn recorder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/readarr")
}

#[tokio::test]
async fn every_recorded_answer_of_authors_books_and_files_matches() {
    let (app, st, dir) = common::server("parity-readarr-books", Default::default()).await;
    st.readarr.execute_batch(MADE_BY_THE_API).unwrap();
    st.readarr.execute_batch(&std::fs::read_to_string(recorder().join("seed.sql")).unwrap()).unwrap();
    let requests: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(recorder().join("requests.json")).unwrap()).unwrap();
    let mut failed = Vec::new();
    for r in &requests {
        let name = r["name"].as_str().unwrap();
        let fx = fixture("readarr", name);
        let path = fx.request["path"].as_str().unwrap().replace("{apikey}", "test-key");
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
        let mut m = Matcher::default();
        m.volatile = vec!["traceId".into(), "description".into()];
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
    assert!(failed.is_empty(), "Folio doesn't answer like Readarr:\n{}", failed.join("\n"));
}
