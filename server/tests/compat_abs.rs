//! The Audiobookshelf API as its apps use it: signing in, the libraries and items, playing with a session,
//! places kept as Folio's progress both ways, offline sessions, bookmarks and files.

mod compat_support;

use axum::http::StatusCode;
use compat_support::{common, library, req, text, PASSWORD, USER};
use serde_json::{json, Value};

/// The JSON of an answer.
async fn call(lib: &compat_support::Library, r: axum::http::Request<axum::body::Body>) -> (StatusCode, Value) {
    let (s, _, b) = text(&lib.app, r).await;
    (s, serde_json::from_str(&b).unwrap_or(Value::Null))
}

/// Signs in the way the apps do; the token.
async fn sign_in(lib: &compat_support::Library, prefix: &str) -> String {
    let (s, v) = call(lib, req("POST", &format!("{prefix}/login"), &[], Some(json!({ "username": USER, "password": PASSWORD })))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["user"]["type"], "root");
    assert_eq!(v["userDefaultLibraryId"], "audiobooks");
    v["user"]["token"].as_str().unwrap().to_string()
}

/// A request with the app's token.
fn abs(method: &str, path: &str, token: &str, body: Option<Value>) -> axum::http::Request<axum::body::Body> {
    req(method, path, &[("authorization", &format!("Bearer {token}"))], body)
}

#[tokio::test]
async fn an_app_finds_the_server_and_signs_in() {
    let lib = library("abs-login", Default::default()).await;
    let (_, v) = call(&lib, req("GET", "/ping", &[], None)).await;
    assert_eq!(v["success"], true);
    let (_, v) = call(&lib, req("GET", "/abs/status", &[], None)).await;
    assert_eq!((v["isInit"].as_bool(), v["authMethods"][0].as_str()), (Some(true), Some("local")));
    let (s, v) = call(&lib, req("POST", "/login", &[], Some(json!({ "username": USER, "password": "nope" })))).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(v["detail"].as_str().unwrap().contains("wrong"));
    let (s, _) = call(&lib, req("GET", "/api/libraries", &[], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    let token = sign_in(&lib, "/abs").await;
    let (s, v) = call(&lib, abs("POST", "/api/authorize", &token, None)).await;
    assert_eq!((s, v["user"]["username"].as_str()), (StatusCode::OK, Some(USER)), "{v}");
    let (_, v) = call(&lib, abs("GET", "/abs/api/libraries", &token, None)).await;
    let names: Vec<&str> = v["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Audiobooks", "Ebooks"]);
    // the token is Folio's own app token
    let (s, _) = common::call(&lib.app, abs("GET", "/api/auth/me", &token, None)).await;
    assert_eq!(s, StatusCode::OK);
    let (_, _) = call(&lib, abs("POST", "/logout", &token, None)).await;
    let (s, _) = call(&lib, abs("GET", "/api/me", &token, None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn libraries_list_items_shelves_series_authors_and_search() {
    let lib = library("abs-browse", Default::default()).await;
    let token = sign_in(&lib, "").await;
    let (_, v) =
        call(&lib, abs("GET", "/api/libraries/audiobooks/items?limit=10&page=0&sort=media.metadata.title&minified=1", &token, None)).await;
    assert_eq!(v["total"], 1, "{v}");
    let it = &v["results"][0];
    assert_eq!(it["id"], format!("li_{}", lib.audio));
    assert_eq!(it["media"]["metadata"]["authorName"], "Andy Weir");
    assert_eq!(it["media"]["metadata"]["narratorName"], "Ray Porter");
    assert_eq!((it["media"]["duration"].as_f64(), it["media"]["numTracks"].as_u64()), (Some(1000.0), Some(2)));
    let (_, v) = call(&lib, abs("GET", "/api/libraries/ebooks/items", &token, None)).await;
    assert_eq!(v["results"][0]["media"]["ebookFormat"], "epub", "{v}");
    let (s, v) = call(&lib, abs("GET", "/api/libraries/podcasts/items", &token, None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{v}");

    let (_, v) = call(&lib, abs("GET", "/api/libraries/ebooks?include=filterdata", &token, None)).await;
    assert_eq!(v["filterdata"]["series"][0]["name"], "Mars", "{v}");
    let (_, v) = call(&lib, abs("GET", "/api/libraries/ebooks/series", &token, None)).await;
    assert_eq!((v["results"][0]["name"].as_str(), v["results"][0]["books"][0]["sequence"].as_str()), (Some("Mars"), Some("1")));
    let (_, v) = call(&lib, abs("GET", "/api/libraries/audiobooks/authors", &token, None)).await;
    assert_eq!((v["authors"][0]["name"].as_str(), v["authors"][0]["numBooks"].as_u64()), (Some("Andy Weir"), Some(1)));
    let author = v["authors"][0]["id"].as_str().unwrap().to_string();
    let (_, v) = call(&lib, abs("GET", &format!("/api/authors/{author}?include=items"), &token, None)).await;
    assert_eq!(v["libraryItems"].as_array().unwrap().len(), 2);
    // filters are base64 of the id
    use base64::Engine;
    let f = base64::engine::general_purpose::STANDARD.encode(&author);
    let (_, v) = call(&lib, abs("GET", &format!("/api/libraries/audiobooks/items?filter=authors.{f}"), &token, None)).await;
    assert_eq!(v["total"], 1);
    let (_, v) = call(&lib, abs("GET", "/api/libraries/audiobooks/search?q=hail", &token, None)).await;
    assert_eq!(v["book"][0]["libraryItem"]["media"]["metadata"]["title"], "Project Hail Mary", "{v}");
    let (_, v) = call(&lib, abs("GET", "/api/libraries/audiobooks/personalized", &token, None)).await;
    assert_eq!(v[0]["id"], "recently-added", "{v}");
}

#[tokio::test]
async fn playing_keeps_the_place_in_folio_and_folio_s_place_starts_the_player() {
    let lib = library("abs-play", Default::default()).await;
    let token = sign_in(&lib, "").await;
    let id = format!("li_{}", lib.audio);
    let (_, v) = call(&lib, abs("GET", &format!("/api/items/{id}?expanded=1&include=progress"), &token, None)).await;
    let tracks = v["media"]["tracks"].as_array().unwrap();
    assert_eq!((tracks[1]["startOffset"].as_f64(), tracks[1]["mimeType"].as_str()), (Some(600.0), Some("audio/mpeg")));
    assert_eq!(v["media"]["chapters"].as_array().unwrap().len(), 2, "each file's chapter, counted from the book's start");
    assert_eq!(v["media"]["chapters"][1]["start"], 600.0);

    // the web player left off 5 minutes in
    let web = json!({ "position_ms": 300_000, "fraction": 0.3, "device": "web", "updated_at": server::compat::now() - 60.0 });
    common::call(&lib.app, common::admin("PUT", &format!("/api/progress/{}", lib.audio), Some(web))).await;
    let (s, session) = call(
        &lib,
        abs(
            "POST",
            &format!("/api/items/{id}/play"),
            &token,
            Some(json!({ "deviceInfo": { "clientName": "Abs Android" }, "mediaPlayer": "exo-player" })),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{session}");
    assert_eq!(session["currentTime"], 300.0);
    let url = session["audioTracks"][0]["contentUrl"].as_str().unwrap().to_string();
    let sid = session["id"].as_str().unwrap().to_string();

    // the track's address works with the token on it, whole and in ranges, at the root and under /abs
    let (s, h, _) = text(&lib.app, req("GET", &format!("{url}?token={token}"), &[("range", "bytes=0-99")], None)).await;
    assert_eq!((s, h["content-length"].to_str().unwrap()), (StatusCode::PARTIAL_CONTENT, "100"));
    let (s, _, _) = text(&lib.app, req("GET", &format!("/abs{url}?token={token}"), &[], None)).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = text(&lib.app, req("GET", &url, &[], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);

    // syncing moves Folio's place
    let (s, _) =
        call(&lib, abs("POST", &format!("/api/session/{sid}/sync"), &token, Some(json!({ "currentTime": 700.0, "timeListened": 30.0 }))))
            .await;
    assert_eq!(s, StatusCode::OK);
    let (_, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.audio), None)).await;
    assert_eq!((p["position_ms"].as_i64(), p["device"].as_str()), (Some(700_000), Some("abs:Abs Android")), "{p}");
    assert_eq!(p["fraction"], 0.7);
    call(&lib, abs("POST", &format!("/api/session/{sid}/close"), &token, Some(json!({ "currentTime": 710.0, "timeListened": 10.0 }))))
        .await;
    let (_, v) = call(&lib, abs("GET", &format!("/api/me/progress/{id}"), &token, None)).await;
    assert_eq!((v["currentTime"].as_f64(), v["isFinished"].as_bool()), (Some(710.0), Some(false)), "{v}");
    let (_, v) = call(&lib, abs("GET", "/api/me/listening-stats", &token, None)).await;
    assert_eq!(v["totalTime"], 40.0);
    let (_, v) = call(&lib, abs("GET", "/api/me/items-in-progress", &token, None)).await;
    assert_eq!(v["libraryItems"][0]["id"], id.as_str());
    let (_, v) = call(&lib, abs("GET", "/api/libraries/audiobooks/personalized", &token, None)).await;
    assert_eq!(v[0]["id"], "continue-listening");

    // an offline session older than the place kept changes nothing; a newer one wins
    let old =
        json!({ "sessions": [{ "id": "local1", "libraryItemId": id, "currentTime": 100.0, "timeListening": 50.0, "updatedAt": 1000.0 }] });
    let (_, v) = call(&lib, abs("POST", "/api/session/local-all", &token, Some(old))).await;
    assert_eq!((v["results"][0]["success"].as_bool(), v["results"][0]["progressSynced"].as_bool()), (Some(true), Some(false)), "{v}");
    let new = json!({ "id": "local2", "libraryItemId": id, "currentTime": 999.5, "timeListening": 50.0, "updatedAt": (server::compat::now() + 5.0) * 1000.0 });
    call(&lib, abs("POST", "/api/session/local", &token, Some(new))).await;
    let (_, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.audio), None)).await;
    assert_eq!(p["position_ms"], 999_500, "{p}");

    // finished by hand
    let later = (server::compat::now() + 10.0) * 1000.0;
    let (_, v) =
        call(&lib, abs("PATCH", &format!("/api/me/progress/{id}"), &token, Some(json!({ "isFinished": true, "lastUpdate": later })))).await;
    assert_eq!((v["isFinished"].as_bool(), v["progress"].as_f64()), (Some(true), Some(1.0)), "{v}");
}

#[tokio::test]
async fn ebook_places_and_bookmarks() {
    let lib = library("abs-ebook", Default::default()).await;
    let token = sign_in(&lib, "").await;
    let id = format!("li_{}", lib.ebook);
    let (s, v) = call(&lib, abs("POST", &format!("/api/items/{id}/play"), &token, None)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("reader"));
    let (s, h, _) = text(&lib.app, abs("GET", &format!("/api/items/{id}/ebook"), &token, None)).await;
    assert_eq!((s, h["content-type"].to_str().unwrap()), (StatusCode::OK, "application/epub+zip"));
    let body = json!({ "ebookLocation": "epubcfi(/6/4!/4/2/1:0)", "ebookProgress": 0.25 });
    let (_, v) = call(&lib, abs("PATCH", &format!("/api/me/progress/{id}"), &token, Some(body))).await;
    assert_eq!((v["ebookLocation"].as_str(), v["ebookProgress"].as_f64()), (Some("epubcfi(/6/4!/4/2/1:0)"), Some(0.25)), "{v}");
    let (_, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.ebook), None)).await;
    assert_eq!((p["locator"]["cfi"].as_str(), p["fraction"].as_f64()), (Some("epubcfi(/6/4!/4/2/1:0)"), Some(0.25)), "{p}");

    let aid = format!("li_{}", lib.audio);
    let (s, v) =
        call(&lib, abs("POST", &format!("/api/me/item/{aid}/bookmark"), &token, Some(json!({ "time": 42.5, "title": "Rocky" })))).await;
    assert_eq!((s, v["time"].as_f64()), (StatusCode::OK, Some(42.5)), "{v}");
    let (_, v) =
        call(&lib, abs("PATCH", &format!("/api/me/item/{aid}/bookmark"), &token, Some(json!({ "time": 42.5, "title": "Rocky!" })))).await;
    assert_eq!(v["title"], "Rocky!");
    let (_, me) = call(&lib, abs("GET", "/api/me", &token, None)).await;
    assert_eq!(me["bookmarks"][0]["title"], "Rocky!", "{me}");
    assert_eq!(me["mediaProgress"][0]["libraryItemId"], id.as_str());
    call(&lib, abs("DELETE", &format!("/api/me/item/{aid}/bookmark/42.5"), &token, None)).await;
    let (_, me) = call(&lib, abs("GET", "/api/me", &token, None)).await;
    assert_eq!(me["bookmarks"], json!([]));
}
