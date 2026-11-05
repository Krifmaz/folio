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
