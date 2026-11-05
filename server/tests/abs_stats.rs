//! The Audiobookshelf port's server statistics and cache clearing (PARITY P6). Replays the recorded requests and
//! compares every answer with the pinned original's; the recording's library isn't in the test server, so its
//! counts are checked by shape there and by value against a library made here.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{set_up, Replay};
use kit_server_core::Db;
use serde_json::{json, Value};

/// The counts that depend on the recording's library.
const COUNTS: [&str; 10] = [
    "numAuthorsAdded",
    "numBooks",
    "numBooksAdded",
    "totalBooksAddedDuration",
    "totalBooksAddedSize",
    "totalBooksDuration",
    "totalBooksSize",
    "numAudioFiles",
    "numItems",
    "totalSize",
];

/// Adds the guest account the recording used for the "not an admin" answers.
async fn with_guest(r: &mut Replay) {
    for name in ["p6s_guest_create", "p6s_guest_login"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn stats_and_cache_answer_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-stats").await;
    r.m.volatile.extend(COUNTS.map(String::from));
    with_guest(&mut r).await;
    for name in [
        "admin_stats_year",
        "admin_stats_year_empty",
        "admin_stats_year_fraction",
        "admin_stats_year_hex",
        "admin_stats_year_early",
        "admin_stats_year_text",
        "admin_stats_year_guest",
        "admin_stats_server",
        "admin_stats_server_guest",
        "cache_purge",
        "cache_items_purge",
        "cache_purge_guest",
        "cache_items_purge_guest",
        "p6s_guest_delete",
    ] {
        r.like(name).await;
    }
    let cache = st.abs_server.metadata_path.join("cache");
    for sub in ["covers", "images", "items"] {
        assert!(cache.join(sub).is_dir(), "{sub}");
    }
}

#[tokio::test]
async fn the_year_in_review_counts_like_the_original() {
    let (_r, st, dir) = set_up("abs-stats-values").await;
    let db = &st.abs;
    let in_year = "2026-03-01 10:00:00.000 +00:00";
    let before = "2025-06-01 10:00:00.000 +00:00";
    let cover = dir.join("cover.jpg");
    std::fs::write(&cover, b"jpg").unwrap();
    for (id, created, duration, size, cover) in
        [("b1", in_year, 60.4, 1000, Some(cover.display().to_string())), ("b2", in_year, 30.0, 500, None), ("b3", before, 10.0, 7, None)]
    {
        db.execute(
            "INSERT INTO books (id, title, duration, coverPath, audioFiles, createdAt, updatedAt) VALUES (?, ?, ?, ?, '[{}, {}]', ?, ?)",
            vec![json!(id), json!(id), json!(duration), json!(cover), json!(created), json!(created)],
        )
        .unwrap();
        db.execute(
            "INSERT INTO libraryItems (id, mediaId, mediaType, size, libraryId, createdAt, updatedAt) VALUES (?, ?, 'book', ?, ?, ?, ?)",
            vec![
                json!(format!("li-{id}")),
                json!(id),
                json!(size),
                json!("2b3f6c6f-0488-43dc-83b8-7fd501ae4620"),
                json!(created),
                json!(created),
            ],
        )
        .unwrap();
    }
    db.execute("INSERT INTO authors (id, name, createdAt, updatedAt) VALUES ('a1', 'A', ?, ?)", vec![json!(in_year), json!(in_year)])
        .unwrap();
    for (id, time, meta) in [
        ("s1", 100, json!({"authors": [{"name": "Ann"}], "narrators": ["Ned"], "genres": ["Fantasy", "Audiobook"]})),
        ("s2", 50, json!({"authors": [{"name": "Bob"}, {"name": "Ann"}], "narrators": [], "genres": ["Sci-Fi"]})),
    ] {
        db.execute(
            "INSERT INTO playbackSessions (id, timeListening, mediaMetadata, createdAt, updatedAt) VALUES (?, ?, ?, ?, ?)",
            vec![json!(id), json!(time), json!(meta.to_string()), json!(in_year), json!(in_year)],
        )
        .unwrap();
    }
    let v = server::abs::stats::stats_for_year(db, 2026.0).unwrap();
    assert_eq!(v["numBooksAdded"], 2);
    assert_eq!(v["numAuthorsAdded"], 1);
    assert_eq!(v["totalBooksAddedSize"], 1500);
    assert_eq!(v["totalBooksAddedDuration"], 90);
    assert_eq!(v["booksAddedWithCovers"], json!(["li-b1"]));
    assert_eq!(v["totalBooksSize"], 1507);
    assert_eq!(v["numBooks"], 3);
    assert_eq!(v["numListeningSessions"], 2);
    assert_eq!(v["totalListeningTime"], 150);
    assert_eq!(v["topAuthors"], json!([{"name": "Ann", "time": 150}, {"name": "Bob", "time": 50}]));
    assert_eq!(v["topNarrators"], json!([{"name": "Ned", "time": 100}]));
    assert_eq!(v["topGenres"], json!([{"genre": "Fantasy", "time": 100}, {"genre": "Sci-Fi", "time": 50}]));
    let duration = v["totalBooksDuration"].as_f64().unwrap();
    assert!((duration - 100.4).abs() < 1e-9, "{duration}");

    let s = server::abs::stats::server_stats(db).unwrap();
    let want: Value = json!({"totalSize": 1507, "numItems": 3, "numAudioFiles": 6});
    assert_eq!(s["books"], want);
    assert_eq!(s["podcasts"], json!({"totalSize": 0, "numItems": 0, "numAudioFiles": 0}));
    assert_eq!(s["total"], want);
}
