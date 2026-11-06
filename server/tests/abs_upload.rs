//! The Audiobookshelf port's upload (PARITY P6): replays the recorded answers, then sends real forms and checks
//! where the files land and how a refused upload is answered.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{set_up, Replay};
use axum::body::Body;
use axum::http::{header, Request};
use kit_server_core::Db;
use serde_json::json;
use std::io::Write;
use tower::ServiceExt;

/// A multipart form with text fields and files (field name, file name, contents).
fn form(fields: &[(&str, &str)], files: &[(&str, &str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "folio-test-boundary";
    let mut body = Vec::new();
    for (name, value) in fields {
        write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").unwrap();
    }
    for (field, filename, data) in files {
        write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n").unwrap();
        write!(body, "Content-Type: application/octet-stream\r\n\r\n").unwrap();
        body.extend_from_slice(data);
        write!(body, "\r\n").unwrap();
    }
    write!(body, "--{boundary}--\r\n").unwrap();
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// Sends a form to `/api/upload` as root; the status and the body as text.
async fn upload(r: &Replay, fields: &[(&str, &str)], files: &[(&str, &str, &[u8])]) -> (u16, String) {
    let (ctype, body) = form(fields, files);
    let req = Request::post("/api/upload")
        .header(header::AUTHORIZATION, format!("Bearer {}", r.ctx["token"]))
        .header(header::CONTENT_TYPE, ctype)
        .body(Body::from(body))
        .unwrap();
    let res = r.app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 24).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn uploads_answer_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-upload").await;
    for name in ["p6u_guest_create", "p6u_guest_login", "upload_guest", "upload_no_files", "upload_empty", "p6u_guest_delete"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn uploaded_files_land_in_the_library_folder() {
    let (r, st, dir) = set_up("abs-upload-files").await;
    let now = "2026-11-05 18:00:00.000 +00:00";
    let books = dir.join("books");
    let podcasts = dir.join("podcasts");
    st.abs
        .execute(
            "INSERT INTO libraries (id, name, displayOrder, mediaType, createdAt, updatedAt) VALUES ('pods', 'Podcasts', 2, 'podcast', ?, ?)",
            vec![json!(now), json!(now)],
        )
        .unwrap();
    for (id, path, library) in [("f1", &books, r.ctx["library"].as_str()), ("f2", &podcasts, "pods")] {
        st.abs
            .execute(
                "INSERT INTO libraryFolders (id, path, libraryId, createdAt, updatedAt) VALUES (?, ?, ?, ?, ?)",
                vec![json!(id), json!(path.display().to_string()), json!(library), json!(now), json!(now)],
            )
            .unwrap();
    }
    let library = r.ctx["library"].clone();
    let tmp = st.abs_server.metadata_path.join("tmp");
    let waiting = || std::fs::read_dir(&tmp).map(|d| d.count()).unwrap_or(0);

    let fields = [("title", "Part: One"), ("author", "An Author"), ("series", "A/Series"), ("library", &library), ("folder", "f1")];
    let (status, _) = upload(&r, &fields, &[("0", "Ch 1.mp3", b"one"), ("1", "Ch 2?.mp3", b"two")]).await;
    assert_eq!(status, 200);
    let out = books.join("An Author").join("ASeries").join("Part - One");
    assert_eq!(std::fs::read(out.join("Ch 1.mp3")).unwrap(), b"one");
    assert_eq!(std::fs::read(out.join("Ch 2.mp3")).unwrap(), b"two");
    assert_eq!(waiting(), 0);

    // no author or series: straight under the folder; podcasts are only one folder deep
    let (status, _) = upload(&r, &[("title", "Solo"), ("library", &library), ("folder", "f1")], &[("0", "s.m4b", b"s")]).await;
    assert_eq!(status, 200);
    assert!(books.join("Solo").join("s.m4b").is_file());
    let fields = [("title", "Show"), ("author", "Host"), ("library", "pods"), ("folder", "f2")];
    let (status, _) = upload(&r, &fields, &[("0", "ep.mp3", b"e")]).await;
    assert_eq!(status, 200);
    assert!(podcasts.join("Show").join("ep.mp3").is_file());

    // refusals, with the waiting files removed
    for (fields, want) in [
        (vec![("library", library.as_str()), ("folder", "f1")], (400, "Invalid request body")),
        (vec![("title", "T"), ("title", "U"), ("library", library.as_str()), ("folder", "f1")], (400, "Invalid request body")),
        (vec![("title", "T"), ("library", "nope"), ("folder", "f1")], (404, "Library not found")),
        (vec![("title", "T"), ("library", library.as_str()), ("folder", "f2")], (404, "Folder not found")),
    ] {
        let (status, body) = upload(&r, &fields, &[("0", "x.mp3", b"x")]).await;
        assert_eq!((status, body.as_str()), want, "{fields:?}");
        assert_eq!(waiting(), 0, "{fields:?}");
    }
    let (status, body) = upload(&r, &[("title", "T"), ("library", &library), ("folder", "f1")], &[]).await;
    assert_eq!((status, body.as_str()), (400, "Bad Request"));
}
