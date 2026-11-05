//! Verification before import and the Review queue, on real files: audiobooks made with ffmpeg (a short stand-in
//! for the edition, whose runtime and chapters are set to match) and EPUBs written here.

mod common;

use axum::http::StatusCode;
use common::{admin, call, lantern, server, sources};
use kit_server_core::{p, Db, RowExt};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A server with the stand-in sources and Lantern, and a wanted entry for this ASIN or ISBN. The edition's
/// runtime (minutes), chapter count and pages are set as given, so short test files can be the whole book.
async fn wanted(name: &str, id: Value, runtime_min: u32, chapters: usize, pages: u32) -> (axum::Router, server::AppState, PathBuf, i64) {
    let mut env = sources().await;
    env.extend(lantern().await);
    let (app, st, dir) = server(name, env).await;
    let (_, e) = call(&app, admin("POST", "/api/library/editions", Some(id))).await;
    let edition = e["edition_id"].as_i64().unwrap();
    let ch: Vec<Value> =
        (0..chapters).map(|i| json!({ "title": format!("Chapter {}", i + 1), "start_ms": i * 40_000, "length_ms": 40_000 })).collect();
    st.db
        .execute(
            "UPDATE editions SET meta=json_set(meta, '$.runtime_min', ?, '$.chapters', json(?), '$.pages', ?) WHERE id=?",
            p![runtime_min, Value::Array(ch).to_string(), pages, edition],
        )
        .unwrap();
    let (s, w) = call(&app, admin("POST", "/api/wanted", Some(json!({ "edition_id": edition })))).await;
    assert_eq!(s, StatusCode::OK, "{w}");
    (app, st, dir, w["id"].as_i64().unwrap())
}

/// A finished grab of `files` for the wanted entry, as Lantern reports it (guid `m4b`, Lantern's best result).
fn arrived(st: &server::AppState, wanted_id: i64, guid: &str, files: &[&Path]) -> i64 {
    let list: Vec<Value> = files.iter().map(|f| json!({ "path": f.display().to_string(), "size": 1 })).collect();
    st.db
        .execute(
            "INSERT INTO grabs (wanted_id, lantern_id, guid, title, status, files, created_at, updated_at) VALUES (?,?,?,?,'completed',?,0,0)",
            p![wanted_id, format!("l-{guid}"), guid, format!("Project Hail Mary {guid}"), Value::Array(list).to_string()],
        )
        .unwrap();
    st.db.execute("UPDATE wanted SET status='downloaded' WHERE id=?", p![wanted_id]).unwrap();
    st.db.one("SELECT MAX(id) id FROM grabs", p![]).unwrap().unwrap().i("id")
}

/// An M4B of `secs` seconds of silence with `chapters` chapters and these tags, made by ffmpeg.
fn m4b(dir: &Path, name: &str, secs: u32, chapters: u32, tags: &[(&str, &str)], lang: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let meta = dir.join(format!("{name}.ffmeta"));
    let mut m = String::from(";FFMETADATA1\n");
    let len = secs * 1000 / chapters.max(1);
    for i in 0..chapters {
        m.push_str(&format!("[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle=Chapter {}\n", i * len, (i + 1) * len, i + 1));
    }
    std::fs::write(&meta, m).unwrap();
    let out = dir.join(format!("{name}.m4b"));
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-y", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=8000:cl=mono", "-i"]).arg(&meta).args([
        "-map",
        "0:a",
        "-map_metadata",
        "1",
        "-map_chapters",
        "1",
        "-t",
        &secs.to_string(),
        "-c:a",
        "aac",
        "-b:a",
        "8k",
    ]);
    for (k, v) in tags {
        cmd.arg("-metadata").arg(format!("{k}={v}"));
    }
    if !lang.is_empty() {
        cmd.arg("-metadata:s:a:0").arg(format!("language={lang}"));
    }
    let st = cmd.args(["-f", "mp4"]).arg(&out).status().expect("ffmpeg is needed for these tests");
    assert!(st.success());
    out
}

/// An EPUB with this metadata (`[isbn, title, author, language]`) and `words` words, optionally locked (an
/// encryption.xml that encrypts content).
fn epub(dir: &Path, name: &str, [isbn, title, author, lang]: [&str; 4], words: usize, drm: bool) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let out = dir.join(format!("{name}.epub"));
    let mut z = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = zip::write::SimpleFileOptions::default();
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", deflated).unwrap();
    z.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
    if drm {
        z.start_file("META-INF/encryption.xml", deflated).unwrap();
        z.write_all(br#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:enc="http://www.w3.org/2001/04/xmlenc#"><enc:EncryptedData><enc:EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></enc:EncryptedData></encryption>"#).unwrap();
    }
    z.start_file("OEBPS/content.opf", deflated).unwrap();
    let opf = format!(
        r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:isbn:{isbn}</dc:identifier><dc:title>{title}</dc:title><dc:creator>{author}</dc:creator><dc:language>{lang}</dc:language></metadata><manifest><item id="c1" href="text/c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#
    );
    z.write_all(opf.as_bytes()).unwrap();
    z.start_file("OEBPS/text/c1.xhtml", deflated).unwrap();
    let body = "word ".repeat(words);
    z.write_all(
        format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>x</title></head><body><p>{body}</p></body></html>").as_bytes(),
    )
    .unwrap();
    z.finish().unwrap();
    out
}

/// The wanted entry `id` as the list shows it.
async fn wanted_entry(app: &axum::Router, id: i64) -> Value {
    let (_, w) = call(app, admin("GET", "/api/wanted", None)).await;
    w["wanted"].as_array().unwrap().iter().find(|x| x["id"] == id).cloned().unwrap()
}

#[tokio::test]
async fn the_right_audiobook_is_verified() {
    let (app, st, dir, id) = wanted("vf-audio-ok", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0).await;
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
    let (app, st, dir, id) = wanted("vf-audio-sample", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0).await;
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
    let (app, st, dir, id) = wanted("vf-audio-untagged", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0).await;
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
    let (app, st, dir, id) = wanted("vf-ebook", json!({ "isbn": "9780593135204" }), 0, 0, 40).await;
    let ok = epub(&dir.join("dl"), "ok", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, false);
    arrived(&st, id, "epub", &[&ok]);
    server::verify::pending(&st).await.unwrap();
    assert_eq!(wanted_entry(&app, id).await["status"], "verified");

    let (app, st, dir, id) = wanted("vf-ebook-drm", json!({ "isbn": "9780593135204" }), 0, 0, 40).await;
    let locked = epub(&dir.join("dl"), "locked", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, true);
    arrived(&st, id, "epub", &[&locked]);
    server::verify::pending(&st).await.unwrap();
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "review");
    assert!(w["note"].as_str().unwrap().contains("DRM"), "{w}");
}

#[tokio::test]
async fn another_book_or_a_short_epub_is_caught() {
    let (app, st, dir, id) = wanted("vf-ebook-wrong", json!({ "isbn": "9780593135204" }), 0, 0, 40).await;
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
    let (app, st, dir, id) = wanted("vf-nofiles", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0).await;
    std::fs::create_dir_all(dir.join("dl")).unwrap();
    let jpg = dir.join("dl/cover.jpg");
    std::fs::write(&jpg, b"x").unwrap();
    arrived(&st, id, "m4b", &[&jpg]);
    server::verify::pending(&st).await.unwrap();
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "review");
    assert_eq!(w["note"], "This isn't the edition you asked for. None of the 1 files that arrived is an audiobook Folio can read.");
}
