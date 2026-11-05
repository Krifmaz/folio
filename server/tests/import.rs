//! Filing checked downloads: one chaptered, tagged M4B with its cover from a lone M4B or from several MP3s,
//! EPUBs with the edition's metadata, hardlinks when clean-up is off, and the `edition_files` rows the
//! player reads. Real files throughout (ffmpeg makes the audio).

mod common;

use common::media::{arrived, epub, m4b, mp3, probe, wanted, wanted_entry};
use common::{admin, call, serve};
use kit_server_core::{p, Db, RowExt};
use serde_json::{json, Value};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;

/// A stand-in for the cover's host: one JPEG at `/cover.jpg`.
async fn cover_host() -> String {
    // the smallest JPEG ffmpeg will take as a cover: made once by ffmpeg, kept as bytes
    let dir = std::env::temp_dir().join(format!("folio-cover-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let jpg = dir.join("c.jpg");
    if !jpg.exists() {
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "color=c=blue:s=64x64", "-frames:v", "1"])
            .arg(&jpg)
            .status()
            .unwrap();
        assert!(ok.success());
    }
    let bytes = std::fs::read(&jpg).unwrap();
    serve(axum::Router::new().route("/cover.jpg", axum::routing::get(move || async move { ([("content-type", "image/jpeg")], bytes) })))
        .await
}

/// The library folder of a test server, and the settings that point there.
fn library(name: &str) -> (PathBuf, Vec<(&'static str, String)>) {
    let lib = std::env::temp_dir().join(format!("folio-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&lib);
    (lib.clone(), vec![("APP_LIBRARY_DIR", lib.display().to_string())])
}

/// The edition's files as `edition_files` has them.
fn files(st: &server::AppState) -> Vec<kit_server_core::Row> {
    st.db.query("SELECT * FROM edition_files ORDER BY kind, position", p![]).unwrap()
}

#[tokio::test]
async fn a_checked_m4b_is_filed_with_chapters_tags_and_cover() {
    let (lib, env) = library("m4b");
    let (app, st, dir, id) = wanted("im-m4b", json!({ "asin": "B08G9PRS1K" }), 2, 3, 0, &env).await;
    let host = cover_host().await;
    st.db.execute("UPDATE editions SET meta=json_set(meta, '$.cover_url', ?)", p![format!("{host}/cover.jpg")]).unwrap();
    let src = m4b(&dir.join("dl"), "book", 120, 0, &[("album", "Project Hail Mary"), ("composer", "Ray Porter")], "eng");
    let before = std::fs::read(&src).unwrap();
    arrived(&st, id, "m4b", &[&src]);
    server::verify::pending(&st).await.unwrap();
    assert_eq!(server::import::pending(&st).await.unwrap(), 1);

    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "imported", "{w}");
    let rows = files(&st);
    let audio = rows.iter().find(|r| r.s("kind") == "audio").unwrap();
    assert!(audio.s("path").starts_with("Andy Weir/"), "{}", audio.s("path"));
    assert!(audio.s("path").ends_with("Project Hail Mary (2021)/Project Hail Mary.m4b"), "{}", audio.s("path"));
    assert!(w["note"].as_str().unwrap().starts_with("Filed in Andy Weir/"), "{w}");
    assert_eq!(audio.s("mime"), "audio/mp4");
    assert!((119_000..121_000).contains(&audio.i("duration_ms")), "{}", audio.i("duration_ms"));
    let chapters: Value = serde_json::from_str(&audio.s("chapters")).unwrap();
    assert_eq!(chapters.as_array().map(Vec::len), Some(3), "Audnexus' chapters fit the length");
    assert!(rows.iter().any(|r| r.s("kind") == "cover"));

    let (secs, titles, tags, has_cover) = probe(&lib.join(audio.s("path")));
    assert!((119.0..121.0).contains(&secs));
    assert_eq!(titles, ["Chapter 1", "Chapter 2", "Chapter 3"]);
    assert_eq!(tags.get("album").map(String::as_str), Some("Project Hail Mary"));
    assert_eq!(tags.get("artist").map(String::as_str), Some("Andy Weir"));
    assert_eq!(tags.get("composer").map(String::as_str), Some("Ray Porter"));
    assert!(has_cover, "the cover is in the file");
    assert_eq!(std::fs::read(&src).unwrap(), before, "Lantern's copy is never changed");
}

#[tokio::test]
async fn several_mp3s_become_one_m4b_with_a_chapter_each() {
    let (lib, env) = library("mp3");
    let (app, st, dir, id) = wanted("im-mp3", json!({ "asin": "B08G9PRS1K" }), 1, 0, 0, &env).await;
    st.db.execute("UPDATE editions SET meta=json_set(meta, '$.cover_url', '')", p![]).unwrap();
    let a = mp3(&dir.join("dl"), "01", 20, "Opening");
    let b = mp3(&dir.join("dl"), "02", 20, "");
    let c = mp3(&dir.join("dl"), "03", 20, "The End");
    let g = arrived(&st, id, "mp3", &[&c, &a, &b]);
    server::verify::pending(&st).await.unwrap();
    // untagged MP3s wait for a look; accepting them files them
    let (s, _) = call(&app, admin("POST", &format!("/api/review/{g}/accept"), None)).await;
    assert_eq!(s, axum::http::StatusCode::OK);
    // filed by the background run Accept starts, or by this one, whichever comes first: once either way
    let _ = server::import::run(&st, g).await;
    for _ in 0..50 {
        if wanted_entry(&app, id).await["status"] == "imported" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let w = wanted_entry(&app, id).await;
    assert_eq!(w["status"], "imported", "{w}");
    assert!(w["note"].as_str().unwrap().contains("3 files were joined into one M4B with 3 chapters"), "{w}");
    let rows = files(&st);
    assert_eq!(rows.len(), 1, "one M4B, no cover");
    let (secs, titles, _, _) = probe(&lib.join(rows[0].s("path")));
    assert!((59.0..62.0).contains(&secs), "{secs}");
    assert_eq!(titles, ["Opening", "Part 2", "The End"], "in name order, one chapter per file");
}

#[tokio::test]
async fn an_epub_gets_the_editions_metadata() {
    let (lib, env) = library("epub");
    let (app, st, dir, id) = wanted("im-epub", json!({ "isbn": "9780593135204" }), 0, 0, 40, &env).await;
    st.db.execute("UPDATE editions SET meta=json_set(meta, '$.cover_url', '')", p![]).unwrap();
    let src = epub(&dir.join("dl"), "dl", ["9780593135204", "project hail mary", "Andy Weir", "en"], 10_000, false);
    arrived(&st, id, "epub", &[&src]);
    server::verify::pending(&st).await.unwrap();
    server::import::pending(&st).await.unwrap();
    assert_eq!(wanted_entry(&app, id).await["status"], "imported");
    let rows = files(&st);
    assert_eq!((rows[0].s("kind").as_str(), rows[0].s("mime").as_str()), ("epub", "application/epub+zip"));
    let facts = server::verify::epub::inspect(&lib.join(rows[0].s("path")));
    assert!(facts.valid, "{}", facts.problem);
    assert_eq!(facts.title, "Project Hail Mary", "the edition's title replaced the file's");
    assert_eq!(facts.isbn13, "9780593135204");
    assert_eq!(facts.words, 10_000);
}

#[tokio::test]
async fn without_clean_up_the_download_is_hardlinked() {
    let (lib, mut env) = library("link");
    env.push(("APP_IMPORT_REWRITE", "false".into()));
    let (_, st, _, id) = wanted("im-link", json!({ "isbn": "9780593135204" }), 0, 0, 40, &env).await;
    st.db.execute("UPDATE editions SET meta=json_set(meta, '$.cover_url', '')", p![]).unwrap();
    // the library and the downloads on the same disk, as in a real setup
    let dl = std::env::temp_dir().join(format!("folio-dl-link-{}", std::process::id()));
    let src = epub(&dl, "dl", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, false);
    arrived(&st, id, "epub", &[&src]);
    server::verify::pending(&st).await.unwrap();
    server::import::pending(&st).await.unwrap();
    let rows = files(&st);
    let filed = lib.join(rows[0].s("path"));
    assert_eq!(std::fs::metadata(&filed).unwrap().ino(), std::fs::metadata(&src).unwrap().ino(), "the same file, linked");
}

#[tokio::test]
async fn local_files_from_another_app_are_checked_and_filed() {
    let (lib, env) = library("local");
    let (app, st, dir) = common::server("im-local", {
        let mut e = common::sources().await;
        e.extend(env.iter().map(|(k, v)| (k.to_string(), v.clone())));
        e
    })
    .await;
    let (_, e) = call(&app, admin("POST", "/api/library/editions", Some(json!({ "isbn": "9780593135204" })))).await;
    let edition = e["edition_id"].as_i64().unwrap();
    st.db.execute("UPDATE editions SET meta=json_set(meta, '$.pages', 40, '$.cover_url', '') WHERE id=?", p![edition]).unwrap();
    let src = epub(&dir.join("calibre"), "book", ["9780593135204", "Project Hail Mary", "Andy Weir", "en"], 10_000, false);
    let g = server::import::queue_local(&st.db, edition, std::slice::from_ref(&src), "Calibre").unwrap();
    server::verify::pending(&st).await.unwrap();
    server::import::pending(&st).await.unwrap();
    let r = st.db.one("SELECT status FROM grabs WHERE id=?", p![g]).unwrap().unwrap();
    assert_eq!(r.s("status"), "imported");
    let (_, w) = call(&app, admin("GET", "/api/wanted", None)).await;
    assert_eq!((w["wanted"][0]["status"].as_str(), w["wanted"][0]["source"].as_str()), (Some("imported"), Some("Calibre")));
    assert!(lib.join(files(&st)[0].s("path")).exists());
    assert!(src.exists(), "the source stays where it was");
}
