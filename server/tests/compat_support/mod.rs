//! What the compatibility tests share: a library with one audiobook and one ebook whose files are on disk, and
//! requests signed in the ways the outside apps sign in.
#![allow(dead_code)]

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use base64::Engine;
use kit_server_core::{p, Db};
use server::metadata::{Chapter, EditionMeta, Format, Person, SeriesRef};
use std::collections::HashMap;
use std::path::PathBuf;
use tower::ServiceExt;

#[path = "../common/mod.rs"]
pub mod common;

/// The test owner's username and password.
pub const USER: &str = "admin";
/// The test owner's password.
pub const PASSWORD: &str = common::PASSWORD;

/// The library the tests use: the server, its state, the library folder and the two books' edition ids.
pub struct Library {
    /// the whole app
    pub app: Router,
    /// its state
    pub st: server::AppState,
    /// the library folder
    pub dir: PathBuf,
    /// the audiobook's edition (Project Hail Mary, two MP3 files of 600 and 400 seconds, a cover)
    pub audio: i64,
    /// the ebook's edition (The Martian, one EPUB)
    pub ebook: i64,
}

/// The audiobook's facts.
pub fn audiobook_meta() -> EditionMeta {
    EditionMeta {
        format: Format::Audiobook,
        asin: "B08G9PRS1K".into(),
        title: "Project Hail Mary".into(),
        authors: vec![Person { name: "Andy Weir".into(), ..Person::default() }],
        narrators: vec!["Ray Porter".into()],
        series: vec![],
        language: "en".into(),
        runtime_min: Some(17),
        abridged: Some(false),
        publisher: "Audible Studios".into(),
        release_date: "2021-05-04".into(),
        description: "<p>A lone astronaut &amp; a mission.</p>".into(),
        genres: vec!["Science Fiction".into()],
        chapters: vec![Chapter { title: "Chapter 1".into(), start_ms: 0, length_ms: 600_000 }],
        ..EditionMeta::default()
    }
}

/// The ebook's facts.
pub fn ebook_meta() -> EditionMeta {
    EditionMeta {
        format: Format::Ebook,
        isbn13: "9780553418026".into(),
        title: "The Martian".into(),
        subtitle: "A Novel".into(),
        authors: vec![Person { name: "Andy Weir".into(), ..Person::default() }],
        series: vec![SeriesRef { name: "Mars".into(), position: "1".into(), ..SeriesRef::default() }],
        language: "en".into(),
        pages: Some(384),
        release_date: "2014".into(),
        ..EditionMeta::default()
    }
}

/// A file to put in the library: its path under the library folder, kind, media type, bytes, play order and
/// length.
pub struct NewFile<'a> {
    /// the path under the library folder
    pub rel: &'a str,
    /// `audio`, `epub`, `cover` …
    pub kind: &'a str,
    /// the media type
    pub mime: &'a str,
    /// what is in it
    pub bytes: &'a [u8],
    /// the play order
    pub position: i64,
    /// how long it plays
    pub duration_ms: i64,
}

/// Adds a row to `edition_files` for edition `edition` and writes the file under the library folder `dir`.
pub fn add_file(st: &server::AppState, dir: &std::path::Path, edition: i64, f: NewFile) -> i64 {
    let path = dir.join(f.rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, f.bytes).unwrap();
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let chapters = "[{\"title\":\"Part\",\"start_ms\":0,\"end_ms\":1000}]";
    st.db
        .execute(
            "INSERT INTO edition_files (edition_id, path, name, kind, mime, size, position, duration_ms, chapters, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
            p![edition, f.rel, name, f.kind, f.mime, f.bytes.len() as i64, f.position, f.duration_ms, chapters, kit_server_core::db::now()],
        )
        .unwrap();
    st.db.one("SELECT last_insert_rowid() AS id", p![]).unwrap().map(|r| kit_server_core::RowExt::i(&r, "id")).unwrap()
}

/// A server whose library holds the audiobook and the ebook, with `env` as extra settings.
pub async fn library(name: &str, mut env: HashMap<String, String>) -> Library {
    let dir = std::env::temp_dir().join(format!("folio-compat-lib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    env.insert("APP_LIBRARY_DIR".into(), dir.to_string_lossy().into());
    let (app, st, _) = common::server(name, env).await;
    let audio = server::library::store::save(&st.db, &audiobook_meta()).unwrap().edition_id;
    let ebook = server::library::store::save(&st.db, &ebook_meta()).unwrap().edition_id;
    add_file(
        &st,
        &dir,
        audio,
        NewFile {
            rel: "Andy Weir/Project Hail Mary (2021)/01.mp3",
            kind: "audio",
            mime: "audio/mpeg",
            bytes: &[1u8; 6000],
            position: 0,
            duration_ms: 600_000,
        },
    );
    add_file(
        &st,
        &dir,
        audio,
        NewFile {
            rel: "Andy Weir/Project Hail Mary (2021)/02.mp3",
            kind: "audio",
            mime: "audio/mpeg",
            bytes: &[2u8; 4000],
            position: 1,
            duration_ms: 400_000,
        },
    );
    add_file(
        &st,
        &dir,
        audio,
        NewFile {
            rel: "Andy Weir/Project Hail Mary (2021)/cover.jpg",
            kind: "cover",
            mime: "image/jpeg",
            bytes: b"\xff\xd8\xff",
            position: 0,
            duration_ms: 0,
        },
    );
    let epub = epub_bytes();
    let rel = "Andy Weir/Mars/01 - The Martian (2014)/The Martian.epub";
    add_file(&st, &dir, ebook, NewFile { rel, kind: "epub", mime: "application/epub+zip", bytes: &epub, position: 0, duration_ms: 0 });
    Library { app, st, dir, audio, ebook }
}

/// A small real EPUB: three chapters (`OEBPS/one.xhtml`, `two.xhtml`, `three.xhtml`) and a stored picture
/// big enough for KOReader's sampled MD5 to read several pieces.
pub fn epub_bytes() -> Vec<u8> {
    use std::io::Write;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).unwrap();
    z.write_all(b"application/epub+zip").unwrap();
    z.start_file("META-INF/container.xml", stored).unwrap();
    z.write_all(br#"<container><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#).unwrap();
    z.start_file("OEBPS/content.opf", stored).unwrap();
    z.write_all(
        br#"<package><manifest><item id="a" href="one.xhtml"/><item id="b" href="two.xhtml"/><item id="c" href="three.xhtml"/></manifest>
        <spine><itemref idref="a"/><itemref idref="b"/><itemref idref="c"/></spine></package>"#,
    )
    .unwrap();
    for name in ["one", "two", "three"] {
        z.start_file(format!("OEBPS/{name}.xhtml"), stored).unwrap();
        z.write_all(format!("<html><body><p>{name}</p></body></html>").as_bytes()).unwrap();
    }
    z.start_file("OEBPS/picture.bin", stored).unwrap();
    z.write_all(&(0..300_000u32).map(|i| (i * 7 % 251) as u8).collect::<Vec<_>>()).unwrap();
    z.finish().unwrap().into_inner()
}

/// `Authorization: Basic` for the owner (or `password`).
pub fn basic(password: &str) -> String {
    format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{password}")))
}

/// A request with the given headers and, when not empty, a JSON body.
pub fn req(method: &str, path: &str, headers: &[(&str, &str)], body: Option<serde_json::Value>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

/// One request; the status, the headers and the body as text.
pub async fn text(app: &Router, r: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let resp = app.clone().oneshot(r).await.unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let b = axum::body::to_bytes(resp.into_body(), 1 << 26).await.unwrap();
    (status, headers, String::from_utf8_lossy(&b).into_owned())
}
