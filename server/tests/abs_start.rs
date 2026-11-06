//! The Audiobookshelf port's clean-up at start (PARITY P6, `Server.cleanUserData` and
//! `CacheManager.ensureCachePaths`): progress of removed books and episodes goes, series that no longer exist leave
//! the users' "hidden from continue listening" lists, and the cache folders are made.

mod abs_replay;
mod common;
mod parity;

use abs_replay::set_up;
use kit_server_core::{Db, RowExt};
use serde_json::{json, Value};

#[tokio::test]
async fn start_removes_what_points_at_gone_things() {
    let (_r, st, _dir) = set_up("abs-start").await;
    let db = &st.abs;
    let now = "2026-11-05 18:00:00.000 +00:00";
    db.execute("INSERT INTO books (id, title, createdAt, updatedAt) VALUES ('b1', 'B', ?, ?)", vec![json!(now), json!(now)]).unwrap();
    db.execute("INSERT INTO podcasts (id, title, createdAt, updatedAt) VALUES ('p1', 'P', ?, ?)", vec![json!(now), json!(now)]).unwrap();
    db.execute(
        "INSERT INTO podcastEpisodes (id, title, podcastId, createdAt, updatedAt) VALUES ('e1', 'E', 'p1', ?, ?)",
        vec![json!(now), json!(now)],
    )
    .unwrap();
    db.execute("INSERT INTO series (id, name, createdAt, updatedAt) VALUES ('s1', 'S', ?, ?)", vec![json!(now), json!(now)]).unwrap();
    let root = db.one("SELECT id FROM users WHERE type = 'root'", vec![]).unwrap().unwrap().s("id");
    for (id, item, kind) in
        [("m1", "b1", "book"), ("m2", "e1", "podcastEpisode"), ("m3", "gone", "book"), ("m4", "gone-ep", "podcastEpisode")]
    {
        db.execute(
            "INSERT INTO mediaProgresses (id, mediaItemId, mediaItemType, userId, createdAt, updatedAt) VALUES (?, ?, ?, ?, ?, ?)",
            vec![json!(id), json!(item), json!(kind), json!(root), json!(now), json!(now)],
        )
        .unwrap();
    }
    let extra = json!({"seriesHideFromContinueListening": ["s1", "gone-series", "s1-gone"], "other": 1});
    db.execute("UPDATE users SET extraData = ? WHERE id = ?", vec![json!(extra.to_string()), json!(root)]).unwrap();

    server::abs::clean_user_data(db).unwrap();
    let left: Vec<String> = db.query("SELECT id FROM mediaProgresses ORDER BY id", vec![]).unwrap().iter().map(|r| r.s("id")).collect();
    assert_eq!(left, ["m1", "m2"]);
    let extra: Value =
        serde_json::from_str(&db.one("SELECT extraData FROM users WHERE id = ?", vec![json!(root)]).unwrap().unwrap().s("extraData"))
            .unwrap();
    assert_eq!(extra, json!({"seriesHideFromContinueListening": ["s1"], "other": 1}));
    // a second start finds nothing to do
    server::abs::clean_user_data(db).unwrap();
    assert_eq!(db.query("SELECT id FROM mediaProgresses", vec![]).unwrap().len(), 2);

    for sub in ["covers", "images", "items"] {
        assert!(st.abs_server.metadata_path.join("cache").join(sub).is_dir(), "{sub}");
    }
}
