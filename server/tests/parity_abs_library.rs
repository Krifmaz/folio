//! The library side of the Audiobookshelf port (PARITY P1, track C) against the pinned original: the original's
//! own database rows after a recording (`fixtures/abs/_db_end.json`) are loaded into Folio, and every `lib_*`
//! request recorded at the end of that run is replayed and compared with the original's answer.
//!
//! Because the rows are the original's, ids are the same on both sides. Times and the filter menus' cache
//! counters are compared by type only (see [`matcher`]).

mod common;
mod parity;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use kit_server_core::Db;
use parity::{assert_like_with, fixture, Fixture, Matcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use tower::ServiceExt;

/// The tables not loaded from the recording: the server settings hold the original's token secret (Folio keeps
/// its own, so tokens are made by Folio), and the sign-in sessions are the recording's.
const SKIP_TABLES: &[&str] = &["settings", "sessions", "migrationsMeta", "SequelizeMeta"];

/// Loads the original's rows into Folio's Audiobookshelf database, replacing what is there.
fn load_rows(db: &dyn Db) {
    let path = parity::dir("abs").join("_db_end.json");
    let all: HashMap<String, Vec<serde_json::Map<String, Value>>> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut tables: Vec<_> = all.into_iter().filter(|(t, _)| !SKIP_TABLES.contains(&t.as_str())).collect();
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    db.execute("PRAGMA foreign_keys = OFF", vec![]).ok();
    for (table, rows) in tables {
        db.execute(&format!("DELETE FROM `{table}`"), vec![]).unwrap();
        for row in rows {
            let cols: Vec<String> = row.keys().map(|k| format!("`{k}`")).collect();
            let marks = vec!["?"; cols.len()].join(", ");
            let values: Vec<Value> = row.values().cloned().collect();
            db.execute(&format!("INSERT INTO `{table}` ({}) VALUES ({marks})", cols.join(", ")), values).unwrap();
        }
    }
}

/// The values the recording saved for `{placeholders}` (library, item, author, series…).
fn context() -> HashMap<String, String> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_lib_context.json")).unwrap()).unwrap();
    v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect()
}

/// A recorded path with its placeholders filled, `{b64:key}` as the URL-encoded base64 of the value.
fn fill(path: &str, ctx: &HashMap<String, String>) -> String {
    use base64::Engine;
    let mut p = path.to_string();
    for (k, v) in ctx {
        p = p.replace(&format!("{{{k}}}"), v);
        let b = base64::engine::general_purpose::STANDARD.encode(v.as_bytes());
        p = p.replace(&format!("{{b64:{k}}}"), &b.replace('+', "%2B").replace('/', "%2F").replace('=', "%3D"));
    }
    p
}

/// A GET with the token; the status and the body (JSON, else text).
async fn get(app: &Router, path: &str, token: &str) -> (u16, Value) {
    let req = Request::builder().uri(path).header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, body)
}

/// Folio with the recording's rows, and a token of its root user (signed in through `/login`).
async fn set_up(name: &str) -> (Router, String) {
    let (app, _st, token) = set_up_with_state(name).await;
    (app, token)
}

/// [`set_up`], also giving the app's state.
async fn set_up_with_state(name: &str) -> (Router, server::AppState, String) {
    let (app, st, _dir) = common::server(name, HashMap::new()).await;
    load_rows(&st.abs);
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"username": "root", "password": "root"}).to_string()))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let token = v["user"]["accessToken"].as_str().or(v["user"]["token"].as_str()).unwrap().to_string();
    (app, st, token)
}

/// A matcher for the library side. The filter menus' `bookCount`, `authorCount`, `seriesCount` and
/// `podcastCount` are the original's cache counters: it fills them when it builds the cache, which in the recording
/// happened when the library was still empty, so they depend on the server's history, not on the data.
fn matcher() -> Matcher {
    let mut m = Matcher::default();
    m.volatile.extend(["bookCount", "authorCount", "seriesCount", "podcastCount"].map(String::from));
    m
}

/// Replays one recorded request and compares the answers.
async fn replay(app: &Router, token: &str, ctx: &HashMap<String, String>, m: &mut Matcher, name: &str) {
    let mut fx: Fixture = fixture("abs", name);
    let path = fill(fx.request["path"].as_str().unwrap(), ctx);
    let (status, mut body) = get(app, &path, token).await;
    // The original's filter menus were built while its scanner added each author and series to the cache, so they
    // are in scan order; built from the database they are in name order. The scanner's port is proven with its own
    // recording, so here the two lists are compared by content.
    for v in [&mut body, &mut fx.body] {
        for key in ["authors", "series"] {
            if let Some(Value::Array(list)) = v.pointer_mut(&format!("/filterdata/{key}")) {
                list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
            }
        }
    }
    assert_like_with(m, status, &body, &fx);
}

/// Every `lib_*` recording of one kind.
fn recorded(prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(parity::dir("abs"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".json").map(String::from))
        .filter(|n| n.starts_with(prefix))
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn libraries_answer_like_the_original() {
    let (app, token) = set_up("abs-lib-libraries").await;
    let ctx = context();
    let names = recorded("lib_librar");
    assert!(names.len() >= 5, "{names:?}");
    for name in names {
        replay(&app, &token, &ctx, &mut matcher(), &name).await;
    }
}

#[tokio::test]
async fn library_pages_filter_and_sort_like_the_original() {
    let (app, token) = set_up("abs-lib-items").await;
    let ctx = context();
    let names = recorded("lib_items_");
    assert!(names.len() >= 27, "{names:?}");
    for name in names {
        replay(&app, &token, &ctx, &mut matcher(), &name).await;
    }
}

#[tokio::test]
async fn items_answer_like_the_original() {
    let (app, token) = set_up("abs-lib-item").await;
    let ctx = context();
    let names = recorded("lib_item");
    let names: Vec<_> = names.into_iter().filter(|n| !n.starts_with("lib_items_")).collect();
    assert!(names.len() >= 5, "{names:?}");
    for name in names {
        replay(&app, &token, &ctx, &mut matcher(), &name).await;
    }
}

#[tokio::test]
async fn a_user_limited_to_other_libraries_gets_403() {
    use server::abs::models::{self, User};
    let (app, st, root) = set_up_with_state("abs-lib-forbidden").await;
    let ctx = context();
    let guest = User {
        id: Some(models::new_id()),
        username: Some("limited".into()),
        kind: Some("user".into()),
        is_active: Some(true),
        permissions: Some(json!({"accessAllLibraries": false, "librariesAccessible": ["elsewhere"], "accessAllTags": true})),
        created_at: Some(models::date_now()),
        updated_at: Some(models::date_now()),
        ..User::default()
    };
    models::insert(&st.abs, &guest).unwrap();
    let token = st.abs_server.tokens.access_token(&guest);
    let library = format!("/api/libraries/{}", ctx["library"]);
    assert_eq!(get(&app, &library, &token).await, (403, json!("Forbidden")));
    assert_eq!(get(&app, &format!("{library}/items"), &token).await.0, 403);
    assert_eq!(get(&app, &format!("/api/items/{}", ctx["item"]), &token).await, (403, json!("Forbidden")));
    let (status, body) = get(&app, "/api/libraries", &token).await;
    assert_eq!((status, body["libraries"].as_array().map(Vec::len)), (200, Some(0)), "only the libraries the user may open");
    assert_eq!(get(&app, &library, &root).await.0, 200);
    assert_eq!(get(&app, &format!("{library}/items?limit=x"), &root).await.0, 200, "a limit that isn't a number counts as 0");
}
