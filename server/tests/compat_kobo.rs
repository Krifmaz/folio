//! Kobo sync as a Kobo e-reader does it: its address, the start, syncing books in, downloading, covers, and the
//! place kept in step with Folio's both ways.

mod compat_support;

use axum::http::StatusCode;
use compat_support::{common, library, req, text};
use serde_json::{json, Value};

/// The JSON of an answer, with its headers.
async fn call(lib: &compat_support::Library, r: axum::http::Request<axum::body::Body>) -> (StatusCode, axum::http::HeaderMap, Value) {
    let (s, h, b) = text(&lib.app, r).await;
    (s, h, serde_json::from_str(&b).unwrap_or(Value::Null))
}

/// The account's Kobo token.
async fn token(lib: &compat_support::Library) -> String {
    let (s, v) = common::call(&lib.app, common::admin("GET", "/kobo/token", None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["token"].as_str().unwrap().to_string()
}

/// A Kobo request.
fn kobo(method: &str, path: &str, headers: &[(&str, &str)], body: Option<Value>) -> axum::http::Request<axum::body::Body> {
    let mut h = vec![("host", "folio.test"), ("x-forwarded-proto", "https")];
    h.extend_from_slice(headers);
    req(method, path, &h, body)
}

#[tokio::test]
async fn a_kobo_starts_with_folio_s_addresses() {
    let lib = library("kobo-init", Default::default()).await;
    let t = token(&lib).await;
    assert!(t.chars().all(|c| c.is_ascii_alphanumeric()) && t.len() >= 20, "{t}");
    let (_, v) = common::call(&lib.app, common::admin("GET", "/kobo/token", None)).await;
    assert_eq!(v["token"], t.as_str(), "the address stays until a new one is asked for");
    let (s, h, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/initialization"), &[], None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-kobo-apitoken"], "e30=");
    let r = &v["Resources"];
    assert_eq!(r["library_sync"], format!("https://folio.test/kobo/{t}/v1/library/sync"));
    assert_eq!(r["image_url_template"], format!("https://folio.test/kobo/{t}/{{ImageId}}/{{Width}}/{{Height}}/false/image.jpg"));
    assert_eq!(r["store_host"], "store.kobobooks.com");
    let (s, _, v) = call(&lib, kobo("POST", &format!("/kobo/{t}/v1/auth/device"), &[], Some(json!({ "UserKey": "abc" })))).await;
    assert_eq!((s, v["UserKey"].as_str(), v["TokenType"].as_str()), (StatusCode::OK, Some("abc"), Some("Bearer")));
    let (s, _, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/tags"), &[], None)).await;
    assert_eq!((s, v), (StatusCode::OK, json!([])));
    // a wrong or old address says what to do
    let (s, _, v) = call(&lib, kobo("GET", "/kobo/wrongtoken/v1/initialization", &[], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(v["detail"].as_str().unwrap().contains("Other apps"));
    let (_, v) = common::call(&lib.app, common::admin("POST", "/kobo/token", None)).await;
    assert_ne!(v["token"], t.as_str());
    let (s, _, _) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/initialization"), &[], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn ebooks_sync_in_download_and_keep_their_place() {
    let lib = library("kobo-sync", Default::default()).await;
    let t = token(&lib).await;
    let (s, h, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[], None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let list = v.as_array().unwrap();
    assert_eq!(list.len(), 1, "only the ebook goes to the Kobo: {v}");
    let e = &list[0]["NewEntitlement"];
    let id = server::compat::kobo::uuid(lib.ebook);
    assert_eq!(e["BookEntitlement"]["Id"], id.as_str());
    let m = &e["BookMetadata"];
    assert_eq!((m["Title"].as_str(), m["Contributors"][0].as_str()), (Some("The Martian: A Novel"), Some("Andy Weir")));
    assert_eq!((m["Series"]["Name"].as_str(), m["Series"]["NumberFloat"].as_f64()), (Some("Mars"), Some(1.0)));
    assert_eq!(e["ReadingState"]["StatusInfo"]["Status"], "ReadyToRead");
    let url = m["DownloadUrls"][0]["Url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("https://folio.test/kobo/{t}/download/")), "{url}");
    let mark = h["x-kobo-synctoken"].to_str().unwrap().to_string();
    assert!(h.get("x-kobo-sync").is_none());

    // nothing new since
    let (_, _, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[("x-kobo-synctoken", &mark)], None)).await;
    assert_eq!(v, json!([]));

    // the download and the cover
    let path = url.strip_prefix("https://folio.test").unwrap();
    let (s, h, _) = text(&lib.app, kobo("GET", path, &[], None)).await;
    assert_eq!((s, h["content-type"].to_str().unwrap()), (StatusCode::OK, "application/epub+zip"));
    let (s, _, _) = text(&lib.app, kobo("GET", &format!("/kobo/{t}/{id}/200/300/false/image.jpg"), &[], None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "the ebook has no cover");

    // the Kobo's place becomes Folio's
    let state = json!({ "ReadingStates": [{
        "EntitlementId": id,
        "LastModified": "2030-01-01T00:00:00Z",
        "CurrentBookmark": { "ProgressPercent": 35, "ContentSourceProgressPercent": 50, "Location": { "Source": "OEBPS/two.xhtml", "Type": "KoboSpan", "Value": "kobo.4.1" } },
        "StatusInfo": { "Status": "Reading" }
    }]});
    let (s, _, v) = call(&lib, kobo("PUT", &format!("/kobo/{t}/v1/library/{id}/state"), &[], Some(state))).await;
    assert_eq!((s, v["RequestResult"].as_str()), (StatusCode::OK, Some("Success")), "{v}");
    let (_, p) = common::call(&lib.app, common::admin("GET", &format!("/api/progress/{}", lib.ebook), None)).await;
    assert_eq!(
        (p["fraction"].as_f64(), p["locator"]["href"].as_str(), p["device"].as_str()),
        (Some(0.35), Some("OEBPS/two.xhtml"), Some("kobo")),
        "{p}"
    );
    let (_, _, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/{id}/state"), &[], None)).await;
    assert_eq!(v[0]["CurrentBookmark"]["ProgressPercent"], 35.0);
    assert_eq!(v[0]["CurrentBookmark"]["Location"]["Value"], "kobo.4.1");
    assert_eq!(v[0]["StatusInfo"]["Status"], "Reading");

    // a place from another app comes to the Kobo with the next sync
    let web = json!({ "locator": { "href": "OEBPS/three.xhtml", "progression": 0.2, "total_progression": 0.9 }, "fraction": 0.9, "device": "web", "updated_at": 1_900_000_000.0 });
    common::call(&lib.app, common::admin("PUT", &format!("/api/progress/{}", lib.ebook), Some(web))).await;
    let (_, _, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[("x-kobo-synctoken", &mark)], None)).await;
    let rs = &v[0]["ChangedReadingState"]["ReadingState"];
    assert_eq!(
        (rs["CurrentBookmark"]["ProgressPercent"].as_f64(), rs["CurrentBookmark"]["Location"]["Source"].as_str()),
        (Some(90.0), Some("OEBPS/three.xhtml")),
        "{v}"
    );

    // removed on the Kobo: not synced again
    let (s, _, _) = call(&lib, kobo("DELETE", &format!("/kobo/{t}/v1/library/{id}"), &[], None)).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, _, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[], None)).await;
    assert_eq!(v, json!([]));
}

#[tokio::test]
async fn big_libraries_sync_a_hundred_at_a_time() {
    let lib = library("kobo-many", Default::default()).await;
    let epub = compat_support::epub_bytes();
    for n in 0..120 {
        let mut m = compat_support::ebook_meta();
        m.isbn13 = format!("978000000{n:04}");
        m.title = format!("Book {n}");
        let ed = server::library::store::save(&lib.st.db, &m).unwrap().edition_id;
        let rel = format!("many/{n}.epub");
        compat_support::add_file(
            &lib.st,
            &lib.dir,
            ed,
            compat_support::NewFile { rel: &rel, kind: "epub", mime: "application/epub+zip", bytes: &epub, position: 0, duration_ms: 0 },
        );
    }
    let t = token(&lib).await;
    let (_, h, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[], None)).await;
    assert_eq!(v.as_array().unwrap().len(), 100);
    assert_eq!(h["x-kobo-sync"], "continue");
    let mark = h["x-kobo-synctoken"].to_str().unwrap().to_string();
    let (_, h, v) = call(&lib, kobo("GET", &format!("/kobo/{t}/v1/library/sync"), &[("x-kobo-synctoken", &mark)], None)).await;
    assert_eq!(v.as_array().unwrap().len(), 21);
    assert!(h.get("x-kobo-sync").is_none());
}
