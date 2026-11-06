//! The Audiobookshelf port's shares (PARITY P6): admins share a book under a public link and visitors listen,
//! download and keep their place through a cookie. Replays the recorded requests and compares the answers with the
//! pinned original's; the shared book is one made here, so its own values are checked by shape.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{recorded_cookies, set_up, Replay};
use kit_server_core::Db;
use serde_json::{json, Value};

/// The cookie the recorded public requests send.
const COOKIE: &str = "6f1c2b9e-4a3d-4c8e-9b2a-1d2e3f4a5b6c";

/// The keys of an object, sorted.
fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

/// Adds a book with one audio file in its own folder, like the recording's, and points `{item}` at it.
fn add_book(r: &mut Replay, st: &server::AppState, dir: &std::path::Path) -> std::path::PathBuf {
    let db = &st.abs;
    let now = "2026-11-05 18:00:00.000 +00:00";
    let folder = dir.join("Becky Chambers").join("A Psalm");
    std::fs::create_dir_all(&folder).unwrap();
    let audio = folder.join("Part 01.mp3");
    std::fs::write(&audio, b"ID3 not really an mp3").unwrap();
    let audio_files = json!([{
        "index": 1, "ino": "11", "duration": 15, "mimeType": "audio/mpeg", "codec": "mp3",
        "metadata": {"filename": "Part 01.mp3", "ext": ".mp3", "path": audio.display().to_string(), "relPath": "Part 01.mp3", "size": 21}
    }]);
    db.execute(
        "INSERT INTO books (id, title, duration, explicit, abridged, tags, genres, narrators, audioFiles, chapters, createdAt, updatedAt) \
         VALUES ('0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d', 'A Psalm for the Wild-Built', 15, 0, 0, '[]', '[]', '[]', ?, '[]', ?, ?)",
        vec![json!(audio_files.to_string()), json!(now), json!(now)],
    )
    .unwrap();
    db.execute(
        "INSERT INTO libraryItems (id, ino, path, relPath, mediaId, mediaType, isFile, isMissing, isInvalid, libraryFiles, title, libraryId, createdAt, updatedAt) \
         VALUES ('1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d', '10', ?, 'Becky Chambers/A Psalm', '0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d', 'book', 0, 0, 0, '[]', 'A Psalm for the Wild-Built', ?, ?, ?)",
        vec![json!(folder.display().to_string()), json!(r.ctx["library"]), json!(now), json!(now)],
    )
    .unwrap();
    db.execute("UPDATE libraries SET settings = ? WHERE id = ?", vec![json!(r#"{"coverAspectRatio":1}"#), json!(r.ctx["library"])])
        .unwrap();
    r.ctx.insert("item".into(), "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d".into());
    audio
}

#[tokio::test]
async fn shares_answer_like_the_original() {
    let (mut r, st, dir) = set_up("abs-shares").await;
    let audio = add_book(&mut r, &st, &dir);
    for name in ["p6h_guest_create", "p6h_guest_login"] {
        r.like(name).await;
    }
    let a = r.send("share_item").await;
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(r.ctx["share_book"], "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d");
    for name in [
        "share_create_guest",
        "share_create_missing",
        "share_create_no_expiry",
        "share_create_bad_type",
        "share_create_unknown",
        "share_create",
        "share_create_again",
        "share_create_slug_taken",
        "public_share_unknown",
    ] {
        r.like(name).await;
    }

    // the visitor's session: the original's fields, starting at ?t=5, on the cookie's device
    let fx = parity::fixture("abs", "public_share");
    let a = r.send("public_share").await;
    assert_eq!((a.status, a.content_type.as_str()), (200, "application/json"), "{}", a.body);
    assert_eq!(a.cookies, recorded_cookies("public_share"));
    assert_eq!(keys(&a.body), keys(&fx.body));
    let ps = &a.body["playbackSession"];
    assert_eq!(keys(ps), keys(&fx.body["playbackSession"]));
    for k in
        ["currentTime", "startTime", "mediaPlayer", "playMethod", "coverAspectRatio", "userId", "libraryItem", "episodeId", "timeListening"]
    {
        assert_eq!(ps[k], fx.body["playbackSession"][k], "{k}");
    }
    assert_eq!(ps["deviceInfo"]["deviceId"], COOKIE);
    assert_eq!(ps["deviceInfo"]["clientName"], "Abs Web Share");
    let track = &ps["audioTracks"][0];
    assert_eq!(keys(track), keys(&fx.body["playbackSession"]["audioTracks"][0]));
    // Folio serves the port at the root, the original's image under /audiobookshelf (docs/DECISIONS.md)
    assert_eq!(track["contentUrl"], "/public/share/folio-share/track/1");
    let session_id = ps["id"].clone();
    let a = r.send("public_share_again").await;
    assert_eq!((a.body["playbackSession"]["id"].clone(), a.body["playbackSession"]["currentTime"].clone()), (session_id, json!(5)));
    assert!(a.cookies.is_empty());

    for name in ["public_share_track_no_cookie", "public_share_track_missing", "public_share_cover"] {
        r.like(name).await;
    }
    let a = r.send("public_share_track").await;
    assert_eq!((a.status, a.content_type.as_str()), (200, "audio/mpeg"));
    assert_eq!(a.body, Value::String(String::from_utf8_lossy(&std::fs::read(&audio).unwrap()).into()));
    let a = r.send("public_share_download").await;
    assert_eq!((a.status, a.content_type.as_str()), (200, "application/zip"));
    assert!(a.body.as_str().unwrap().starts_with("PK"));
    assert_eq!(std::fs::read_dir(st.abs_server.metadata_path.join("tmp")).unwrap().count(), 0);

    for name in ["public_share_progress_bad", "public_share_progress"] {
        r.like(name).await;
    }
    let a = r.send("public_share_again").await;
    assert_eq!(a.body["playbackSession"]["currentTime"], 12);

    // a cookie that isn't a session id is cleared and replaced
    let a = r.send("public_share_bad_cookie").await;
    assert_eq!(a.status, 200);
    assert_eq!(a.cookies, recorded_cookies("public_share_bad_cookie"));
    assert_eq!(a.body["playbackSession"]["currentTime"], 0);

    for name in ["share_delete_guest", "share_delete", "share_delete_again", "public_share_after_delete", "p6h_guest_delete"] {
        r.like(name).await;
    }
    assert!(st.abs.query("SELECT id FROM mediaItemShares", vec![]).unwrap().is_empty());
}

#[tokio::test]
async fn shares_are_loaded_at_start_and_end_on_time() {
    let (mut r, st, dir) = set_up("abs-shares-start").await;
    add_book(&mut r, &st, &dir);
    let db = &st.abs;
    let now = "2026-11-05 18:00:00.000 +00:00";
    let past = "2020-01-01 00:00:00.000 +00:00";
    let soon = server::abs::models::date_text(server::abs::models::now_ms() + 1500);
    for (id, slug, media, expires) in [
        ("s1", "kept", "0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d", None),
        ("s2", "ended", "book-x", Some(past.to_string())),
        ("s3", "soon", "book-y", Some(soon)),
    ] {
        db.execute(
            "INSERT INTO mediaItemShares (id, mediaItemId, mediaItemType, slug, expiresAt, isDownloadable, createdAt, updatedAt) \
             VALUES (?, ?, 'book', ?, ?, 0, ?, ?)",
            vec![json!(id), json!(media), json!(slug), json!(expires), json!(now), json!(now)],
        )
        .unwrap();
    }
    server::abs::shares::init(&st);
    let shares = &st.abs_server.shares;
    assert!(shares.find_by_slug("kept").is_some());
    assert!(shares.find_by_slug("ended").is_none());
    assert!(shares.find_by_slug("soon").is_some());
    let kept = shares.find_by_media_item_id("0b1c2d3e-4f50-4a6b-8c7d-9e0f1a2b3c4d").unwrap();
    assert_eq!(
        keys(&Value::Object(kept.clone())),
        ["createdAt", "expiresAt", "id", "isDownloadable", "mediaItemId", "mediaItemType", "slug", "updatedAt"]
    );
    assert_eq!(kept["isDownloadable"], false);
    let ids = |db: &dyn Db| -> Vec<String> {
        db.query("SELECT id FROM mediaItemShares ORDER BY id", vec![])
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(db), ["s1", "s3"]);
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    assert!(shares.find_by_slug("soon").is_none());
    assert_eq!(ids(db), ["s1"]);

    // a slug that was never shared has no download
    let a = r.send("public_share_download").await;
    assert_eq!(a.status, 404, "unknown slug: {}", a.body);
}
