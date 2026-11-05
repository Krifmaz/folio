//! The Audiobookshelf port's playback (PARITY P3): media progress and bookmarks under `/api/me`, each compared
//! with the answer recorded from the pinned original (tests/parity/fixtures/abs), plus the original's rules for
//! the cases the recordings don't cover (finishing, batches, hiding, renaming and removing).

mod common;
mod parity;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use kit_server_core::Db;
use parity::{assert_like_with, fixture, Matcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use tower::ServiceExt;

/// The library item the recordings used (`{item}`), its book and library.
const ITEM: &str = "219e8d3a-70a4-425f-a31e-8e92423148cf";
const BOOK: &str = "79cd5675-c67e-4d96-8856-f1a8f24becc1";
const LIBRARY: &str = "773bf283-748c-431f-87f4-654a3cebc280";

/// One answer: status, content type (without parameters) and body (JSON, else text).
struct Answer {
    status: u16,
    content_type: String,
    body: Value,
}

/// Sends a request with an optional JSON body and a bearer token.
async fn send(app: &Router, method: &str, path: &str, body: Option<Value>, token: &str) -> Answer {
    let b = Request::builder().method(method).uri(path).header(header::AUTHORIZATION, format!("Bearer {token}"));
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let content_type = r
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .to_string();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    Answer { status, content_type, body }
}

/// Checks status, content type and body against a recording.
fn like(m: &mut Matcher, a: &Answer, name: &str) {
    let fx = fixture("abs", name);
    assert_eq!(a.content_type, fx.content_type, "{name}: content type");
    assert_like_with(m, a.status, &a.body, &fx);
}

/// A recording's path with the recordings' ids filled in.
fn path_of(name: &str) -> String {
    fixture("abs", name).request["path"].as_str().unwrap().replace("{item}", ITEM)
}

/// A set-up server with the root user (root / root), a library and one audiobook, like the recordings had;
/// answers the app and the root user's access token.
async fn set_up(name: &str) -> (Router, server::AppState, String) {
    let (app, st, _dir) = common::server(name, HashMap::new()).await;
    let now = "2026-11-05 18:00:00.000 +00:00";
    st.abs
        .execute(
            "INSERT INTO libraries (id, name, displayOrder, mediaType, createdAt, updatedAt) VALUES (?, 'Books', 1, 'book', ?, ?)",
            vec![json!(LIBRARY), json!(now), json!(now)],
        )
        .unwrap();
    st.abs
        .execute(
            "INSERT INTO books (id, title, duration, explicit, abridged, tags, genres, narrators, audioFiles, chapters, createdAt, updatedAt)
             VALUES (?, 'A Book', 60, 0, 0, '[]', '[]', '[]', '[]', '[]', ?, ?)",
            vec![json!(BOOK), json!(now), json!(now)],
        )
        .unwrap();
    st.abs
        .execute(
            "INSERT INTO libraryItems (id, path, relPath, mediaId, mediaType, isFile, isMissing, isInvalid, libraryFiles, title, libraryId, createdAt, updatedAt)
             VALUES (?, '/books/A Book', 'A Book', ?, 'book', 0, 0, 0, '[]', 'A Book', ?, ?, ?)",
            vec![json!(ITEM), json!(BOOK), json!(LIBRARY), json!(now), json!(now)],
        )
        .unwrap();
    let init = Request::post("/init")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"newRoot": {"username": "root", "password": "root"}}).to_string()))
        .unwrap();
    assert_eq!(app.clone().oneshot(init).await.unwrap().status(), 200);
    let login = Request::post("/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-return-tokens", "true")
        .body(Body::from(json!({"username": "root", "password": "root"}).to_string()))
        .unwrap();
    let r = app.clone().oneshot(login).await.unwrap();
    let b: Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    (app, st, b["user"]["accessToken"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn progress_and_bookmarks_answer_like_the_original() {
    let (app, _st, token) = set_up("abs-playback-progress").await;
    let mut m = Matcher::default();
    let req = |name: &str| fixture("abs", name).request["body"].clone();

    like(&mut m, &send(&app, "GET", &path_of("me_progress_missing"), None, &token).await, "me_progress_missing");
    like(&mut m, &send(&app, "PATCH", &path_of("me_progress_set"), Some(req("me_progress_set")), &token).await, "me_progress_set");
    like(&mut m, &send(&app, "GET", &path_of("me_progress"), None, &token).await, "me_progress");
    like(&mut m, &send(&app, "POST", &path_of("me_bookmark_add"), Some(req("me_bookmark_add")), &token).await, "me_bookmark_add");
    like(&mut m, &send(&app, "GET", "/api/me", None, &token).await, "me_after_bookmark");
}

#[tokio::test]
async fn progress_follows_the_originals_rules() {
    let (app, st, token) = set_up("abs-playback-rules").await;
    let p = format!("/api/me/progress/{ITEM}");
    let get = |a: Answer| a.body;

    // a new progress is only finished when the app says so; an update near the end (less than 10 s left)
    // finishes it, and going back un-finishes it
    send(&app, "PATCH", &p, Some(json!({"currentTime": 55, "duration": 60})), &token).await;
    assert_eq!(get(send(&app, "GET", &p, None, &token).await)["isFinished"], false);
    send(&app, "PATCH", &p, Some(json!({"currentTime": 56})), &token).await;
    let v = get(send(&app, "GET", &p, None, &token).await);
    assert_eq!((v["isFinished"].clone(), v["progress"].clone()), (json!(true), json!(1)), "{v}");
    assert!(v["finishedAt"].is_number());
    send(&app, "PATCH", &p, Some(json!({"currentTime": 20})), &token).await;
    let v = get(send(&app, "GET", &p, None, &token).await);
    assert_eq!((v["isFinished"].clone(), v["finishedAt"].clone()), (json!(false), Value::Null), "{v}");

    // marking finished sets progress 1; un-marking starts over
    send(&app, "PATCH", &p, Some(json!({"isFinished": true})), &token).await;
    let v = get(send(&app, "GET", &p, None, &token).await);
    assert_eq!((v["isFinished"].clone(), v["progress"].clone()), (json!(true), json!(1)));
    send(&app, "PATCH", &p, Some(json!({"isFinished": false})), &token).await;
    let v = get(send(&app, "GET", &p, None, &token).await);
    assert_eq!((v["isFinished"].clone(), v["progress"].clone(), v["currentTime"].clone()), (json!(false), json!(0), json!(0)));

    // a percentage to finish at, from the library's settings
    send(&app, "PATCH", &p, Some(json!({"currentTime": 50, "markAsFinishedPercentComplete": 80})), &token).await;
    assert_eq!(get(send(&app, "GET", &p, None, &token).await)["isFinished"], true);

    // unknown items and podcasts' episodes: the original's text
    let a = send(&app, "PATCH", "/api/me/progress/nope", Some(json!({"currentTime": 1})), &token).await;
    assert_eq!((a.status, a.content_type.as_str(), a.body), (404, "text/html", json!("Library item not found")));
    let a = send(&app, "PATCH", &format!("{p}/nope"), Some(json!({"currentTime": 1})), &token).await;
    assert_eq!((a.status, a.body), (404, json!("Episode not found")));

    // a batch skips what it can't do
    let a = send(&app, "PATCH", "/api/me/progress/batch/update", Some(json!([])), &token).await;
    assert_eq!((a.status, a.body), (400, json!("Missing request payload")));
    let batch = json!([{"libraryItemId": "nope", "currentTime": 3}, {"libraryItemId": ITEM, "currentTime": 7, "duration": 60}]);
    assert_eq!(send(&app, "PATCH", "/api/me/progress/batch/update", Some(batch), &token).await.status, 200);
    let v = get(send(&app, "GET", &p, None, &token).await);
    assert_eq!((v["currentTime"].clone(), v["isFinished"].clone()), (json!(7), json!(false)), "{v}");

    // a phone's sync keeps the time it was made
    send(&app, "PATCH", &p, Some(json!({"currentTime": 8, "lastUpdate": 1_700_000_000_000_i64})), &token).await;
    assert_eq!(get(send(&app, "GET", &p, None, &token).await)["lastUpdate"], json!(1_700_000_000_000_i64));

    // hiding from Continue Listening, then removing, by the progress's own id
    let id = get(send(&app, "GET", &p, None, &token).await)["id"].as_str().unwrap().to_string();
    let a = send(&app, "GET", &format!("/api/me/progress/{id}/remove-from-continue-listening"), None, &token).await;
    assert_eq!(a.body["mediaProgress"][0]["hideFromContinueListening"], true, "{}", a.body);
    let all = get(send(&app, "GET", "/api/me/progress", None, &token).await);
    assert_eq!(all["mediaProgress"].as_array().unwrap().len(), 1);
    assert_eq!(send(&app, "DELETE", &format!("/api/me/progress/{id}"), None, &token).await.body, json!("OK"));
    assert_eq!(send(&app, "DELETE", &format!("/api/me/progress/{id}"), None, &token).await.status, 404);
    let left = st.abs.query("SELECT id FROM mediaProgresses", vec![]).unwrap();
    assert!(left.is_empty());
}

#[tokio::test]
async fn bookmarks_follow_the_originals_rules() {
    let (app, st, token) = set_up("abs-playback-bookmarks").await;
    let b = format!("/api/me/item/{ITEM}/bookmark");
    send(&app, "POST", &b, Some(json!({"time": 12, "title": "One"})), &token).await;
    // the same time again renames it
    let a = send(&app, "POST", &b, Some(json!({"time": 12, "title": "Uno"})), &token).await;
    assert_eq!(a.body["title"], "Uno");
    send(&app, "POST", &b, Some(json!({"time": 30, "title": "Two"})), &token).await;
    let all = send(&app, "GET", "/api/me/bookmarks", None, &token).await.body;
    assert_eq!(all["bookmarks"].as_array().unwrap().len(), 2, "{all}");
    let mine = send(&app, "GET", &format!("/api/me/bookmarks/{ITEM}"), None, &token).await.body;
    assert_eq!(mine["bookmarks"].as_array().unwrap().len(), 2);

    let a = send(&app, "PATCH", &b, Some(json!({"time": 30, "title": "Deux"})), &token).await;
    assert_eq!((a.status, a.body["title"].clone()), (200, json!("Deux")));
    let a = send(&app, "PATCH", &b, Some(json!({"time": 99, "title": "None"})), &token).await;
    assert_eq!(a.status, 404);

    // the original's checks, in its order
    let a = send(&app, "POST", &b, Some(json!({"time": "x", "title": "Bad"})), &token).await;
    assert_eq!((a.status, a.content_type.as_str(), a.body), (400, "text/html", json!("Invalid time")));
    let a = send(&app, "POST", &b, Some(json!({"time": 5})), &token).await;
    assert_eq!((a.status, a.body), (400, json!("Invalid title")));
    let a = send(&app, "POST", "/api/me/item/nope/bookmark", Some(json!({"time": 5, "title": "x"})), &token).await;
    assert_eq!((a.status, a.body), (404, json!("Not Found")));

    // another library's items are refused to a user who may only open some
    st.abs.execute("UPDATE users SET permissions = json_set(permissions, '$.accessAllLibraries', json('false'))", vec![]).unwrap();
    let a = send(&app, "GET", &format!("/api/me/bookmarks/{ITEM}"), None, &token).await;
    assert_eq!((a.status, a.body), (403, json!("Forbidden")));
    st.abs.execute("UPDATE users SET permissions = json_set(permissions, '$.accessAllLibraries', json('true'))", vec![]).unwrap();

    assert_eq!(send(&app, "DELETE", &format!("{b}/12"), None, &token).await.body, json!("OK"));
    assert_eq!(send(&app, "DELETE", &format!("{b}/12"), None, &token).await.status, 404);
    assert_eq!(send(&app, "DELETE", &format!("{b}/abc"), None, &token).await.body, json!("Invalid time"));
    let all = send(&app, "GET", "/api/me/bookmarks", None, &token).await.body;
    assert_eq!(
        all["bookmarks"],
        json!([{"libraryItemId": ITEM, "time": 30, "title": "Deux", "createdAt": all["bookmarks"][0]["createdAt"]}])
    );
}

/// The original's rows after its recording, loaded into Folio (like `parity_abs_library.rs`), without its
/// playback sessions and devices, and with the item's progress back where the recording had it before `play`.
async fn set_up_recorded(name: &str) -> (Router, server::AppState, String, HashMap<String, String>) {
    let (app, st, _dir) = common::server(name, HashMap::new()).await;
    let all: HashMap<String, Vec<serde_json::Map<String, Value>>> =
        serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_db_end.json")).unwrap()).unwrap();
    st.abs.execute("PRAGMA foreign_keys = OFF", vec![]).ok();
    for (table, rows) in all {
        if ["settings", "sessions", "migrationsMeta", "SequelizeMeta"].contains(&table.as_str()) {
            continue;
        }
        st.abs.execute(&format!("DELETE FROM `{table}`"), vec![]).unwrap();
        if ["playbackSessions", "devices", "mediaProgresses"].contains(&table.as_str()) {
            continue;
        }
        for row in rows {
            let cols: Vec<String> = row.keys().map(|k| format!("`{k}`")).collect();
            let marks = vec!["?"; cols.len()].join(", ");
            st.abs
                .execute(&format!("INSERT INTO `{table}` ({}) VALUES ({marks})", cols.join(", ")), row.values().cloned().collect())
                .unwrap();
        }
    }
    let login = Request::post("/login")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-return-tokens", "true")
        .body(Body::from(json!({"username": "root", "password": "root"}).to_string()))
        .unwrap();
    let r = app.clone().oneshot(login).await.unwrap();
    let b: Value = serde_json::from_slice(&axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap()).unwrap();
    let token = b["user"]["accessToken"].as_str().unwrap().to_string();
    let ctx: Value = serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("_lib_context.json")).unwrap()).unwrap();
    let ctx: HashMap<String, String> =
        ctx.as_object().unwrap().iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect();
    let set = fixture("abs", "me_progress_set");
    let path = set.request["path"].as_str().unwrap().replace("{item}", &ctx["item"]);
    assert_eq!(send(&app, "PATCH", &path, Some(set.request["body"].clone()), &token).await.status, 200);
    (app, st, token, ctx)
}

/// Fills `{placeholders}` in every text of a recorded request.
fn fill_value(v: &Value, ctx: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => {
            let mut t = s.clone();
            for (k, val) in ctx {
                t = t.replace(&format!("{{{k}}}"), val);
            }
            Value::String(t)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fill_value(x, ctx)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), fill_value(x, ctx))).collect()),
        other => other.clone(),
    }
}

/// Listening stats keyed by day, weekday and item id as lists sorted by time, and file addresses without their
/// ids: the recording's days, ids and file numbers are not the test's; the amounts and the rest must be the same.
fn by_amount(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if ["days", "dayOfWeek", "items"].contains(&k.as_str()) && x.is_object() {
                    let mut list: Vec<Value> = x.as_object().unwrap().values().cloned().collect();
                    let amount = |e: &Value| e.get("timeListening").unwrap_or(e).as_f64().unwrap_or(0.0);
                    list.sort_by(|a, b| amount(a).total_cmp(&amount(b)));
                    *x = Value::Array(list);
                } else if k == "contentUrl" {
                    // `/api/items/<item>/file/<ino>`: the recording's item id and file numbers are not the test's.
                    if let Some(t) = x.as_str() {
                        let parts: Vec<&str> = t.split('/').collect();
                        *x = json!(parts
                            .iter()
                            .enumerate()
                            .map(|(i, p)| if i == 3 || i == 5 { "*" } else { p })
                            .collect::<Vec<_>>()
                            .join("/"));
                    }
                } else {
                    by_amount(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(by_amount),
        _ => {}
    }
}

#[tokio::test]
async fn playback_sessions_answer_like_the_original() {
    let (app, _st, token, mut ctx) = set_up_recorded("pb-sessions").await;
    let requests: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(parity::dir("abs").join("../../abs/requests.json")).unwrap()).unwrap();
    let names: Vec<String> = requests.iter().map(|r| r["name"].as_str().unwrap().to_string()).collect();
    let (from, to) = (names.iter().position(|n| n == "play").unwrap(), names.iter().position(|n| n == "pb_play_episode_of_book").unwrap());
    let mut m = Matcher::default();
    m.volatile.extend(["date", "dayOfWeek", "ipAddress"].map(String::from));
    for name in &names[from..=to] {
        // A request the recorder skipped (it needed an answer that didn't come) has no recording.
        if !parity::dir("abs").join(format!("{name}.json")).exists() {
            continue;
        }
        let mut fx = fixture("abs", name);
        let path = fill_value(&fx.request["path"], &ctx).as_str().unwrap().to_string();
        let method = fx.request["method"].as_str().unwrap_or("GET").to_string();
        let body = fx.request.get("body").filter(|b| !b.is_null()).map(|b| fill_value(b, &ctx));
        let mut b = Request::builder()
            .method(method.as_str())
            .uri(&path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header("x-forwarded-for", "172.17.0.1");
        if let Some(Value::Object(h)) = fx.request.get("headers") {
            for (k, v) in h {
                b = b.header(k.as_str(), v.as_str().unwrap());
            }
        }
        let req = match body {
            Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
            None => b.body(Body::empty()).unwrap(),
        };
        let r = app.clone().oneshot(req).await.unwrap();
        let status = r.status().as_u16();
        let ctype =
            r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().split(';').next().unwrap().to_string();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
        let mut answer = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        for (key, from) in [("session", "play"), ("session2", "pb_play_firefox"), ("session3", "pb_play_chrome"), ("userid", "pb_me")] {
            if let Some(id) = answer["id"].as_str().filter(|_| name == from) {
                ctx.insert(key.into(), id.to_string());
            }
        }
        if name == "pb_me" {
            continue;
        }
        assert_eq!(ctype, fx.content_type, "{name}: content type");
        by_amount(&mut answer);
        by_amount(&mut fx.body);
        assert_like_with(&mut m, status, &answer, &fx);
    }
}
