//! The wanted list through its HTTP API: wanting editions under the preferences, changing the preferences, and
//! followed authors and series bringing their new books.

mod common;

use axum::http::StatusCode;
use common::{admin, call, server, sources};
use serde_json::{json, Value};

/// Adds the edition with this ASIN or ISBN; its edition id.
async fn add(app: &axum::Router, id: Value) -> i64 {
    let (s, v) = call(app, admin("POST", "/api/library/editions", Some(id))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["edition_id"].as_i64().unwrap()
}

#[tokio::test]
async fn editions_are_wanted_under_their_formats_preferences() {
    let (app, _, _) = server("want-basic", sources().await).await;
    let audio = add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    let abridged = add(&app, json!({ "asin": "B0ABRIDGE1" })).await;
    let ebook = add(&app, json!({ "isbn": "9780593135204" })).await;
    let (s, w) = call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": audio })))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    assert_eq!((w["profile"].as_str(), w["status"].as_str()), (Some("Audiobooks"), Some("wanted")));
    let (s, v) = call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": abridged })))).await;
    assert_eq!(
        (s, v["detail"].as_str()),
        (StatusCode::BAD_REQUEST, Some("This audiobook is abridged, and the Audiobooks preferences want unabridged ones only."))
    );
    call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": ebook })))).await;
    let (_, list) = call(&app, admin("GET", "/api/wanted?format=ebook", None)).await;
    assert_eq!(list["wanted"].as_array().map(Vec::len), Some(1));
    assert_eq!(list["wanted"][0]["edition"]["isbn13"], "9780593135204");
    let (_, all) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!(all["wanted"].as_array().map(Vec::len), Some(2));
    // an ebook profile can't take an audiobook
    let (_, profiles) = call(&app, admin("GET", "/api/profiles", None)).await;
    let ebook_profile = profiles["profiles"].as_array().unwrap().iter().find(|p| p["format"] == "ebook").unwrap()["id"].clone();
    let (s, _) = call(&app, admin("PATCH", &format!("/api/wanted/{}", w["id"]), Some(json!({ "profile_id": ebook_profile })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&app, admin("DELETE", &format!("/api/wanted/{}", w["id"]), None)).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn preferences_change_what_is_taken() {
    let (app, _, _) = server("want-profiles", sources().await).await;
    let abridged = add(&app, json!({ "asin": "B0ABRIDGE1" })).await;
    let (_, profiles) = call(&app, admin("GET", "/api/profiles", None)).await;
    assert_eq!(profiles["file_formats"]["ebook"], json!(["epub", "azw3", "mobi", "pdf"]));
    let mut audio = profiles["profiles"].as_array().unwrap().iter().find(|p| p["format"] == "audiobook").unwrap().clone();
    assert_eq!(audio["file_formats"], json!(["m4b", "mp3"]));
    audio["unabridged_only"] = json!(false);
    let (s, v) = call(&app, admin("PUT", &format!("/api/profiles/{}", audio["id"]), Some(audio.clone()))).await;
    assert_eq!((s, v["unabridged_only"].as_bool()), (StatusCode::OK, Some(false)));
    let (s, _) = call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": abridged })))).await;
    assert_eq!(s, StatusCode::OK);
    audio["file_formats"] = json!(["wav"]);
    let (s, v) = call(&app, admin("PUT", &format!("/api/profiles/{}", audio["id"]), Some(audio))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
}

#[tokio::test]
async fn a_followed_author_brings_new_audiobooks_and_ebooks_once() {
    let (app, _, _) = server("want-follow", sources().await).await;
    add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    add(&app, json!({ "isbn": "9780593135204" })).await;
    let (_, authors) = call(&app, admin("GET", "/api/library/authors", None)).await;
    let id = authors["authors"][0]["id"].as_i64().unwrap();
    let (s, _) = call(&app, admin("PUT", &format!("/api/library/authors/{id}/follow"), Some(json!({ "follow": "both" })))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, r) = call(&app, admin("POST", "/api/follow/check", None)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    // the new English unabridged audiobook and The Martian's English ebook; not the abridged or German ones,
    // not the books already here
    assert_eq!(r["added"], json!(["The Next One (audiobook)", "The Martian (ebook)"]), "{r}");
    assert_eq!(r["problems"], json!([]));
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    let wanted: Vec<&Value> = w["wanted"].as_array().unwrap().iter().collect();
    assert_eq!(wanted.len(), 2);
    assert!(wanted.iter().all(|x| x["source"] == "follow"));
    let martian = wanted.iter().find(|x| x["edition"]["format"] == "ebook").unwrap();
    assert_eq!(martian["edition"]["isbn13"], "9780804139021", "the ebook edition, in English");
    // a second check adds nothing, even after a wanted book is taken off the list
    call(&app, admin("DELETE", &format!("/api/wanted/{}", martian["id"]), None)).await;
    let (_, r) = call(&app, admin("POST", "/api/follow/check", None)).await;
    assert_eq!((r["added"].clone(), r["looked_at"].clone()), (json!([]), json!(0)), "{r}");
}

#[tokio::test]
async fn a_followed_series_brings_its_other_audiobooks() {
    let (app, _, _) = server("want-series", sources().await).await;
    add(&app, json!({ "asin": "B003ZWFO7E" })).await;
    let (_, series) = call(&app, admin("GET", "/api/library/series", None)).await;
    let id = series["series"][0]["id"].as_i64().unwrap();
    let (s, _) = call(&app, admin("PUT", &format!("/api/library/series/{id}/follow"), Some(json!({ "follow": "sideways" })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    call(&app, admin("PUT", &format!("/api/library/series/{id}/follow"), Some(json!({ "follow": "audiobook" })))).await;
    let (_, r) = call(&app, admin("POST", "/api/follow/check", None)).await;
    assert_eq!(r["added"], json!(["Words of Radiance (audiobook)"]), "{r}");
    let (_, series) = call(&app, admin("GET", "/api/library/series", None)).await;
    assert_eq!(series["series"][0]["follow"], "audiobook");
    let titles: Vec<&str> = series["series"][0]["works"].as_array().unwrap().iter().map(|w| w["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["The Way of Kings", "Words of Radiance"]);
}

#[tokio::test]
async fn following_without_a_source_key_says_what_to_do() {
    let (app, _, _) = server("want-nokey", sources().await).await;
    add(&app, json!({ "asin": "B08G9PRS1K" })).await;
    let (_, authors) = call(&app, admin("GET", "/api/library/authors", None)).await;
    let id = authors["authors"][0]["id"].as_i64().unwrap();
    call(&app, admin("PUT", &format!("/api/library/authors/{id}/follow"), Some(json!({ "follow": "ebook" })))).await;
    let (_, r) = call(&app, admin("POST", "/api/follow/check", None)).await;
    assert_eq!(r["added"], json!([]));
    assert!(r["problems"][0].as_str().unwrap().contains("Open Library doesn't know this author yet"), "{r}");
}
