//! Playing and places through the Audiobookshelf API: playback sessions (start, sync, close), places played
//! offline, progress set by hand, bookmarks, and the listening numbers. Every place is kept as Folio's own
//! progress (`playback::store`, the last writer wins), with the device `abs:<app>`.

use super::{find, shape, Abs};
use crate::compat::model::Item;
use crate::compat::shelf;
use crate::playback::store::{self as playback, Progress};
use crate::AppState;
use axum::extract::{Path, State};
use axum::Json;
use kit_server_core::{err, p, Db, Result, RowExt};
use serde::Deserialize;
use serde_json::{json, Value};

/// The app a request names, for the progress' device (`abs:<client name>`).
fn device_of(info: &Value) -> String {
    let name = ["clientName", "deviceName", "model", "manufacturer"]
        .iter()
        .find_map(|k| info.get(*k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
        .unwrap_or("Audiobookshelf app");
    format!("abs:{}", name.chars().take(80).collect::<String>())
}

/// Keeps an audiobook place `secs` seconds in (or an ebook place), unless a newer one is kept; the one that
/// stands.
fn keep(st: &AppState, a: &Abs, it: &Item, secs: f64, finished: Option<bool>, device: &str, at: f64) -> Result<Progress> {
    let kept = playback::progress(&st.db, a.who.id, it.id)?;
    let duration = it.duration();
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let fraction = if duration > 0.0 { (secs / duration).clamp(0.0, 1.0) } else { 0.0 };
    let finished = finished.unwrap_or_else(|| kept.as_ref().is_some_and(|k| k.finished));
    let place = Progress {
        edition_id: it.id,
        position_ms: (secs * 1000.0) as i64,
        locator: kept.map(|k| k.locator).unwrap_or(Value::Null),
        fraction: if finished { 1.0 } else { fraction },
        finished,
        device: device.to_string(),
        updated_at: at,
    };
    playback::save_progress(&st.db, a.who.id, it.id, place)
}

/// `POST /api/items/{id}/play` body.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    /// the phone or app
    #[serde(default)]
    pub device_info: Value,
    /// the app's player
    #[serde(default)]
    pub media_player: String,
}

/// A playback session as it starts: its id, the place, when, and the app's device and player.
struct Started<'a> {
    /// the session's id
    sid: &'a str,
    /// the place, in seconds
    current: f64,
    /// when it started, in seconds since 1970
    started: f64,
    /// the phone or app
    info: &'a Value,
    /// the app's player
    player: &'a str,
}

/// A playback session as the apps read it.
fn session(st: &AppState, a: &Abs, it: &Item, s: Started) -> Value {
    let Started { sid, current, started, info, player } = s;
    let listened = 0.0;
    let root = shelf::library_dir(st);
    let day = crate::compat::opds::rfc3339(started);
    let weekday =
        ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"][((started / 86400.0) as i64).rem_euclid(7) as usize];
    json!({
        "id": sid,
        "userId": a.who.id.to_string(),
        "libraryId": shape::library_of(it),
        "libraryItemId": shape::item_id(it.id),
        "bookId": format!("bk_{}", it.id),
        "episodeId": null,
        "mediaType": "book",
        "mediaMetadata": shape::item(it, &root, false)["media"]["metadata"].clone(),
        "chapters": shape::chapters(it),
        "displayTitle": it.meta.title,
        "displayAuthor": it.authors(),
        "coverPath": it.cover.as_ref().map(|(p, _)| json!(p.to_string_lossy())).unwrap_or(Value::Null),
        "duration": it.duration(),
        "playMethod": 0,
        "mediaPlayer": player,
        "deviceInfo": info,
        "serverVersion": shape::SERVER_VERSION,
        "date": &day[..10],
        "dayOfWeek": weekday,
        "timeListening": listened,
        "startTime": current,
        "currentTime": current,
        "startedAt": shape::ms(started),
        "updatedAt": shape::ms(crate::compat::now()),
        "audioTracks": shape::tracks(it, &root),
        "libraryItem": shape::item(it, &root, false)
    })
}

/// `POST /api/items/{id}/play`: a playback session from the person's place.
pub async fn start(State(st): State<AppState>, a: Abs, Path(id): Path<String>, body: Option<Json<PlayRequest>>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    if !it.is_audio() {
        return Err(err(400, "This is an ebook, so it opens in the reader instead of the player."));
    }
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let place = playback::progress(&st.db, a.who.id, it.id)?;
    let current = place.filter(|p| !p.finished).map(|p| p.position_ms as f64 / 1000.0).unwrap_or(0.0);
    let sid = format!("ps_{}", crate::auth::random_token(12));
    let now = crate::compat::now();
    st.db.execute(
        "INSERT INTO compat_abs_sessions (id, user_id, edition_id, device, started_at, updated_at, position, listened, closed)
         VALUES (?,?,?,?,?,?,?,0,0)",
        p![sid, a.who.id, it.id, device_of(&b.device_info), now, now, current],
    )?;
    Ok(Json(session(&st, &a, &it, Started { sid: &sid, current, started: now, info: &b.device_info, player: &b.media_player })))
}

/// `POST /api/session/{id}/sync` and `/close` body.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncBody {
    /// where the player is, in seconds
    #[serde(default)]
    pub current_time: Option<f64>,
    /// seconds listened since the last sync
    #[serde(default)]
    pub time_listened: Option<f64>,
}

/// Moves a session on: its place and listened time, and the person's progress.
fn advance(st: &AppState, a: &Abs, sid: &str, b: &SyncBody, close: bool) -> Result<()> {
    let row = st
        .db
        .one("SELECT * FROM compat_abs_sessions WHERE id=? AND user_id=?", p![sid, a.who.id])?
        .ok_or_else(|| err(404, "That playing session has ended. Press play again."))?;
    let it = shelf::item(st, &a.who, row.i("edition_id"))?;
    let at = crate::compat::now();
    let listened = b.time_listened.filter(|t| t.is_finite() && *t > 0.0).unwrap_or(0.0).min(3600.0);
    let position = b.current_time.unwrap_or(row.f("position"));
    st.db.execute(
        "UPDATE compat_abs_sessions SET position=?, listened=listened+?, updated_at=?, closed=? WHERE id=?",
        p![position, listened, at, close || row.b("closed"), sid],
    )?;
    if b.current_time.is_some() {
        let finished = (it.duration() > 0.0 && position >= it.duration() - 1.0).then_some(true);
        keep(st, a, &it, position, finished, &row.s("device"), at)?;
    }
    Ok(())
}

/// `POST /api/session/{id}/sync`.
pub async fn sync(State(st): State<AppState>, a: Abs, Path(sid): Path<String>, body: Option<Json<SyncBody>>) -> Result<Json<Value>> {
    advance(&st, &a, &sid, &body.map(|Json(b)| b).unwrap_or_default(), false)?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/session/{id}/close`.
pub async fn close(State(st): State<AppState>, a: Abs, Path(sid): Path<String>, body: Option<Json<SyncBody>>) -> Result<Json<Value>> {
    advance(&st, &a, &sid, &body.map(|Json(b)| b).unwrap_or_default(), true)?;
    Ok(Json(json!({ "success": true })))
}

/// A session played offline, as an app sends it later.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalSession {
    /// the app's id for the session
    #[serde(default)]
    pub id: String,
    /// the item
    #[serde(default)]
    pub library_item_id: String,
    /// where it stopped, in seconds
    #[serde(default)]
    pub current_time: f64,
    /// seconds listened
    #[serde(default)]
    pub time_listening: f64,
    /// when it last moved, in milliseconds since 1970
    #[serde(default)]
    pub updated_at: f64,
    /// the phone or app
    #[serde(default)]
    pub device_info: Value,
}

/// Keeps a session played offline as the person's place (unless a newer place is kept). Whether it was.
fn local_one(st: &AppState, a: &Abs, s: &LocalSession) -> Result<bool> {
    let it = find(st, a, &s.library_item_id)?;
    let at = if s.updated_at > 0.0 { s.updated_at / 1000.0 } else { crate::compat::now() };
    let kept = keep(st, a, &it, s.current_time, None, &device_of(&s.device_info), at)?;
    let sid = if s.id.is_empty() { format!("ps_{}", crate::auth::random_token(12)) } else { s.id.chars().take(80).collect() };
    st.db.execute(
        "INSERT INTO compat_abs_sessions (id, user_id, edition_id, device, started_at, updated_at, position, listened, closed)
         VALUES (?,?,?,?,?,?,?,?,1)
         ON CONFLICT (id) DO UPDATE SET position=excluded.position, listened=excluded.listened, updated_at=excluded.updated_at",
        p![sid, a.who.id, it.id, device_of(&s.device_info), at, at, s.current_time, s.time_listening.max(0.0)],
    )?;
    Ok((kept.updated_at - at).abs() < 0.001)
}

/// `POST /api/session/local`: one session played offline.
pub async fn local(State(st): State<AppState>, a: Abs, Json(s): Json<LocalSession>) -> Result<Json<Value>> {
    local_one(&st, &a, &s)?;
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/session/local-all` body.
#[derive(Debug, Deserialize, Default)]
pub struct LocalAll {
    /// the sessions
    #[serde(default)]
    pub sessions: Vec<LocalSession>,
}

/// `POST /api/session/local-all`: every session played offline; how each went.
pub async fn local_all(State(st): State<AppState>, a: Abs, Json(b): Json<LocalAll>) -> Result<Json<Value>> {
    let results: Vec<Value> = b
        .sessions
        .iter()
        .map(|s| match local_one(&st, &a, s) {
            Ok(synced) => json!({ "id": s.id, "success": true, "progressSynced": synced }),
            Err(e) => json!({ "id": s.id, "success": false, "error": e.detail }),
        })
        .collect();
    Ok(Json(json!({ "results": results })))
}

/// A place an app sets by hand (`PATCH /api/me/progress/{id}`).
#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProgressBody {
    /// the item (in a batch)
    #[serde(default)]
    pub library_item_id: String,
    /// where in the audiobook, in seconds
    #[serde(default)]
    pub current_time: Option<f64>,
    /// how far, 0 to 1
    #[serde(default)]
    pub progress: Option<f64>,
    /// whether it is finished
    #[serde(default)]
    pub is_finished: Option<bool>,
    /// where in the ebook (an EPUB CFI from epub.js)
    #[serde(default)]
    pub ebook_location: Option<String>,
    /// how far through the ebook, 0 to 1
    #[serde(default)]
    pub ebook_progress: Option<f64>,
    /// when, in milliseconds since 1970
    #[serde(default)]
    pub last_update: Option<f64>,
}

/// Keeps a place set by hand; the one that stands, in Audiobookshelf's shape.
fn set_one(st: &AppState, a: &Abs, it: &Item, b: &ProgressBody) -> Result<Value> {
    let at = b.last_update.filter(|t| *t > 0.0).map(|t| t / 1000.0).unwrap_or_else(crate::compat::now);
    let kept = playback::progress(&st.db, a.who.id, it.id)?.unwrap_or_default();
    let mut place = Progress { edition_id: it.id, device: "abs:Audiobookshelf app".into(), updated_at: at, ..kept.clone() };
    if it.is_audio() {
        if let Some(t) = b.current_time {
            place.position_ms = (t.max(0.0) * 1000.0) as i64;
            let d = it.duration();
            place.fraction = b.progress.unwrap_or(if d > 0.0 { t / d } else { 0.0 });
        } else if let Some(f) = b.progress {
            place.fraction = f;
            place.position_ms = (f.clamp(0.0, 1.0) * it.duration() * 1000.0) as i64;
        }
    } else {
        if let Some(cfi) = &b.ebook_location {
            let far = b.ebook_progress.or(b.progress).unwrap_or(kept.fraction);
            place.locator = json!({ "cfi": cfi, "total_progression": far });
        }
        if let Some(f) = b.ebook_progress.or(b.progress) {
            place.fraction = f;
        }
    }
    if let Some(done) = b.is_finished {
        place.finished = done;
        if done {
            place.fraction = 1.0;
        }
    }
    let stands = playback::save_progress(&st.db, a.who.id, it.id, place)?;
    Ok(shape::progress(a.who.id, it, &stands))
}

/// `GET /api/me/progress/{id}`.
pub async fn get_progress(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    let p = playback::progress(&st.db, a.who.id, it.id)?.ok_or_else(|| err(404, "You haven't started this book yet."))?;
    Ok(Json(shape::progress(a.who.id, &it, &p)))
}

/// `PATCH /api/me/progress/{id}`.
pub async fn set_progress(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Json(b): Json<ProgressBody>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    Ok(Json(set_one(&st, &a, &it, &b)?))
}

/// `DELETE /api/me/progress/{id}`: start the book again.
pub async fn clear_progress(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    playback::clear_progress(&st.db, a.who.id, it.id)?;
    Ok(Json(json!({ "success": true })))
}

/// `PATCH /api/me/progress/batch/update`: several places at once.
pub async fn batch_progress(State(st): State<AppState>, a: Abs, Json(list): Json<Vec<ProgressBody>>) -> Result<Json<Value>> {
    for b in &list {
        let it = find(&st, &a, &b.library_item_id)?;
        set_one(&st, &a, &it, b)?;
    }
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/me/sync-local-progress` body: the places an older app kept offline.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalProgress {
    /// the places
    #[serde(default)]
    pub local_media_progress: Vec<Value>,
}

/// `POST /api/me/sync-local-progress`: keeps each newer place; answers the server's places for the others.
pub async fn sync_local_progress(State(st): State<AppState>, a: Abs, Json(b): Json<LocalProgress>) -> Result<Json<Value>> {
    let mut updates = 0;
    let mut back = Vec::new();
    for v in &b.local_media_progress {
        let id = v.get("libraryItemId").and_then(Value::as_str).unwrap_or("");
        let Ok(it) = find(&st, &a, id) else { continue };
        let body = ProgressBody {
            library_item_id: id.to_string(),
            current_time: v.get("currentTime").and_then(Value::as_f64),
            progress: v.get("progress").and_then(Value::as_f64),
            is_finished: v.get("isFinished").and_then(Value::as_bool),
            ebook_location: v.get("ebookLocation").and_then(Value::as_str).map(str::to_string),
            ebook_progress: v.get("ebookProgress").and_then(Value::as_f64),
            last_update: v.get("lastUpdate").and_then(Value::as_f64),
        };
        let stands = set_one(&st, &a, &it, &body)?;
        if stands["lastUpdate"].as_f64() == body.last_update.map(|t| t.trunc()) {
            updates += 1;
        } else {
            back.push(stands);
        }
    }
    Ok(Json(json!({ "numServerProgressUpdates": updates, "localProgressUpdates": back })))
}

/// `POST|PATCH /api/me/item/{id}/bookmark` body.
#[derive(Debug, Deserialize, Default)]
pub struct BookmarkBody {
    /// where, in seconds
    #[serde(default)]
    pub time: f64,
    /// the person's note
    #[serde(default)]
    pub title: String,
}

/// `POST /api/me/item/{id}/bookmark`.
pub async fn add_bookmark(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Json(b): Json<BookmarkBody>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    let bm = playback::Bookmark { edition_id: it.id, position_ms: (b.time.max(0.0) * 1000.0) as i64, note: b.title, ..Default::default() };
    Ok(Json(shape::bookmark(&playback::add_bookmark(&st.db, a.who.id, it.id, bm)?)))
}

/// The bookmark of the person's in `it` at `secs` seconds (to the millisecond).
fn bookmark_at(st: &AppState, a: &Abs, it: &Item, secs: f64) -> Result<playback::Bookmark> {
    let at = (secs * 1000.0) as i64;
    playback::bookmarks(&st.db, a.who.id, it.id)?
        .into_iter()
        .find(|b| (b.position_ms - at).abs() <= 1)
        .ok_or_else(|| err(404, "There is no bookmark at that time any more."))
}

/// `PATCH /api/me/item/{id}/bookmark`: change the note of the bookmark at `time`.
pub async fn edit_bookmark(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Json(b): Json<BookmarkBody>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    let bm = bookmark_at(&st, &a, &it, b.time)?;
    Ok(Json(shape::bookmark(&playback::edit_bookmark(&st.db, a.who.id, bm.id, &b.title)?)))
}

/// `DELETE /api/me/item/{id}/bookmark/{time}`.
pub async fn delete_bookmark(State(st): State<AppState>, a: Abs, Path((id, time)): Path<(String, String)>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    let secs: f64 = time.parse().map_err(|_| err(400, "The bookmark's time isn't a number."))?;
    if let Ok(bm) = bookmark_at(&st, &a, &it, secs) {
        playback::delete_bookmark(&st.db, a.who.id, bm.id)?;
    }
    Ok(Json(json!({ "success": true })))
}

/// `GET /api/me/listening-stats`: time listened in total, by day and by book.
pub async fn listening_stats(State(st): State<AppState>, a: Abs) -> Result<Json<Value>> {
    let rows = st.db.query("SELECT edition_id, started_at, listened FROM compat_abs_sessions WHERE user_id=?", p![a.who.id])?;
    let mut total = 0.0;
    let mut days: std::collections::BTreeMap<String, f64> = Default::default();
    let mut items: std::collections::BTreeMap<String, f64> = Default::default();
    for r in &rows {
        let l = r.f("listened");
        total += l;
        *days.entry(crate::compat::opds::rfc3339(r.f("started_at"))[..10].to_string()).or_default() += l;
        *items.entry(shape::item_id(r.i("edition_id"))).or_default() += l;
    }
    let today = crate::compat::opds::rfc3339(crate::compat::now())[..10].to_string();
    let items: serde_json::Map<String, Value> =
        items.into_iter().map(|(id, l)| (id.clone(), json!({ "id": id, "timeListening": l, "mediaMetadata": {} }))).collect();
    Ok(Json(json!({
        "totalTime": total,
        "items": items,
        "days": days,
        "dayOfWeek": {},
        "today": days.get(&today).copied().unwrap_or(0.0),
        "recentSessions": []
    })))
}

/// `GET /api/me/listening-sessions`: none kept in detail.
pub async fn listening_sessions(_a: Abs) -> Json<Value> {
    Json(json!({ "total": 0, "numPages": 0, "page": 0, "itemsPerPage": 10, "sessions": [] }))
}
