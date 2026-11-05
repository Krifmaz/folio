//! The port of the original's server statistics (`server/controllers/StatsController.js`,
//! `server/utils/queries/adminStats.js`) and cache clearing (`server/controllers/CacheController.js`): the
//! year in review admins see (books and authors added, listening time, top authors, narrators and genres) and the
//! library's size, plus emptying the covers and items cache. Answers are the original's (tests/abs_stats.rs).

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::backups::purge_cache;
use crate::abs::models::{media_progress::js_json, user};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// The routes, under `/api/stats` and `/api/cache`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/stats/year/{year}", get(year))
        .route("/api/stats/server", get(server))
        .route("/api/cache/purge", post(purge_all))
        .route("/api/cache/items/purge", post(purge_items))
}

/// The original's check for the stats routes (`middleware`): admins only.
fn admin_only(me: &crate::abs::models::User, what: &str) -> Option<Response> {
    if user::is_admin_or_up(me) {
        return None;
    }
    tracing::error!("\"{}\" tried to {what} but isn't an admin", me.username.as_deref().unwrap_or_default());
    Some(send_status(StatusCode::FORBIDDEN))
}

/// A number column as a float (`0` when missing).
fn num(v: Option<&Value>) -> f64 {
    v.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).unwrap_or(0.0)
}

/// JavaScript's `Number(text)` for a whole year, `None` for NaN.
fn js_year(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    let lower = t.to_ascii_lowercase();
    for (prefix, radix) in [("0x", 16), ("0b", 2), ("0o", 8)] {
        if let Some(d) = lower.strip_prefix(prefix) {
            return u64::from_str_radix(d, radix).ok().map(|v| v as f64);
        }
    }
    if t.chars().any(|c| c.is_ascii_alphabetic() && !matches!(c, 'e' | 'E')) && t != "Infinity" {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| !n.is_nan())
}

/// `"<year>-01-01"` with the year written as JavaScript writes the number (`2026`, `2026.5`); stored times are
/// compared with it as text.
fn year_start(year: f64) -> String {
    format!("{}-01-01", js_json(Some(year)).to_string().trim_matches('"'))
}

/// Whether a name is an array index, which JavaScript objects list first, in number order.
fn index_key(s: &str) -> Option<u32> {
    if s == "0" || (!s.starts_with('0') && !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())) {
        s.parse::<u32>().ok().filter(|n| *n < u32::MAX)
    } else {
        None
    }
}

/// A tally kept in JavaScript object order: index-like names first (by number), the rest as first seen.
#[derive(Default)]
struct Tally(Vec<(String, f64)>);

impl Tally {
    /// Adds time to a name.
    fn add(&mut self, name: &str, time: f64) {
        match self.0.iter_mut().find(|(n, _)| n == name) {
            Some((_, t)) => *t += time,
            None => self.0.push((name.to_string(), time)),
        }
    }

    /// The three names with the most time, `{<key>: name, time}` with the time rounded.
    fn top(mut self, key: &str) -> Value {
        let mut indexed: Vec<(String, f64)> = self.0.iter().filter(|(n, _)| index_key(n).is_some()).cloned().collect();
        indexed.sort_by_key(|(n, _)| index_key(n));
        self.0.retain(|(n, _)| index_key(n).is_none());
        let mut all: Vec<(String, f64)> = indexed.into_iter().chain(self.0).map(|(n, t)| (n, (t + 0.5).floor())).collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1));
        Value::Array(all.into_iter().take(3).map(|(n, t)| json!({ key: n, "time": js_json(Some(t)) })).collect())
    }
}

/// The year in review (`getStatsForYear`).
pub fn stats_for_year(db: &dyn Db, year: f64) -> Result<Value> {
    let (from, to) = (year_start(year), year_start(year + 1.0));
    let books = db.query(
        "SELECT b.id, b.coverPath, b.duration, li.id AS itemId, li.size FROM books b
         JOIN libraryItems li ON li.mediaId = b.id WHERE b.createdAt >= ? AND b.createdAt < ? ORDER BY RANDOM()",
        vec![json!(from), json!(to)],
    )?;
    let mut with_covers: Vec<String> = Vec::new();
    let (mut added_size, mut added_duration) = (0.0, 0.0);
    for b in &books {
        let item = b.get("itemId").and_then(Value::as_str).unwrap_or_default().to_string();
        let cover = b.get("coverPath").and_then(Value::as_str).filter(|c| !c.is_empty());
        if cover.is_some_and(|c| std::path::Path::new(c).exists()) && !with_covers.contains(&item) && with_covers.len() < 25 {
            with_covers.push(item);
        }
        added_duration += num(b.get("duration"));
        added_size += num(b.get("size"));
    }
    let authors = db.one("SELECT COUNT(*) AS n FROM authors WHERE createdAt >= ? AND createdAt < ?", vec![json!(from), json!(to)])?;
    let sessions = db.query(
        "SELECT timeListening, mediaMetadata FROM playbackSessions WHERE createdAt >= ? AND createdAt < ?",
        vec![json!(from), json!(to)],
    )?;
    let (mut authors_time, mut narrators_time, mut genres_time) = (Tally::default(), Tally::default(), Tally::default());
    let mut listening = 0.0;
    for s in &sessions {
        let time = num(s.get("timeListening"));
        listening += time;
        let meta = match s.get("mediaMetadata") {
            Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
            Some(v) => v.clone(),
            None => Value::Null,
        };
        for a in meta["authors"].as_array().into_iter().flatten() {
            authors_time.add(&js_key(&a["name"]), time);
        }
        for n in meta["narrators"].as_array().into_iter().flatten() {
            narrators_time.add(&js_key(n), time);
        }
        for g in meta["genres"].as_array().into_iter().flatten() {
            let Some(g) = g.as_str().filter(|g| !g.is_empty()) else { continue };
            let lower = g.to_lowercase();
            if !lower.contains("audiobook") && !lower.contains("audio book") {
                genres_time.add(g, time);
            }
        }
    }
    let totals = db.one(
        "SELECT SUM(li.size) AS totalSize, SUM(b.duration) AS totalDuration, COUNT(*) AS totalItems FROM libraryItems li, books b
         WHERE b.id = li.mediaId AND li.mediaType = 'book' AND li.createdAt < ?",
        vec![json!(to.clone())],
    )?;
    let total = |k: &str| match totals.as_ref().and_then(|t| t.get(k)) {
        Some(Value::Number(n)) if n.as_f64() != Some(0.0) => js_json(n.as_f64()),
        _ => json!(0),
    };
    Ok(json!({
        "numListeningSessions": sessions.len(),
        "numBooksAdded": books.len(),
        "numAuthorsAdded": authors.as_ref().map(|r| num(r.get("n"))).map(|n| js_json(Some(n))).unwrap_or(json!(0)),
        "totalBooksAddedSize": js_json(Some(added_size)),
        "totalBooksAddedDuration": js_json(Some((added_duration + 0.5).floor())),
        "booksAddedWithCovers": with_covers,
        "totalBooksSize": total("totalSize"),
        "totalBooksDuration": total("totalDuration"),
        "totalListeningTime": js_json(Some(listening)),
        "numBooks": total("totalItems"),
        "topAuthors": authors_time.top("name"),
        "topNarrators": narrators_time.top("name"),
        "topGenres": genres_time.top("genre"),
    }))
}

/// A value used as an object key in JavaScript (`String(v)`).
fn js_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// `GET /api/stats/year/:year`.
async fn year(State(st): State<AppState>, me: AbsUser, Path(year): Path<String>) -> Response {
    if let Some(r) = admin_only(&me.user, "see the server statistics") {
        return r;
    }
    let y = js_year(&year).filter(|y| (2000.0..=9999.0).contains(y));
    let Some(y) = y else {
        tracing::error!("The year {year} isn't valid");
        return send_text(StatusCode::BAD_REQUEST, "Invalid year");
    };
    match stats_for_year(&st.abs, y) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// The library's size by kind (`getTotalSize` and `getNumAudioFiles`).
pub fn server_stats(db: &dyn Db) -> Result<Value> {
    let rows = db.query("SELECT mediaType, SUM(size) AS totalSize, COUNT(*) AS numItems FROM libraryItems GROUP BY mediaType", vec![])?;
    let of = |kind: &str, k: &str| {
        rows.iter().find(|r| r.get("mediaType").and_then(Value::as_str) == Some(kind)).map(|r| num(r.get(k))).unwrap_or(0.0)
    };
    let book_files = db.one("SELECT SUM(json_array_length(audioFiles)) AS n FROM books", vec![])?;
    let book_files = book_files.map(|r| num(r.get("n"))).unwrap_or(0.0);
    let episodes = db.one("SELECT COUNT(*) AS n FROM podcastEpisodes", vec![])?.map(|r| num(r.get("n"))).unwrap_or(0.0);
    let part = |size: f64, items: f64, files: f64| {
        let mut m = Map::new();
        m.insert("totalSize".into(), js_json(Some(size)));
        m.insert("numItems".into(), js_json(Some(items)));
        m.insert("numAudioFiles".into(), js_json(Some(files)));
        Value::Object(m)
    };
    let (bs, bn, ps, pn) = (of("book", "totalSize"), of("book", "numItems"), of("podcast", "totalSize"), of("podcast", "numItems"));
    Ok(json!({
        "books": part(bs, bn, book_files),
        "podcasts": part(ps, pn, episodes),
        "total": part(bs + ps, bn + pn, book_files + episodes),
    }))
}

/// `GET /api/stats/server`.
async fn server(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user, "see the server statistics") {
        return r;
    }
    match server_stats(&st.abs) {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /api/cache/purge`: empties the covers, images and items cache (`purgeAll`).
async fn purge_all(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    purge_cache(&st.abs_server.metadata_path);
    send_status(StatusCode::OK)
}

/// `POST /api/cache/items/purge`: empties the items cache (`purgeItems`).
async fn purge_items(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let cache = st.abs_server.metadata_path.join("cache");
    let items = cache.join("items");
    if items.exists() {
        if let Err(e) = std::fs::remove_dir_all(&items) {
            tracing::error!("The items cache {} can't be removed: {e}", items.display());
        }
    }
    for sub in ["covers", "images", "items"] {
        let _ = std::fs::create_dir_all(cache.join(sub));
    }
    send_status(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years_are_read_like_javascript_numbers() {
        assert_eq!(js_year("2026"), Some(2026.0));
        assert_eq!(js_year(" 2026 "), Some(2026.0));
        assert_eq!(js_year("0x7EA"), Some(2026.0));
        assert_eq!(js_year("2e3"), Some(2000.0));
        assert_eq!(js_year("abc"), None);
        assert_eq!(js_year("2026abc"), None);
    }

    #[test]
    fn the_top_three_are_kept_in_javascript_order() {
        let mut t = Tally::default();
        for (n, time) in [("b", 10.0), ("a", 10.0), ("10", 10.0), ("2", 10.0), ("c", 1.4)] {
            t.add(n, time);
        }
        let top = t.top("name");
        let names: Vec<&str> = top.as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["2", "10", "b"]);
    }
}
