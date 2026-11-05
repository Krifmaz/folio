//! The OPDS catalogue as an e-reader sees it: signing in with a username and password, browsing, searching and
//! downloading.

mod compat_support;

use axum::http::StatusCode;
use compat_support::{basic, library, req, text, PASSWORD};

#[tokio::test]
async fn a_reader_must_sign_in_and_is_asked_how() {
    let lib = library("opds-auth", Default::default()).await;
    let (s, h, _) = text(&lib.app, req("GET", "/opds", &[], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h["www-authenticate"].to_str().unwrap().starts_with("Basic"));
    let (s, _, body) = text(&lib.app, req("GET", "/opds", &[("authorization", &basic("wrong password"))], None)).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(body.contains("username or password is wrong"), "{body}");
    let (s, h, body) = text(&lib.app, req("GET", "/opds", &[("authorization", &basic(PASSWORD))], None)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(h["content-type"].to_str().unwrap().contains("kind=navigation"));
    for href in ["/opds/new", "/opds/audiobooks", "/opds/ebooks", "/opds/authors", "/opds/series", "/opds/search.xml"] {
        assert!(body.contains(&format!("href=\"{href}\"")), "{href} missing: {body}");
    }
    // the API key works too
    let (s, _, _) = text(&lib.app, req("GET", "/opds", &[("x-api-key", "test-key")], None)).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn books_are_listed_with_their_facts_cover_and_files() {
    let lib = library("opds-books", Default::default()).await;
    let auth = basic(PASSWORD);
    let (s, h, body) = text(&lib.app, req("GET", "/opds/new", &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert!(h["content-type"].to_str().unwrap().contains("kind=acquisition"));
    assert!(body.contains("<title>The Martian: A Novel</title>"), "{body}");
    assert!(body.contains("<title>Project Hail Mary</title>"));
    assert!(body.contains("<dc:identifier>urn:isbn:9780553418026</dc:identifier>"));
    assert!(body.contains("<contributor><name>Ray Porter</name></contributor>"));
    assert!(body.contains("A lone astronaut &amp; a mission."), "the blurb is plain text: {body}");
    assert!(body.contains(&format!("href=\"/opds/books/{}/cover\" type=\"image/jpeg\"", lib.audio)));
    assert!(body.contains("type=\"application/epub+zip\" title=\"The Martian.epub\""));
    assert!(body.contains("<opensearch:totalResults>2</opensearch:totalResults>"));
    // the cover is not a download of its own
    assert_eq!(body.matches("rel=\"http://opds-spec.org/acquisition\"").count(), 3);

    let (_, _, body) = text(&lib.app, req("GET", "/opds/ebooks", &[("authorization", &auth)], None)).await;
    assert!(body.contains("The Martian") && !body.contains("Project Hail Mary"));
    let (_, _, body) = text(&lib.app, req("GET", "/opds/audiobooks", &[("authorization", &auth)], None)).await;
    assert!(!body.contains("The Martian") && body.contains("Project Hail Mary"));
}

#[tokio::test]
async fn authors_series_and_search_lead_to_the_books() {
    let lib = library("opds-browse", Default::default()).await;
    let auth = basic(PASSWORD);
    let (_, _, body) = text(&lib.app, req("GET", "/opds/authors", &[("authorization", &auth)], None)).await;
    assert!(
        body.contains("<title>Andy Weir</title>") && body.contains("2 books") && body.contains("href=\"/opds/authors/andyweir\""),
        "{body}"
    );
    let (s, _, body) = text(&lib.app, req("GET", "/opds/authors/andyweir", &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(body.contains("The Martian") && body.contains("Project Hail Mary"));
    let (_, _, body) = text(&lib.app, req("GET", "/opds/series/mars", &[("authorization", &auth)], None)).await;
    assert!(body.contains("The Martian") && !body.contains("Project Hail Mary"), "{body}");
    let (s, _, body) = text(&lib.app, req("GET", "/opds/series/nothing", &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND, "{body}");
    let (_, _, body) = text(&lib.app, req("GET", "/opds/search?q=porter%20hail", &[("authorization", &auth)], None)).await;
    assert!(body.contains("Project Hail Mary") && !body.contains("The Martian"), "{body}");
    let (_, h, body) = text(&lib.app, req("GET", "/opds/search.xml", &[("authorization", &auth)], None)).await;
    assert!(h["content-type"].to_str().unwrap().contains("opensearchdescription"));
    assert!(body.contains("/opds/search?q={searchTerms}"));
}

#[tokio::test]
async fn files_download_whole_or_in_ranges() {
    let lib = library("opds-files", Default::default()).await;
    let auth = basic(PASSWORD);
    let (_, _, feed) = text(&lib.app, req("GET", "/opds/ebooks", &[("authorization", &auth)], None)).await;
    let start = feed.find(&format!("/opds/books/{}/files/", lib.ebook)).unwrap();
    let href = &feed[start..feed[start..].find('"').unwrap() + start];
    let (s, h, _) = text(&lib.app, req("GET", href, &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "application/epub+zip");
    assert!(h["content-disposition"].to_str().unwrap().contains("The Martian.epub"));
    assert_eq!(h["content-length"], compat_support::epub_bytes().len().to_string().as_str());
    let (s, h, _) = text(&lib.app, req("GET", href, &[("authorization", &auth), ("range", "bytes=0-9")], None)).await;
    assert_eq!(s, StatusCode::PARTIAL_CONTENT);
    assert_eq!(h["content-length"], "10");
    let (s, h, _) = text(&lib.app, req("GET", &format!("/opds/books/{}/cover", lib.audio), &[("authorization", &auth)], None)).await;
    assert_eq!((s, h["content-type"].to_str().unwrap()), (StatusCode::OK, "image/jpeg"));
    // a file of another book is not served through this one
    let (s, _, _) = text(&lib.app, req("GET", &format!("/opds/books/{}/files/1", lib.ebook), &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // a file gone from the folder says so
    std::fs::remove_dir_all(lib.dir.join("Andy Weir/Mars")).unwrap();
    let (s, _, body) = text(&lib.app, req("GET", href, &[("authorization", &auth)], None)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(body.contains("missing from the library folder"), "{body}");
}
