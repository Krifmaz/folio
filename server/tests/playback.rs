//! Files, progress and bookmarks through the HTTP API: files are sent whole or in parts, each person keeps their own
//! place, the newest change wins across devices, bookmarks come back in book order.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{admin, call, server, sources};
use kit_server_core::{p, Db};
use serde_json::{json, Value};
use tower::ServiceExt;

/// Adds the edition with this ASIN or ISBN; its edition id.
async fn add(app: &axum::Router, id: Value) -> i64 {
    let (s, v) = call(app, admin("POST", "/api/library/editions", Some(id))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["edition_id"].as_i64().unwrap()
}

/// Makes the member `name` and signs them in; their session cookie.
async fn member(app: &axum::Router, st: &server::AppState, name: &str) -> String {
    server::auth::create_user(&st.db, name, "a long enough password", "user").unwrap();
    let req = Request::post("/api/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "username": name, "password": "a long enough password" }).to_string()))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    r.headers().get(header::SET_COOKIE).unwrap().to_str().unwrap().split(';').next().unwrap().to_string()
}

/// A request as the member with `cookie` (with the CSRF header).
fn as_member(cookie: &str, method: &str, path: &str, body: Option<Value>) -> Request<Body> {
    let b = Request::builder().method(method).uri(path).header(header::COOKIE, cookie).header("x-requested-with", server::NAME);
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn the_newest_place_wins_and_each_person_has_their_own() {
    let (app, st, _) = server("progress", sources().await).await;
    let audio = add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    let (s, v) = call(&app, admin("GET", &format!("/api/progress/{audio}"), None)).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::NOT_FOUND, Some("You haven't opened this book yet.")));

    let put = |body: Value| admin("PUT", &format!("/api/progress/{audio}"), Some(body));
    let (s, v) = call(&app, put(json!({ "position_ms": 60_000, "fraction": 0.1, "device": "android:pixel", "updated_at": 2_000.0 }))).await;
    assert_eq!((s, v["position_ms"].as_i64()), (StatusCode::OK, Some(60_000)), "{v}");
    // an older place from another device doesn't move it back: the one that stands comes back
    let (_, v) = call(&app, put(json!({ "position_ms": 10_000, "fraction": 0.01, "device": "web", "updated_at": 1_000.0 }))).await;
    assert_eq!((v["position_ms"].as_i64(), v["device"].as_str()), (Some(60_000), Some("android:pixel")));
    let (_, v) = call(&app, put(json!({ "position_ms": 90_000, "fraction": 0.2, "device": "web", "updated_at": 3_000.0 }))).await;
    assert_eq!(v["position_ms"].as_i64(), Some(90_000));
    // no time from the device: now, which is newer
    let (_, v) = call(&app, put(json!({ "position_ms": 95_000, "fraction": 0.21, "device": "web" }))).await;
    assert!(v["updated_at"].as_f64().unwrap() > 1_700_000_000.0);

    // the shelf lists it with its title; `since` leaves out older places
    let (_, v) = call(&app, admin("GET", "/api/progress", None)).await;
    assert_eq!(v["progress"].as_array().map(Vec::len), Some(1));
    assert_eq!(v["progress"][0]["format"], "audiobook");
    assert!(!v["progress"][0]["title"].as_str().unwrap_or("").is_empty());
    let (_, v) = call(&app, admin("GET", "/api/progress?since=99999999999", None)).await;
    assert_eq!(v["progress"], json!([]));

    // another person starts at the beginning
    let cookie = member(&app, &st, "reader").await;
    let (s, _) = call(&app, as_member(&cookie, "GET", &format!("/api/progress/{audio}"), None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = call(&app, as_member(&cookie, "GET", "/api/progress", None)).await;
    assert_eq!(v["progress"], json!([]));

    // an ebook's place keeps its locator
    let ebook = add(&app, json!({ "isbn": "9780593135204" })).await;
    let loc = json!({ "href": "OEBPS/ch03.xhtml", "progression": 0.5, "total_progression": 0.3, "cfi": "epubcfi(/6/8!/4/2)" });
    let (s, v) =
        call(&app, as_member(&cookie, "PUT", &format!("/api/progress/{ebook}"), Some(json!({ "locator": loc, "fraction": 0.3 })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((&v["locator"], v["edition_id"].as_i64()), (&loc, Some(ebook)));

    // wrong places are refused in plain words
    let (s, v) = call(&app, put(json!({ "position_ms": -5 }))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("The place in the book can't be before its start.")));
    let (s, v) = call(&app, put(json!({ "locator": "page 4" }))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::BAD_REQUEST, Some("The place in the ebook isn't in the form Folio reads.")));
    let (s, v) = call(&app, admin("PUT", "/api/progress/9999", Some(json!({ "position_ms": 5 })))).await;
    assert_eq!((s, v["detail"].as_str()), (StatusCode::NOT_FOUND, Some("That book isn't in the library any more.")));

    // start again
    let (s, _) = call(&app, admin("DELETE", &format!("/api/progress/{audio}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, admin("GET", &format!("/api/progress/{audio}"), None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bookmarks_are_kept_in_book_order_and_private() {
    let (app, st, _) = server("bookmarks", sources().await).await;
    let audio = add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    let path = format!("/api/editions/{audio}/bookmarks");
    let (s, late) = call(&app, admin("POST", &path, Some(json!({ "position_ms": 500_000, "note": "The reveal" })))).await;
    assert_eq!(s, StatusCode::OK, "{late}");
    let (_, early) = call(&app, admin("POST", &path, Some(json!({ "position_ms": 1_000, "note": "Good start" })))).await;
    let (_, v) = call(&app, admin("GET", &path, None)).await;
    let ids: Vec<i64> = v["bookmarks"].as_array().unwrap().iter().map(|b| b["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![early["id"].as_i64().unwrap(), late["id"].as_i64().unwrap()]);

    let (s, v) = call(&app, admin("PATCH", &format!("/api/bookmarks/{}", early["id"]), Some(json!({ "note": "Start" })))).await;
    assert_eq!((s, v["note"].as_str()), (StatusCode::OK, Some("Start")));

    // someone else neither sees nor changes them
    let cookie = member(&app, &st, "other").await;
    let (_, v) = call(&app, as_member(&cookie, "GET", &path, None)).await;
    assert_eq!(v["bookmarks"], json!([]));
    let (s, _) = call(&app, as_member(&cookie, "PATCH", &format!("/api/bookmarks/{}", early["id"]), Some(json!({ "note": "x" })))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    call(&app, as_member(&cookie, "DELETE", &format!("/api/bookmarks/{}", early["id"]), None)).await;
    let (_, v) = call(&app, admin("GET", &path, None)).await;
    assert_eq!(v["bookmarks"].as_array().map(Vec::len), Some(2));

    let (s, _) = call(&app, admin("DELETE", &format!("/api/bookmarks/{}", early["id"]), None)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = call(&app, admin("GET", &path, None)).await;
    assert_eq!(v["bookmarks"].as_array().map(Vec::len), Some(1));

    // ebook bookmarks go by how far into the book
    let ebook = add(&app, json!({ "isbn": "9780593135204" })).await;
    let path = format!("/api/editions/{ebook}/bookmarks");
    for t in [0.8, 0.2] {
        call(&app, admin("POST", &path, Some(json!({ "locator": { "href": "c.xhtml", "progression": 0.1, "total_progression": t } }))))
            .await;
    }
    let (_, v) = call(&app, admin("GET", &path, None)).await;
    assert_eq!(v["bookmarks"][0]["locator"]["total_progression"], 0.2);
}

/// A request as the owner with extra headers.
fn get_with(path: &str, headers: &[(&str, &str)]) -> Request<Body> {
    let mut b = Request::get(path).header("x-api-key", "test-key");
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    b.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn files_are_sent_whole_or_in_parts() {
    let lib = std::env::temp_dir().join(format!("folio-test-library-{}", std::process::id()));
    std::fs::create_dir_all(lib.join("Andy Weir/Project Hail Mary (2021)")).unwrap();
    let bytes: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(lib.join("Andy Weir/Project Hail Mary (2021)/Project Hail Mary.m4b"), &bytes).unwrap();
    let mut env = sources().await;
    env.insert("APP_LIBRARY_DIR".into(), lib.display().to_string());
    let (app, st, _) = server("files", env).await;
    let audio = add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    let insert = |path: &str, kind: &str, mime: &str, position: i64| {
        st.db
            .execute(
                "INSERT INTO edition_files (edition_id, path, name, kind, mime, size, position, duration_ms, chapters, created_at) VALUES (?,?,?,?,?,?,?,?,?,0)",
                p![audio, path, path.rsplit('/').next().unwrap(), kind, mime, 4096, position, 60_000, r#"[{"title":"Chapter 1","start_ms":0,"end_ms":60000}]"#],
            )
            .unwrap()
    };
    let id = insert("Andy Weir/Project Hail Mary (2021)/Project Hail Mary.m4b", "audio", "audio/mp4", 0);
    let gone = insert("Andy Weir/Project Hail Mary (2021)/missing.m4b", "audio", "audio/mp4", 1);
    let outside = insert("../outside.m4b", "audio", "audio/mp4", 2);

    let (s, v) = call(&app, admin("GET", &format!("/api/library/editions/{audio}/files"), None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["files"].as_array().map(Vec::len), Some(3));
    assert_eq!(v["files"][0]["chapters"][0]["title"], "Chapter 1");
    assert_eq!((v["files"][0]["kind"].as_str(), v["files"][0]["duration_ms"].as_i64()), (Some("audio"), Some(60_000)));

    let r = app.clone().oneshot(get_with(&format!("/api/files/{id}"), &[("accept-encoding", "gzip")])).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()[header::CONTENT_TYPE], "audio/mp4");
    assert_eq!(r.headers()[header::ACCEPT_RANGES], "bytes");
    let tag = r.headers()[header::ETAG].to_str().unwrap().to_string();
    let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    assert_eq!(body.as_ref(), &bytes[..]);

    let r = app.clone().oneshot(get_with(&format!("/api/files/{id}"), &[("range", "bytes=100-199")])).await.unwrap();
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(r.headers()[header::CONTENT_RANGE], "bytes 100-199/4096");
    let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
    assert_eq!(body.as_ref(), &bytes[100..200]);

    let r = app.clone().oneshot(get_with(&format!("/api/files/{id}"), &[("if-none-match", &tag)])).await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_MODIFIED);

    let (s, v) = call(&app, admin("GET", &format!("/api/files/{gone}"), None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(v["detail"], "The file missing.m4b is missing from the library folder. Import the book again, or check the folder.");
    let (s, _) = call(&app, admin("GET", &format!("/api/files/{outside}"), None)).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    let (s, _) = call(&app, admin("GET", "/api/files/99999", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // signed out: nothing
    let r = app.clone().oneshot(Request::get(format!("/api/files/{id}")).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
