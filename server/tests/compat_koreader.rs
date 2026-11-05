//! KOReader's progress sync against Folio: the sync password, signing in, and places kept in step with Folio's
//! own progress both ways.

mod compat_support;

use axum::http::StatusCode;
use compat_support::{common, library, req, text, USER};
use md5::{Digest, Md5};
use serde_json::{json, Value};

/// KOReader's key for a password: its MD5 in hex.
fn md5(s: &[u8]) -> String {
    hex::encode(Md5::digest(s))
}

/// A KOReader request signed in with `password`.
fn ko(method: &str, path: &str, password: &str, body: Option<Value>) -> axum::http::Request<axum::body::Body> {
    let key = md5(password.as_bytes());
    req(method, path, &[("x-auth-user", USER), ("x-auth-key", &key), ("accept", "application/vnd.koreader.v1+json")], body)
}

/// The JSON of an answer.
async fn json_of(lib: &compat_support::Library, r: axum::http::Request<axum::body::Body>) -> (StatusCode, Value) {
    let (s, _, b) = text(&lib.app, r).await;
    (s, serde_json::from_str(&b).unwrap_or(Value::Null))
}

#[tokio::test]
async fn koreader_signs_in_with_the_sync_password_set_in_folio() {
    let lib = library("ko-auth", Default::default()).await;
    let (s, v) = json_of(&lib, ko("GET", "/koreader/users/auth", "kosync pass", None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(v["code"], 2001);
    assert!(v["detail"].as_str().unwrap().contains("Other apps"));
    let (s, v) = json_of(&lib, common::admin("PUT", "/api/apps/koreader", Some(json!({ "password": "short" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v) = json_of(&lib, common::admin("PUT", "/api/apps/koreader", Some(json!({ "password": "kosync pass" })))).await;
    assert_eq!((s, v["set"].as_bool()), (StatusCode::OK, Some(true)), "{v}");
    let (s, v) = json_of(&lib, ko("GET", "/koreader/users/auth", "kosync pass", None)).await;
    assert_eq!((s, v["authorized"].as_str()), (StatusCode::OK, Some("OK")), "{v}");
    let (s, _) = json_of(&lib, ko("GET", "/koreader/users/auth", "other pass", None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    // Register says where accounts come from
    let (s, v) = json_of(&lib, req("POST", "/koreader/users/create", &[], Some(json!({ "username": "x", "password": md5(b"y") })))).await;
    assert_eq!(s, StatusCode::PAYMENT_REQUIRED);
    assert!(v["message"].as_str().unwrap().contains("Folio makes accounts"));
    let (_, v) = json_of(&lib, req("GET", "/koreader/healthcheck", &[], None)).await;
    assert_eq!(v["state"], "OK");
    // forgetting the password stops KOReader
    json_of(&lib, common::admin("DELETE", "/api/apps/koreader", None)).await;
    let (s, _) = json_of(&lib, ko("GET", "/koreader/users/auth", "kosync pass", None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_place_from_koreader_becomes_the_books_place_and_back() {
    let lib = library("ko-sync", Default::default()).await;
    json_of(&lib, common::admin("PUT", "/api/apps/koreader", Some(json!({ "password": "kosync pass" })))).await;
    let path = lib.dir.join("Andy Weir/Mars/01 - The Martian (2014)/The Martian.epub");
    let doc = server::compat::koreader::partial_md5(&path).unwrap();

    // nothing yet
    let (s, v) = json_of(&lib, ko("GET", &format!("/koreader/syncs/progress/{doc}"), "kosync pass", None)).await;
    assert_eq!((s, v), (StatusCode::OK, json!({})));

    let place = json!({ "document": doc, "progress": "/body/DocFragment[2]/body/p[1]/text().0", "percentage": 0.42, "device": "Kobo Libra", "device_id": "DEV1" });
    let (s, v) = json_of(&lib, ko("PUT", "/koreader/syncs/progress", "kosync pass", Some(place))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["document"], doc.as_str());
    assert!(v["timestamp"].as_i64().unwrap() > 0);

    // Folio's own progress has it, in the second chapter
    let (s, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.ebook), None)).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["fraction"], 0.42);
    assert_eq!(p["device"], "koreader:Kobo Libra");
    assert_eq!(p["locator"]["href"], "OEBPS/two.xhtml");
    assert_eq!(p["locator"]["total_progression"], 0.42);

    // KOReader gets its own place back exactly
    let (_, v) = json_of(&lib, ko("GET", &format!("/koreader/syncs/progress/{doc}"), "kosync pass", None)).await;
    assert_eq!(v["progress"], "/body/DocFragment[2]/body/p[1]/text().0");
    assert_eq!((v["percentage"].as_f64(), v["device_id"].as_str()), (Some(0.42), Some("DEV1")));

    // the web reader moves on to the third chapter: KOReader is sent to its start
    let later = server::compat::now() + 60.0;
    let web = json!({ "locator": { "href": "OEBPS/three.xhtml", "progression": 0.5, "total_progression": 0.8 }, "fraction": 0.8, "device": "web", "updated_at": later });
    let (s, p) = common::call(&lib.app, common::admin("PUT", &format!("/api/progress/{}", lib.ebook), Some(web))).await;
    assert_eq!(s, StatusCode::OK, "{p}");
    let (_, v) = json_of(&lib, ko("GET", &format!("/koreader/syncs/progress/{doc}"), "kosync pass", None)).await;
    assert_eq!(v["progress"], "/body/DocFragment[3]/body");
    assert_eq!((v["percentage"].as_f64(), v["device"].as_str()), (Some(0.8), Some("web")));
    assert_ne!(v["device_id"], "DEV1");
}

#[tokio::test]
async fn books_outside_the_library_still_sync_between_koreaders() {
    let lib = library("ko-other", Default::default()).await;
    json_of(&lib, common::admin("PUT", "/api/apps/koreader", Some(json!({ "password": "kosync pass" })))).await;
    let doc = md5(b"some other book.epub");
    let place = json!({ "document": doc, "progress": "17", "percentage": 0.1, "device": "Phone", "device_id": "P" });
    let (s, _) = json_of(&lib, ko("PUT", "/koreader/syncs/progress", "kosync pass", Some(place))).await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = json_of(&lib, ko("GET", &format!("/koreader/syncs/progress/{doc}"), "kosync pass", None)).await;
    assert_eq!((v["progress"].as_str(), v["device"].as_str()), (Some("17"), Some("Phone")));
    // a book named by its file name is found too
    let by_name = md5(b"The Martian.epub");
    let place =
        json!({ "document": by_name, "progress": "/body/DocFragment[1]/body", "percentage": 0.05, "device": "Phone", "device_id": "P" });
    json_of(&lib, ko("PUT", "/koreader/syncs/progress", "kosync pass", Some(place))).await;
    let (_, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.ebook), None)).await;
    assert_eq!(p["fraction"], 0.05);
    // nonsense is refused in plain words
    let bad = json!({ "document": "not a document", "percentage": 0.1 });
    let (s, v) = json_of(&lib, ko("PUT", "/koreader/syncs/progress", "kosync pass", Some(bad))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(v["detail"].as_str().unwrap().contains("which book"));
}
