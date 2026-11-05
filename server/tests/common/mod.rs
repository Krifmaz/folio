//! What the app's tests share: a fresh server on a data folder of its own, requests as the owner, and a stand-in
//! for the outside sources (Audnexus, Open Library, Google Books, Hardcover) serving the fixtures.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tower::ServiceExt;

/// The password the tests' owner signs in with.
pub const PASSWORD: &str = "correct horse battery";

/// A fresh server on a data folder of its own, with an owner `admin`; `env` are the settings' environment
/// variables (`APP_API_KEY` is always `test-key`).
pub async fn server(name: &str, env: HashMap<String, String>) -> (Router, server::AppState, PathBuf) {
    let dir = std::env::temp_dir().join(format!("folio-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let web = dir.join("static");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "<!doctype html><title>app</title>").unwrap();
    let st = server::AppState::open(
        &dir.join("data"),
        web,
        move |k| if k == "APP_API_KEY" { Some("test-key".into()) } else { env.get(k).cloned() },
    )
    .await
    .unwrap();
    server::auth::ensure_owner(&st, None, Some(PASSWORD.into())).unwrap();
    (server::app(st.clone()), st, dir)
}

/// One request; the status and the body as JSON (`Null` when it is not JSON).
pub async fn call(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let b = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    (status, serde_json::from_slice(&b).unwrap_or(Value::Null))
}

/// A request as the owner, through the API key.
pub fn admin(method: &str, path: &str, body: Option<Value>) -> Request<Body> {
    let b = Request::builder().method(method).uri(path).header("x-api-key", "test-key");
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

/// A fixture file of tests/fixtures as JSON.
pub fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Serves `router` on a free port of this machine; its address (`http://127.0.0.1:<port>`).
pub async fn serve(router: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, router).await.unwrap() });
    format!("http://{addr}")
}

/// The stand-in sources, serving the fixtures (audiobooks by Andy Weir and The Stormlight Archive, the ebooks
/// Project Hail Mary and The Martian); the
/// environment that points the settings at them (Hardcover with a token).
pub async fn sources() -> HashMap<String, String> {
    let found = |name: &'static str| move || async move { Json(fixture(name)) };
    let missing = || async { (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "not found" }))) };
    let book = |axum::extract::Path(asin): axum::extract::Path<String>| async move {
        match asin.as_str() {
            "B08G9PRS1K" => Ok(Json(fixture("audnexus_book.json"))),
            "B003ZWFO7E" => Ok(Json(fixture("audnexus_series_book.json"))),
            "B0NEWBOOK1" => Ok(Json(fixture("audnexus_next.json"))),
            "B0SECOND01" => Ok(Json(fixture("audnexus_second.json"))),
            "B0ABRIDGE1" => Ok(Json(fixture("audnexus_abridged.json"))),
            _ => Err(StatusCode::NOT_FOUND),
        }
    };
    let chapters = |axum::extract::Path(asin): axum::extract::Path<String>| async move {
        if asin == "B08G9PRS1K" {
            Ok(Json(fixture("audnexus_chapters.json")))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    };
    let isbn = |axum::extract::Path(file): axum::extract::Path<String>| async move {
        if file == "9780593135204.json" {
            Ok(Json(fixture("ol_isbn.json")))
        } else if file == "9780804139021.json" {
            Ok(Json(fixture("ol_isbn_martian.json")))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    };
    let volumes = |axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
        if q.get("q").map(String::as_str) == Some("isbn:9780593135204") {
            Json(fixture("gb_volumes.json"))
        } else {
            Json(serde_json::json!({ "kind": "books#volumes", "totalItems": 0 }))
        }
    };
    let hardcover = |headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
        if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer hc-token") {
            return Json(serde_json::json!({ "errors": [{ "message": "Unable to verify token" }] }));
        }
        if body["variables"]["isbn13"] == "9780593135204" {
            Json(fixture("hardcover_edition.json"))
        } else {
            Json(serde_json::json!({ "data": { "editions": [] } }))
        }
    };
    let catalogue = |axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>| async move {
        if q.get("author").map(String::as_str) == Some("Andy Weir") {
            Json(fixture("audible_author.json"))
        } else {
            Json(serde_json::json!({ "products": [], "total_results": 0 }))
        }
    };
    let product = |axum::extract::Path(asin): axum::extract::Path<String>| async move {
        if asin == "B00DA6YEKS" {
            Ok(Json(fixture("audible_series.json")))
        } else {
            Err(StatusCode::NOT_FOUND)
        }
    };
    let editions = |axum::extract::Path(key): axum::extract::Path<String>| async move {
        match key.as_str() {
            "OL17091839W" => Ok(Json(fixture("ol_work_editions.json"))),
            _ => Err(StatusCode::NOT_FOUND),
        }
    };
    let r = Router::new()
        .route("/audible/1.0/catalog/products", get(catalogue))
        .route("/audible/1.0/catalog/products/{asin}", get(product))
        .route("/ol/authors/OL7234434A/works.json", get(found("ol_author_works.json")))
        .route("/ol/works/{key}/editions.json", get(editions))
        .route("/audnexus/books/{asin}", get(book))
        .route("/audnexus/books/{asin}/chapters", get(chapters))
        .route("/ol/isbn/{file}", get(isbn))
        .route("/ol/works/OL21745884W.json", get(found("ol_work.json")))
        .route("/ol/authors/OL7234434A.json", get(found("ol_author.json")))
        .route("/ol/works/{other}", get(missing))
        .route("/gb/volumes", get(volumes))
        .route("/hc", post(hardcover));
    let base = serve(r).await;
    HashMap::from([
        ("APP_AUDNEXUS_URL".to_string(), format!("{base}/audnexus")),
        ("APP_AUDIBLE_URL".to_string(), format!("{base}/audible")),
        ("APP_OPENLIBRARY_URL".to_string(), format!("{base}/ol")),
        ("APP_GOOGLEBOOKS_URL".to_string(), format!("{base}/gb")),
        ("APP_HARDCOVER_URL".to_string(), format!("{base}/hc")),
        ("APP_HARDCOVER_TOKEN".to_string(), "hc-token".to_string()),
    ])
}
