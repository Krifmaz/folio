//! Verification before import and the Review queue, on real files: audiobooks made with ffmpeg (a short stand-in
//! for the edition, whose runtime and chapters are set to match) and EPUBs written here.

mod common;

use axum::http::StatusCode;
use common::media::{arrived, epub, m4b, wanted, wanted_entry};
use common::{admin, call};
use kit_server_core::{p, Db, RowExt};
use serde_json::json;

#[tokio::test]
async fn the_right_audiobook_is_verified() {
    let (app, st, dir, id) = wanted("vf-audio-ok", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0, &[]).await;
    let f = m4b(&dir.join("dl"), "book", 120, 3, &[("album", "Project Hail Mary"), ("composer", "Ray Porter")], "eng");
    let g = arrived(&st, id, "m4b", &[&f]);
    assert_eq!(server::verify::pending(&st).await.unwrap(), 1);
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "verified", "{w}");
    assert!(w["note"].as_str().unwrap().starts_with("It is the edition you asked for"), "{w}");
    let r = st.db.one("SELECT status, confidence FROM grabs WHERE id=?", p![g]).unwrap().unwrap();
    assert_eq!((r.s("status").as_str(), r.i("confidence")), ("verified", 100));
    let (_, q) = call(&app, admin("GET", "/api/review", None)).await;
    assert_eq!(q["items"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn a_sample_waits_under_review_and_rejecting_it_searches_again() {
    let (app, st, dir, id) = wanted("vf-audio-sample", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0, &[]).await;
    let f = m4b(&dir.join("dl"), "sample", 20, 1, &[("title", "Project Hail Mary (Sample)")], "eng");
    let g = arrived(&st, id, "m4b", &[&f]);
    server::verify::pending(&st).await.unwrap();
    assert_eq!(wanted_entry(&app, id).await["status"], "review");
    let (s, q) = call(&app, admin("GET", "/api/review", None)).await;
    assert_eq!(s, StatusCode::OK);
    let item = &q["items"][0];
    assert_eq!(item["grab"]["id"], g);
    assert_eq!(item["wanted"]["id"], id);
    assert!(item["verdict"]["summary"].as_str().unwrap().starts_with("This isn't the edition you asked for"), "{item}");
    let runtime = item["verdict"]["checks"].as_array().unwrap().iter().find(|c| c["name"] == "Runtime").unwrap();
    assert_eq!(
        (runtime["requested"].as_str(), runtime["arrived"].as_str(), runtime["outcome"].as_str()),
        (Some("2 min"), Some("0 min"), Some("fail"))
    );
    // searching while it waits for a look is refused
    let (s, _) = call(&app, admin("POST", &format!("/api/wanted/{id}/search"), None)).await;
    assert_eq!(s, StatusCode::CONFLICT);
    // reject it and search again: the rejected result is never taken again (and the others don't fit a 2 min
    // book), so nothing is grabbed and the book is wanted again
    let (s, v) = call(&app, admin("POST", &format!("/api/review/{g}/research"), None)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["grab"].is_null(), "{v}");
    assert_eq!(wanted_entry(&app, id).await["status"], "wanted");
    let (_, r) = call(&app, admin("GET", &format!("/api/wanted/{id}/releases"), None)).await;
    let m4b = r["results"].as_array().unwrap().iter().find(|x| x["release"]["guid"] == "m4b").unwrap();
    assert_eq!(m4b["refused"], "You rejected this download for this book before.");
    // a second answer about the same download changes nothing
    let (s, _) = call(&app, admin("POST", &format!("/api/review/{g}/accept"), None)).await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[tokio::test]
async fn untagged_audio_waits_under_review_and_can_be_accepted() {
    let (app, st, dir, id) = wanted("vf-audio-untagged", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0, &[]).await;
    let f = m4b(&dir.join("dl"), "plain", 120, 0, &[], "");
    let g = arrived(&st, id, "m4b", &[&f]);
    server::verify::pending(&st).await.unwrap();
    let (_, q) = call(&app, admin("GET", "/api/review", None)).await;
    let summary = q["items"][0]["verdict"]["summary"].as_str().unwrap().to_string();
    assert!(summary.contains("% sure") && summary.contains("who reads it"), "{summary}");
    let (s, v) = call(&app, admin("POST", &format!("/api/review/{g}/accept"), None)).await;
    assert_eq!((s, v["grab"]["status"].as_str()), (StatusCode::OK, Some("verified")), "{v}");
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "verified");
    assert!(w["note"].as_str().unwrap().contains("accepted it"), "{w}");
}

#[tokio::test]
async fn the_right_epub_is_verified_and_a_locked_one_is_not() {
    let (app, st, dir, id) = wanted("vf-ebook", json!({ "isbn": "9780593135204" }), 0, 0, 40, &[]).await;
    let ok = epub(&dir.join("dl"), "ok", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, false);
    arrived(&st, id, "epub", &[&ok]);
    server::verify::pending(&st).await.unwrap();
    assert_eq!(wanted_entry(&app, id).await["status"], "verified");

    let (app, st, dir, id) = wanted("vf-ebook-drm", json!({ "isbn": "9780593135204" }), 0, 0, 40, &[]).await;
    let locked = epub(&dir.join("dl"), "locked", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, true);
    arrived(&st, id, "epub", &[&locked]);
    server::verify::pending(&st).await.unwrap();
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "review");
    assert!(w["note"].as_str().unwrap().contains("DRM"), "{w}");
}

#[tokio::test]
async fn another_book_or_a_short_epub_is_caught() {
    let (app, st, dir, id) = wanted("vf-ebook-wrong", json!({ "isbn": "9780593135204" }), 0, 0, 40, &[]).await;
    let f = epub(&dir.join("dl"), "other", ["9780553418026", "The Martian", "Andy Weir", "de"], 1_000, false);
    arrived(&st, id, "epub", &[&f]);
    server::verify::pending(&st).await.unwrap();
    let (_, q) = call(&app, admin("GET", "/api/review", None)).await;
    let checks = q["items"][0]["verdict"]["checks"].as_array().unwrap().clone();
    let outcome = |n: &str| checks.iter().find(|c| c["name"] == n).unwrap()["outcome"].as_str().unwrap().to_string();
    for n in ["ISBN", "Title", "Language", "Length"] {
        assert_eq!(outcome(n), "fail", "{n}");
    }
    assert_eq!(outcome("Author"), "pass");
    assert_eq!(wanted_entry(&app, id).await["status"], "review");
}

#[tokio::test]
async fn files_that_arent_the_format_go_to_review() {
    let (app, st, dir, id) = wanted("vf-nofiles", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0, &[]).await;
    std::fs::create_dir_all(dir.join("dl")).unwrap();
    let jpg = dir.join("dl/cover.jpg");
    std::fs::write(&jpg, b"x").unwrap();
    arrived(&st, id, "m4b", &[&jpg]);
    server::verify::pending(&st).await.unwrap();
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "review");
    assert_eq!(w["note"], "This isn't the edition you asked for. None of the 1 files that arrived is an audiobook Folio can read.");
}
