//! Downloads through (a stand-in) Lantern: searching for a wanted edition, scoring and grabbing the best
//! result, following the grab until its files arrive, and Lantern's callback.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{admin, call, lantern, server, sources};
use serde_json::{json, Value};

/// A server with the stand-in sources and Lantern, and the wanted entry of this ASIN or ISBN.
async fn wanted(name: &str, id: Value, with_lantern: bool) -> (axum::Router, server::AppState, i64) {
    let mut env = sources().await;
    if with_lantern {
        env.extend(lantern().await);
    }
    let (app, st, _) = server(name, env).await;
    let (_, e) = call(&app, admin("POST", "/api/library/editions", Some(id))).await;
    let (s, w) = call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": e["edition_id"] })))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    (app, st, w["id"].as_i64().unwrap())
}

#[tokio::test]
async fn without_lantern_set_up_it_says_where_to_add_it() {
    let (app, _, id) = wanted("lt-none", json!({ "asin": "B08G9PRS1K" }), false).await;
    let (s, v) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(
        (s, v["detail"].as_str()),
        (StatusCode::BAD_REQUEST, Some("Lantern isn't set up yet. Add its address and API key under Settings → Downloads."))
    );
}

#[tokio::test]
async fn the_best_audiobook_is_grabbed_and_followed_until_it_arrives() {
    let (app, st, id) = wanted("lt-audio", json!({ "asin": "B08G9PRS1K" }), true).await;
    let (s, r) = call(&app, admin("GET", &format!("/api/wanted/{id}/releases"), None)).await;
    assert_eq!(s, StatusCode::OK, "{r}");
    let results = r["results"].as_array().unwrap();
    assert_eq!(results[0]["release"]["guid"], "m4b", "the M4B with ASIN and narrator first");
    let refused: Vec<&str> = results.iter().filter(|x| !x["refused"].is_null()).map(|x| x["release"]["guid"].as_str().unwrap()).collect();
    assert_eq!(refused, vec!["sample", "abridged"]);
    let (s, v) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["grab"]["lantern_id"].as_str(), v["grab"]["file_format"].as_str()), (Some("g1"), Some("m4b")));
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!(w["wanted"][0]["status"], "downloading");
    // searching again while it downloads is refused
    let (s, _) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(server::lantern::grabs::poll(&st).await.unwrap(), 1);
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!(w["wanted"][0]["status"], "downloaded");
    assert!(w["wanted"][0]["note"].as_str().unwrap().contains("arrived: 1 files in /downloads/folio/g1"), "{w}");
    let (_, g) = call(&app, admin("GET", "/api/grabs", None)).await;
    assert_eq!(g["grabs"][0]["files"][0]["path"], "/downloads/folio/g1/book.m4b");
}

#[tokio::test]
async fn an_ebook_takes_the_epub_and_never_the_mobi() {
    let (app, _, id) = wanted("lt-ebook", json!({ "isbn": "9780593135204" }), true).await;
    let (_, v) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(v["grab"]["file_format"], "epub", "{v}");
}

#[tokio::test]
async fn a_refused_result_is_grabbed_only_when_forced() {
    let (app, _, id) = wanted("lt-force", json!({ "asin": "B08G9PRS1K" }), true).await;
    let (s, v) = call(&app, admin("POST", &format!("/api/wanted/{id}/grab"), Some(json!({ "guid": "sample", "indexer_id": 1 })))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert_eq!(v["detail"], "Folio won't take this result: Its name says \"sample\": it isn't the whole book.");
    let (s, v) =
        call(&app, admin("POST", &format!("/api/wanted/{id}/grab"), Some(json!({ "guid": "sample", "indexer_id": 1, "force": true }))))
            .await;
    assert_eq!((s, v["status"].as_str()), (StatusCode::OK, Some("queued")), "{v}");
    let (s, _) = call(&app, admin("POST", &format!("/api/wanted/{id}/grab"), Some(json!({ "guid": "gone" })))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn nothing_good_leaves_it_wanted_with_the_reason() {
    let (app, _, id) = wanted("lt-nothing", json!({ "asin": "B003ZWFO7E" }), true).await;
    let (s, v) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["grab"], Value::Null);
    assert_eq!(v["note"], "Lantern found nothing for this edition yet.");
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!(
        (w["wanted"][0]["status"].as_str(), w["wanted"][0]["note"].as_str()),
        (Some("wanted"), Some("Lantern found nothing for this edition yet."))
    );
}

#[tokio::test]
async fn lanterns_callback_needs_the_secret_address() {
    let (app, st, id) = wanted("lt-callback", json!({ "asin": "B08G9PRS1K" }), true).await;
    call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    let post = |path: String, body: Value| {
        Request::post(path).header("content-type", "application/json").body(Body::from(body.to_string())).unwrap()
    };
    let done = json!({ "id": "g1", "status": "failed", "error": "The tracker removed the torrent." });
    let (s, _) = call(&app, post("/api/lantern/callback/wrong".into(), done.clone())).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let token = server::lantern::grabs::callback_token(&st.db).unwrap();
    let (s, v) = call(&app, post(format!("/api/lantern/callback/{token}"), done)).await;
    assert_eq!((s, v["known"].as_bool()), (StatusCode::OK, Some(true)));
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!(w["wanted"][0]["status"], "failed");
    assert!(w["wanted"][0]["note"].as_str().unwrap().ends_with("The tracker removed the torrent."));
}

#[tokio::test]
async fn the_test_button_reaches_lantern_and_a_wrong_key_is_named() {
    let mut env = sources().await;
    env.extend(lantern().await);
    env.insert("APP_LANTERN_API_KEY".into(), "wrong".into());
    let (app, _, _) = server("lt-test", env).await;
    let (s, v) = call(&app, admin("POST", "/api/lantern/test", None)).await;
    assert_eq!(s, StatusCode::BAD_GATEWAY);
    assert_eq!(v["detail"], "Lantern refused Folio's API key. Copy the key from Lantern's settings into Settings → Downloads.");
}
