//! The Audiobookshelf port's backups (PARITY P6): listing, making, downloading, restoring, uploading and
//! removing backups and moving the backup folder. Replays the recorded requests in the recorded order and compares
//! every answer with the pinned original's; uploads (which the recorder can't send) and what a restore puts back
//! are checked here directly.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{set_up, Replay};
use axum::body::Body;
use axum::http::{header, Request};
use kit_server_core::Db;
use serde_json::{json, Value};
use std::io::Write;
use tower::ServiceExt;

/// The keys of a backup that change with every run (its time and where it is).
const BACKUP_KEYS: [&str; 9] =
    ["id", "datePretty", "fullPath", "path", "filename", "fileSize", "createdAt", "backupDirPath", "backupLocation"];

/// Adds the guest account the recording used for the "not an admin" answers.
async fn with_guest(r: &mut Replay) {
    for name in ["p6b_guest_create", "p6b_guest_login"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn backups_answer_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-backups").await;
    r.m.volatile.extend(BACKUP_KEYS.map(String::from));
    with_guest(&mut r).await;
    for name in ["backups_guest", "backups_empty", "backups_create", "backups_list"] {
        r.like(name).await;
    }
    let a = r.send("backups_download").await;
    assert_eq!(a.status, 200);
    assert_eq!(a.content_type, "application/octet-stream");
    for name in [
        "backups_download_missing",
        "backups_apply",
        "backups_apply_missing",
        "backups_list_after_apply",
        "backups_path_missing",
        "backups_path_blank",
        "backups_path_not_text",
        "backups_path_same",
        "backups_path_move",
        "backups_list_moved",
        "backups_path_bad",
        "backups_path_back",
        "backups_list_back",
        "backups_create_guest",
        "backups_upload_no_file",
        "backups_delete",
        "backups_delete_missing",
        "p6b_guest_delete",
    ] {
        r.like(name).await;
    }
    assert_eq!(st.abs_server.backups.json(), Vec::<Value>::new());
}

/// A multipart body with one file field named `file`.
fn multipart(filename: &str, data: &[u8]) -> (String, Vec<u8>) {
    let boundary = "folio-test-boundary";
    let mut body = Vec::new();
    write!(body, "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n").unwrap();
    write!(body, "Content-Type: application/octet-stream\r\n\r\n").unwrap();
    body.extend_from_slice(data);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// Uploads a file as a backup; the status and the body as text.
async fn upload(r: &Replay, filename: &str, data: &[u8]) -> (u16, String) {
    let (ctype, body) = multipart(filename, data);
    let req = Request::post("/api/backups/upload")
        .header(header::AUTHORIZATION, format!("Bearer {}", r.ctx["token"]))
        .header(header::CONTENT_TYPE, ctype)
        .body(Body::from(body))
        .unwrap();
    let res = r.app.clone().oneshot(req).await.unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 24).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A zip with the given files.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut out);
    for (name, data) in files {
        z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
    out.into_inner()
}

#[tokio::test]
async fn a_restore_puts_back_the_database_and_metadata() {
    let (r, st, _dir) = set_up("abs-backups-restore").await;
    let items = st.abs_server.metadata_path.join("items/item1");
    std::fs::create_dir_all(&items).unwrap();
    std::fs::write(items.join("metadata.json"), "{\"title\":\"Kept\"}").unwrap();
    let made =
        abs_replay::send(&r.app, "POST", "/api/backups", None, &[("Authorization".into(), format!("Bearer {}", r.ctx["token"]))]).await;
    assert_eq!(made.status, 200, "{}", made.body);
    let id = made.body["backups"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(made.body["backups"][0]["key"], "sqlite");
    assert_eq!(made.body["backups"][0]["serverVersion"], server::abs::ABS_VERSION);

    // changes after the backup are undone by the restore
    let now = "2026-11-05 18:00:00.000 +00:00";
    st.abs
        .execute("INSERT INTO books (id, title, createdAt, updatedAt) VALUES ('later', 'Added later', ?, ?)", vec![json!(now), json!(now)])
        .unwrap();
    std::fs::write(items.join("metadata.json"), "changed").unwrap();
    let auth = [("Authorization".to_string(), format!("Bearer {}", r.ctx["token"]))];
    let a = abs_replay::send(&r.app, "GET", &format!("/api/backups/{id}/apply"), None, &auth).await;
    assert_eq!((a.status, a.body.clone()), (200, json!("OK")));
    assert!(st.abs.one("SELECT id FROM books WHERE id = 'later'", vec![]).unwrap().is_none());
    assert_eq!(std::fs::read_to_string(items.join("metadata.json")).unwrap(), "{\"title\":\"Kept\"}");
    assert!(st.abs.one("SELECT id FROM users WHERE username = 'root'", vec![]).unwrap().is_some(), "the users are back");
    assert!(st.abs_server.metadata_path.join("cache/covers").is_dir());

    // the same file uploaded under another name is listed in place of the one with its id
    let file = std::fs::read(st.abs_server.backups.find(&id).unwrap().full_path).unwrap();
    let (status, body) = upload(&r, "copy.audiobookshelf", &file).await;
    assert_eq!(status, 200, "{body}");
    let list: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list["backups"].as_array().unwrap().len(), 1);
    assert_eq!(list["backups"][0]["filename"], "copy.audiobookshelf");

    // the original's refusals
    assert_eq!(upload(&r, "x.zip", &file).await, (500, "Invalid backup file".into()));
    assert_eq!(
        upload(&r, "bad.audiobookshelf", b"not a zip").await,
        (400, "Failed to read backup file - backup might not be a valid .zip file".into())
    );
    let no_db = zip_of(&[("details", b"a\nsqlite\n1\n2.37.2")]);
    assert_eq!(
        upload(&r, "nodb.audiobookshelf", &no_db).await,
        (500, "Invalid backup with no absdatabase.sqlite file - might be a backup created on an old Audiobookshelf server.".into())
    );
    let no_details = zip_of(&[("absdatabase.sqlite", b"")]);
    assert_eq!(upload(&r, "nodetails.audiobookshelf", &no_details).await, (400, "Invalid backup file - missing details entry".into()));
    let old = zip_of(&[("absdatabase.sqlite", b""), ("details", b"a\n1\n5")]);
    assert_eq!(
        upload(&r, "old.audiobookshelf", &old).await,
        (500, "Invalid backup. Might be a backup created before version 2.0.0.".into())
    );
    let backups = st.abs_server.settings().str("backupPath").to_string();
    for refused in ["bad", "nodb", "nodetails", "old"] {
        assert!(!std::path::Path::new(&backups).join(format!("{refused}.audiobookshelf")).exists(), "{refused} is removed");
    }
}

#[tokio::test]
async fn only_the_newest_backups_are_kept() {
    let (r, st, _dir) = set_up("abs-backups-keep").await;
    let older = zip_of(&[("absdatabase.sqlite", b""), ("details", b"2020-01-01T0000\nsqlite\n1577836800000\n2.37.2")]);
    let (status, body) = upload(&r, "2020-01-01T0000.audiobookshelf", &older).await;
    assert_eq!(status, 200, "{body}");
    let mut s = st.abs_server.settings();
    s.0.insert("backupsToKeep".into(), json!(1));
    st.abs_server.save_settings(&st.abs, s).unwrap();
    assert!(server::abs::backups::run(&st).await);
    let list = st.abs_server.backups.json();
    assert_eq!(list.len(), 1);
    assert_ne!(list[0]["id"], "2020-01-01T0000");
    let dir = st.abs_server.settings().str("backupPath").to_string();
    assert!(!std::path::Path::new(&dir).join("2020-01-01T0000.audiobookshelf").exists());

    // a fresh start finds the backup in the folder
    let again = server::abs::backups::Manager::new(&dir);
    assert_eq!(again.json(), list);
}
