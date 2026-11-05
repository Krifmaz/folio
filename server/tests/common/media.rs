//! Real book files for the tests: M4Bs made by ffmpeg, EPUBs written here, and a wanted book whose download
//! has arrived.
#![allow(dead_code)]

use super::{admin, call, lantern, server, sources};
use axum::http::StatusCode;
use kit_server_core::{p, Db, RowExt};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A server with the stand-in sources and Lantern, and a wanted entry for this ASIN or ISBN. The edition's
/// runtime (minutes), chapter count and pages are set as given, so short test files can be the whole book.
pub async fn wanted(
    name: &str,
    id: Value,
    runtime_min: u32,
    chapters: usize,
    pages: u32,
    extra: &[(&str, String)],
) -> (axum::Router, server::AppState, PathBuf, i64) {
    let mut env = sources().await;
    env.extend(lantern().await);
    env.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
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
pub fn arrived(st: &server::AppState, wanted_id: i64, guid: &str, files: &[&Path]) -> i64 {
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
pub fn m4b(dir: &Path, name: &str, secs: u32, chapters: u32, tags: &[(&str, &str)], lang: &str) -> PathBuf {
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
pub fn epub(dir: &Path, name: &str, [isbn, title, author, lang]: [&str; 4], words: usize, drm: bool) -> PathBuf {
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
pub async fn wanted_entry(app: &axum::Router, id: i64) -> Value {
    let (_, w) = call(app, admin("GET", "/api/wanted", None)).await;
    w["wanted"].as_array().unwrap().iter().find(|x| x["id"] == id).cloned().unwrap()
}

/// An MP3 of `secs` seconds of silence titled `title`, made by ffmpeg.
pub fn mp3(dir: &Path, name: &str, secs: u32, title: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let out = dir.join(format!("{name}.mp3"));
    let st = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=8000:cl=mono",
            "-t",
            &secs.to_string(),
            "-c:a",
            "libmp3lame",
            "-b:a",
            "8k",
        ])
        .arg("-metadata")
        .arg(format!("title={title}"))
        .arg(&out)
        .status()
        .expect("ffmpeg is needed for these tests");
    assert!(st.success());
    out
}

/// What ffprobe says about `path`: (seconds, chapter titles, tags by lower-case name, has a cover picture).
pub fn probe(path: &Path) -> (f64, Vec<String>, std::collections::BTreeMap<String, String>, bool) {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters"])
        .arg(path)
        .output()
        .expect("ffprobe is needed for these tests");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let secs = v["format"]["duration"].as_str().unwrap_or("0").parse().unwrap_or(0.0);
    let chapters = v["chapters"].as_array().unwrap().iter().map(|c| c["tags"]["title"].as_str().unwrap_or("").to_string()).collect();
    let tags = v["format"]["tags"]
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    let cover = v["streams"].as_array().unwrap().iter().any(|s| s["codec_type"] == "video");
    (secs, chapters, tags, cover)
}
