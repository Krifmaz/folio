//! The library through its HTTP API: editions looked up on the (stand-in) sources by ASIN and ISBN, kept as
//! works and editions, listed, refreshed and removed.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{admin, call, server, sources};
use serde_json::json;

#[tokio::test]
async fn an_audiobook_is_added_by_its_asin_with_narrator_runtime_and_chapters() {
    let (app, _, _) = server("lib-asin", sources().await).await;
    let (s, v) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "asin": "b08g9prs1k" })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["created"], true);
    let work = &v["work"];
    assert_eq!(work["title"], "Project Hail Mary");
    assert_eq!(work["authors"][0]["name"], "Andy Weir");
    let ed = &work["editions"][0];
    assert_eq!((ed["format"].as_str(), ed["asin"].as_str()), (Some("audiobook"), Some("B08G9PRS1K")));
    assert_eq!(ed["narrators"], json!(["Ray Porter"]));
    assert_eq!((ed["runtime_min"].as_u64(), ed["abridged"].as_bool(), ed["language"].as_str()), (Some(970), Some(false), Some("en")));
    assert_eq!(ed["chapters"].as_array().map(Vec::len), Some(3));
    // the same ASIN again is the same edition
    let (_, again) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "asin": "B08G9PRS1K" })))).await;
    assert_eq!((again["created"].as_bool(), again["edition_id"].clone()), (Some(false), v["edition_id"].clone()));
}

#[tokio::test]
async fn an_ebook_is_added_by_isbn_from_three_sources_and_joins_the_audiobooks_work() {
    let (app, _, _) = server("lib-isbn", sources().await).await;
    let (_, a) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "asin": "B08G9PRS1K" })))).await;
    let (s, v) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "isbn": "0-593-13520-2" })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["work"]["id"], a["work"]["id"], "one work, two editions");
    assert_eq!(v["work"]["formats"], json!(["audiobook", "ebook"]));
    let ebook = v["work"]["editions"].as_array().unwrap().iter().find(|e| e["format"] == "ebook").unwrap().clone();
    assert_eq!(ebook["isbn13"], "9780593135204");
    assert_eq!(ebook["isbn10"], "0593135202");
    assert_eq!(ebook["pages"], 496, "Open Library first");
    assert_eq!(ebook["language"], "en");
    assert_eq!(ebook["sources"], json!(["openlibrary", "hardcover", "googlebooks"]));
    assert_eq!(ebook["ol_work"], "OL21745884W");
    let (_, authors) = call(&app, admin("GET", "/api/library/authors", None)).await;
    assert_eq!(authors["authors"].as_array().map(Vec::len), Some(1));
    assert_eq!(authors["authors"][0]["ol_key"], "OL7234434A");
}

#[tokio::test]
async fn lookups_say_plainly_what_is_wrong() {
    let (app, _, _) = server("lib-errors", sources().await).await;
    let (s, v) = call(&app, admin("GET", "/api/library/lookup?isbn=9780593135205", None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("is not a valid ISBN"), "{v}");
    let (s, v) = call(&app, admin("GET", "/api/library/lookup?asin=B000000000", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(v["detail"].as_str().unwrap().contains("Audnexus doesn't know the ASIN B000000000"), "{v}");
    let (s, v) = call(&app, admin("GET", "/api/library/lookup?isbn=9780306406157", None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");
    let (s, _) = call(&app, admin("GET", "/api/library/lookup", None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // a lookup keeps nothing, and says when the edition is already kept
    let (s, v) = call(&app, admin("GET", "/api/library/lookup?asin=B08G9PRS1K", None)).await;
    assert_eq!((s, v["kept"].clone()), (StatusCode::OK, json!(null)));
    let (_, works) = call(&app, admin("GET", "/api/library/works", None)).await;
    assert_eq!(works["works"], json!([]));
}

#[tokio::test]
async fn a_wrong_hardcover_token_does_not_stop_the_other_sources() {
    let mut env = sources().await;
    env.insert("APP_HARDCOVER_TOKEN".into(), "wrong".into());
    let (app, _, _) = server("lib-hc", env).await;
    let (s, v) = call(&app, admin("GET", "/api/library/lookup?isbn=9780593135204", None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["edition"]["sources"], json!(["openlibrary", "googlebooks"]));
}

#[tokio::test]
async fn series_list_works_and_editions_are_removed() {
    let (app, _, _) = server("lib-series", sources().await).await;
    let (_, kings) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "asin": "B003ZWFO7E" })))).await;
    call(&app, admin("POST", "/api/library/editions", Some(json!({ "asin": "B08G9PRS1K" })))).await;
    let (_, s) = call(&app, admin("GET", "/api/library/series", None)).await;
    assert_eq!(s["series"][0]["name"], "The Stormlight Archive");
    assert_eq!(s["series"][0]["works"][0]["title"], "The Way of Kings");
    let (_, w) = call(&app, admin("GET", "/api/library/works?q=sanderson&format=audiobook", None)).await;
    assert_eq!(w["works"].as_array().map(Vec::len), Some(1));
    let (s, _) = call(&app, admin("GET", "/api/library/works?format=paper", None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let id = kings["edition_id"].as_i64().unwrap();
    let (s, v) = call(&app, admin("POST", &format!("/api/library/editions/{id}/refresh"), None)).await;
    assert_eq!((s, v["narrators"].clone()), (StatusCode::OK, json!(["Michael Kramer", "Kate Reading"])));
    let (s, _) = call(&app, admin("DELETE", &format!("/api/library/editions/{id}"), None)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, w) = call(&app, admin("GET", "/api/library/works", None)).await;
    assert_eq!(w["works"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn adding_needs_an_admin_and_reading_needs_a_sign_in() {
    let (app, _, _) = server("lib-auth", sources().await).await;
    let (s, _) = call(&app, Request::get("/api/library/works").body(Body::empty()).unwrap()).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _) = call(
        &app,
        Request::post("/api/library/editions")
            .header("content-type", "application/json")
            .body(Body::from("{\"asin\":\"B08G9PRS1K\"}"))
            .unwrap(),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}
