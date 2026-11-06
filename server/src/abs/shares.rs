//! The port of the original's shares (`controllers/ShareController.js`, `managers/ShareManager.js` and
//! `models/MediaItemShare.js`): an admin shares a book under a public link (`/share/<slug>`), with or without an
//! end date and with or without a download. Anyone with the link listens through a share session tied to a
//! cookie (`share_session_id`) that lasts 30 days. Shares that end are removed on time; share sessions untouched
//! for a day are closed every night. Answers are the original's (tests/abs_shares.rs).

use crate::abs::auth::routes::{body_fields, Peer};
use crate::abs::auth::{client_ip, send_status, AbsUser};
use crate::abs::models::{self, book, user, Library, LibraryItem};
use crate::abs::playback::device::{self, DeviceInfo};
use crate::abs::playback::session::{self, Session};
use crate::abs::progress::send_text;
use crate::abs::settings::{js_number, truthy};
use crate::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tower::ServiceExt;

/// The routes: the admins' `/api/share/mediaitem` and the public `/public/share/:slug`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/share/mediaitem", post(create))
        .route("/api/share/mediaitem/{id}", delete(remove))
        .route("/public/share/{slug}", get(by_slug))
        .route("/public/share/{slug}/track/{index}", get(track))
        .route("/public/share/{slug}/cover", get(cover))
        .route("/public/share/{slug}/download", get(download))
        .route("/public/share/{slug}/progress", axum::routing::patch(progress))
}

/// How long the share session cookie lasts: 30 days, in seconds.
const COOKIE_MAX_AGE: i64 = 60 * 60 * 24 * 30;

/// A share that is open, as the original's `ShareManager` keeps it.
struct Open {
    /// The share's id.
    id: String,
    /// The share as the model's `toJSON` gives it.
    share: Map<String, Value>,
    /// The timer that removes it when it ends, if it has an end.
    timer: Option<tokio::task::AbortHandle>,
}

/// A listening session through a share: a playback session without a user, tied to the visitor's cookie.
#[derive(Clone)]
pub struct ShareSession {
    /// The playback session.
    pub session: Session,
    /// The visitor's cookie (`shareSessionId`).
    pub share_session_id: String,
    /// The share it plays (`mediaItemShareId`).
    pub media_item_share_id: String,
    /// The library's cover shape (`coverAspectRatio`), for the share page.
    pub cover_aspect_ratio: Value,
}

impl ShareSession {
    /// The session as the share page gets it (`toJSONForClient()` without a library item, with the cover shape).
    fn to_json_for_client(&self) -> Value {
        let mut v = self.session.to_json_for_client(None);
        if let (Value::Object(m), false) = (&mut v, self.cover_aspect_ratio.is_null()) {
            m.insert("coverAspectRatio".into(), self.cover_aspect_ratio.clone());
        }
        v
    }
}

/// The open shares and their listening sessions (`ShareManager`). Shared by every clone of the server state.
#[derive(Default)]
pub struct Manager {
    /// The open shares (`openMediaItemShares`).
    open: Mutex<Vec<Open>>,
    /// The open share sessions (`openSharePlaybackSessions`).
    sessions: Mutex<Vec<ShareSession>>,
}

impl Manager {
    /// Runs `f` on the open shares.
    fn with_open<R>(&self, f: impl FnOnce(&mut Vec<Open>) -> R) -> R {
        f(&mut self.open.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// Runs `f` on the open share sessions.
    fn with_sessions<R>(&self, f: impl FnOnce(&mut Vec<ShareSession>) -> R) -> R {
        f(&mut self.sessions.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The open share with this slug, as the share page gets it (`findBySlug`): without `pash`, `userId` and
    /// `extraData`.
    pub fn find_by_slug(&self, slug: &str) -> Option<Map<String, Value>> {
        self.with_open(|list| list.iter().find(|o| o.share.get("slug").and_then(Value::as_str) == Some(slug)).map(|o| for_client(&o.share)))
    }

    /// The open share of a book or episode, as the item pages get it (`findByMediaItemId`).
    pub fn find_by_media_item_id(&self, media_item_id: &str) -> Option<Map<String, Value>> {
        self.with_open(|list| {
            list.iter().find(|o| o.share.get("mediaItemId").and_then(Value::as_str) == Some(media_item_id)).map(|o| for_client(&o.share))
        })
    }

    /// The share session of a visitor's cookie (`findPlaybackSessionBySessionId`).
    fn session_of(&self, cookie: &str) -> Option<ShareSession> {
        self.with_sessions(|list| list.iter().find(|s| s.share_session_id == cookie).cloned())
    }

    /// Every open share session as JSON (`openSharePlaybackSessions.map(s => s.toJSON())`), for the admins' list
    /// of open sessions.
    pub fn sessions_json(&self) -> Vec<Value> {
        self.with_sessions(|list| list.iter().map(|s| s.session.to_json()).collect())
    }

    /// Closes a share session (`closeSharePlaybackSession`).
    fn close_session(&self, id: &str) {
        self.with_sessions(|list| {
            if let Some(s) = list.iter().find(|s| s.session.id == id) {
                tracing::info!("Closing share playback session \"{}\"", s.session.display_title.as_str().unwrap_or_default());
            }
            list.retain(|s| s.session.id != id)
        });
    }

    /// Closes the share sessions untouched for a day (`closeStaleOpenShareSessions`).
    pub fn close_stale_sessions(&self) {
        let cutoff = models::now_ms() - 1000 * 60 * 60 * 24;
        self.with_sessions(|list| {
            list.retain(|s| {
                let stale = s.session.updated_at.as_f64().is_some_and(|u| (u as i64) < cutoff);
                if stale {
                    tracing::info!(
                        "Closing stale session \"{}\" ({}) last updated at {}",
                        s.session.display_title.as_str().unwrap_or_default(),
                        s.session.id,
                        models::date_text(s.session.updated_at.as_f64().unwrap_or(0.0) as i64)
                    );
                }
                !stale
            })
        });
    }
}

/// A share without what visitors mustn't see (`pash`, `userId`, `extraData`).
fn for_client(share: &Map<String, Value>) -> Map<String, Value> {
    let mut m = share.clone();
    for k in ["pash", "userId", "extraData"] {
        m.remove(k);
    }
    m
}

/// A stored share as the model's `toJSON` gives it: dates as ISO text, `isDownloadable` as true or false.
fn share_json(row: &Map<String, Value>) -> Map<String, Value> {
    let text = |k: &str| row.get(k).and_then(Value::as_str);
    let mut m = Map::new();
    for k in ["id", "mediaItemId", "mediaItemType", "slug", "pash"] {
        m.insert(k.into(), row.get(k).cloned().unwrap_or(Value::Null));
    }
    m.insert("expiresAt".into(), models::date_iso(text("expiresAt")));
    let extra = match row.get("extraData") {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(v) => v.clone(),
        None => Value::Null,
    };
    m.insert("extraData".into(), extra);
    let downloadable = match row.get("isDownloadable") {
        Some(Value::Null) | None => Value::Null,
        Some(v) => Value::Bool(truthy(Some(v))),
    };
    m.insert("isDownloadable".into(), downloadable);
    m.insert("createdAt".into(), models::date_iso(text("createdAt")));
    m.insert("updatedAt".into(), models::date_iso(text("updatedAt")));
    m.insert("userId".into(), row.get("userId").cloned().unwrap_or(Value::Null));
    m
}

/// A share as the admins' apps get it (`toJSONForClient`).
fn client_json(share: &Map<String, Value>) -> Value {
    let get = |k: &str| share.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "id": get("id"),
        "mediaItemId": get("mediaItemId"),
        "mediaItemType": get("mediaItemType"),
        "slug": get("slug"),
        "expiresAt": get("expiresAt"),
        "createdAt": get("createdAt"),
        "updatedAt": get("updatedAt"),
        "isDownloadable": get("isDownloadable"),
    })
}

/// When a share ends, in milliseconds, if it has an end.
fn expires_ms(share: &Map<String, Value>) -> Option<i64> {
    share.get("expiresAt").and_then(Value::as_str).and_then(models::date_ms)
}

/// Loads the stored shares at start (`ShareManager.init`): ended ones are removed, the others opened. Also starts
/// the nightly closing of stale share sessions (00:30 UTC, like the original's cleanup schedule).
pub fn init(st: &AppState) {
    match st.abs.query("SELECT * FROM mediaItemShares", vec![]) {
        Ok(rows) => {
            for row in rows {
                let share = share_json(&row);
                let id = share.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                match expires_ms(&share) {
                    Some(end) if end < models::now_ms() => {
                        tracing::info!("Removing expired media item share \"{id}\"");
                        destroy(st, &id);
                    }
                    Some(_) => schedule(st, share),
                    None => {
                        tracing::info!("Loaded permanent media item share \"{id}\"");
                        st.abs_server.shares.with_open(|list| list.push(Open { id, share, timer: None }));
                    }
                }
            }
        }
        Err(e) => tracing::error!("The shares can't be read: {}", e.detail),
    }
    let st = st.clone();
    tokio::spawn(async move {
        let schedule = crate::abs::cron::convert("30 0 * * *").unwrap_or_default();
        let mut last = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let now = models::now_ms();
            let minute = now / 60_000;
            if minute != last && crate::abs::cron::matches(&schedule, crate::abs::cron::time_parts(minute * 60_000)) {
                last = minute;
                st.abs_server.shares.close_stale_sessions();
            }
        }
    });
}

/// Opens a share that ends, with a timer that removes it then (`scheduleMediaItemShare`); one already ended is
/// removed now.
fn schedule(st: &AppState, share: Map<String, Value>) {
    let id = share.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let left = expires_ms(&share).unwrap_or(0) - models::now_ms();
    if left <= 0 {
        tracing::warn!("Attempted to schedule expired media item share \"{id}\"");
        destroy(st, &id);
        return;
    }
    let later = st.clone();
    let timer_id = id.clone();
    let timer = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(left as u64)).await;
        tracing::info!("Removing expired media item share \"{timer_id}\"");
        remove_share(&later, &timer_id);
    })
    .abort_handle();
    st.abs_server.shares.with_open(|list| list.push(Open { id: id.clone(), share, timer: Some(timer) }));
    tracing::info!("Scheduled media item share \"{id}\" to expire in {}", elapsed_pretty(left as f64 / 1000.0));
}

/// A length of time as the original writes it in its log (`elapsedPretty`): `45 sec`, `12 min`, `2 d 3 hr 5 min`.
fn elapsed_pretty(seconds: f64) -> String {
    if seconds > 0.0 && seconds < 1.0 {
        return format!("{} ms", (seconds * 1000.0).floor());
    }
    if seconds < 60.0 {
        return format!("{} sec", seconds.floor());
    }
    let mut minutes = (seconds / 60.0).floor() as i64;
    if minutes < 70 {
        return format!("{minutes} min");
    }
    let mut hours = minutes / 60;
    minutes -= hours * 60;
    let days = hours / 24;
    hours -= days * 24;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days} d"));
    }
    if hours > 0 || (days > 0 && minutes > 0) {
        parts.push(format!("{hours} hr"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes} min"));
    }
    parts.join(" ")
}

/// Deletes a share from the database (`destroyMediaItemShare`).
fn destroy(st: &AppState, id: &str) {
    if let Err(e) = st.abs.execute("DELETE FROM mediaItemShares WHERE id = ?", vec![json!(id)]) {
        tracing::error!("The share {id} can't be removed: {}", e.detail);
    }
}

/// Closes an open share, its sessions and its database row, and tells the admins (`removeMediaItemShare`).
fn remove_share(st: &AppState, id: &str) {
    let Some(open) = st.abs_server.shares.with_open(|list| list.iter().position(|o| o.id == id).map(|i| list.remove(i))) else {
        return;
    };
    if let Some(t) = &open.timer {
        t.abort();
    }
    st.abs_server.shares.with_sessions(|list| list.retain(|s| s.media_item_share_id != id));
    destroy(st, id);
    st.sockets.admin_emitter("share_closed", &Value::Object(for_client(&open.share)));
}

/// `POST /api/share/mediaitem` (admins only): shares a book or episode under a slug (`createMediaItemShare`).
async fn create(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("Non-admin user \"{}\" attempted to create item share", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let b = body_fields(&headers, &body);
    let slug = b.get("slug").and_then(Value::as_str).filter(|s| !s.trim().is_empty());
    let kind = b.get("mediaItemType").and_then(Value::as_str);
    let item_id = b.get("mediaItemId").and_then(Value::as_str);
    let (Some(slug), Some(kind), Some(item_id)) = (slug, kind, item_id) else {
        return send_text(StatusCode::BAD_REQUEST, "Missing or invalid required fields");
    };
    let expires_raw = b.get("expiresAt");
    let expires = match expires_raw {
        Some(Value::Null) => None,
        v => js_number(v).filter(|n| *n >= 0.0),
    };
    let Some(expires) = expires else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid expiration date");
    };
    if !["book", "podcastEpisode"].contains(&kind) {
        return send_text(StatusCode::BAD_REQUEST, "Invalid media item type");
    }
    let done = (|| -> Result<Response> {
        let existing = st
            .abs
            .one("SELECT mediaItemId FROM mediaItemShares WHERE slug = ? OR mediaItemId = ? LIMIT 1", vec![json!(slug), json!(item_id)])?;
        if let Some(row) = existing {
            let text = if row.s("mediaItemId") == item_id { "Item is already shared" } else { "Slug is already in use" };
            return Ok(send_text(StatusCode::CONFLICT, text));
        }
        let table = if kind == "book" { "books" } else { "podcastEpisodes" };
        if st.abs.one(&format!("SELECT id FROM {table} WHERE id = ?"), vec![json!(item_id)])?.is_none() {
            return Ok(send_text(StatusCode::NOT_FOUND, "Media item not found"));
        }
        let now = models::date_now();
        // JavaScript's `expiresAt || null`, kept as a time like Sequelize's date field
        let expires_at = (expires != 0.0 && expires_raw.is_some_and(|v| truthy(Some(v)))).then(|| models::date_text(expires as i64));
        let downloadable = match b.get("isDownloadable") {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(s)) if s == "false" || s == "0" => json!(0),
            Some(v) => json!(i64::from(truthy(Some(v)))),
        };
        let id = models::new_id();
        st.abs.execute(
            "INSERT INTO mediaItemShares (id, mediaItemId, mediaItemType, slug, expiresAt, isDownloadable, createdAt, updatedAt, userId) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                json!(id),
                json!(item_id),
                json!(kind),
                json!(slug),
                json!(expires_at),
                downloadable,
                json!(now),
                json!(now),
                json!(me.user.id),
            ],
        )?;
        let row = st.abs.one("SELECT * FROM mediaItemShares WHERE id = ?", vec![json!(id)])?.unwrap_or_default();
        let share = share_json(&row);
        let answer = client_json(&share);
        if expires_ms(&share).is_some() {
            schedule(&st, share);
        } else {
            st.abs_server.shares.with_open(|list| list.push(Open { id, share, timer: None }));
        }
        st.sockets.admin_emitter("share_open", &answer);
        Ok((StatusCode::CREATED, Json(answer)).into_response())
    })();
    done.unwrap_or_else(|e| {
        tracing::error!("The share can't be made: {}", e.detail);
        send_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
    })
}

/// `DELETE /api/share/mediaitem/:id` (admins only): ends a share (`deleteMediaItemShare`).
async fn remove(State(st): State<AppState>, me: AbsUser, UrlPath(id): UrlPath<String>) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("Non-admin user \"{}\" attempted to delete item share", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    match st.abs.one("SELECT id FROM mediaItemShares WHERE id = ?", vec![json!(id)]) {
        Ok(Some(_)) => {
            remove_share(&st, &id);
            destroy(&st, &id);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => send_text(StatusCode::NOT_FOUND, "Media item share not found"),
        Err(e) => {
            tracing::error!("The share can't be read: {}", e.detail);
            send_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// The visitor's share session cookie, if sent.
fn share_cookie(headers: &HeaderMap) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == "share_session_id" && !v.is_empty()).then(|| v.to_string())
    })
}

/// Whether text is a version 4 UUID (`uuid.validate` and `uuid.version(...) === 4`).
fn is_uuid_v4(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 36
        && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
        && b[14] == b'4'
        && matches!(b[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

/// `GET /public/share/:slug`: the share and a listening session for the visitor, opened (or picked up again)
/// with the visitor's cookie (`getMediaItemShareBySlug`). `?t=` starts at that second.
async fn by_slug(
    State(st): State<AppState>,
    UrlPath(slug): UrlPath<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    peer: Peer,
) -> Response {
    let mut start_time = q
        .get("t")
        .filter(|t| !t.is_empty())
        .and_then(|t| js_number(Some(&json!(t))).map(|_| t))
        .map(|t| parse_int(t).max(0.0))
        .unwrap_or(0.0);
    let shares = &st.abs_server.shares;
    let Some(mut share) = shares.find_by_slug(&slug) else {
        tracing::warn!("Media item share not found with slug {slug}");
        return send_status(StatusCode::NOT_FOUND);
    };
    if expires_ms(&share).is_some_and(|end| end < models::now_ms()) {
        let id = share.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        remove_share(&st, &id);
        return send_text(StatusCode::NOT_FOUND, "Media item share not found");
    }
    let share_id = share.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let cookie = share_cookie(&headers);
    let mut clear_cookie = false;
    if let Some(c) = &cookie {
        match shares.session_of(c) {
            Some(found) if found.media_item_share_id == share_id => {
                let duration = found.session.duration.as_f64().unwrap_or(0.0);
                let mut picked = found.clone();
                if start_time > 0.0 && start_time < duration {
                    shares.with_sessions(|list| {
                        if let Some(s) = list.iter_mut().find(|s| s.share_session_id == *c) {
                            s.session.current_time = json!(start_time);
                            picked = s.clone();
                        }
                    });
                }
                share.insert("playbackSession".into(), picked.to_json_for_client());
                return Json(Value::Object(share)).into_response();
            }
            Some(other) => {
                tracing::debug!(
                    "Other playback session is already open for share session. Closing session \"{}\"",
                    other.session.display_title.as_str().unwrap_or_default()
                );
                shares.close_session(&other.session.id);
            }
            None => {
                tracing::info!("Share playback session not found with id {c}");
                if !is_uuid_v4(c) {
                    tracing::warn!("Invalid share session id {c}");
                    clear_cookie = true;
                }
            }
        }
    }
    let item_id = share.get("mediaItemId").and_then(Value::as_str).unwrap_or_default().to_string();
    let kind = share.get("mediaItemType").and_then(Value::as_str).unwrap_or_default().to_string();
    let opened = (|| -> Result<Option<ShareSession>> {
        if kind != "book" {
            return Ok(None);
        }
        let Some(item) = models::list::<LibraryItem>(&st.abs, "mediaId = ?", "", vec![json!(item_id)])?.into_iter().next() else {
            return Ok(None);
        };
        let Some(b) = book::load(&st.abs, std::slice::from_ref(&item_id))?.remove(&item_id) else { return Ok(None) };
        let mut start_offset = 0.0;
        let tracks: Vec<Value> = b
            .included_audio_files()
            .into_iter()
            .map(|af| {
                let duration = af.get("duration").cloned().unwrap_or(Value::Null);
                let index = af.get("index").cloned().unwrap_or(Value::Null);
                let t = json!({
                    "index": index,
                    "startOffset": crate::abs::library::util::js_num(start_offset),
                    "duration": duration,
                    "title": af.pointer("/metadata/filename").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!("")),
                    "contentUrl": format!("/public/share/{slug}/track/{}", index.as_i64().map(|i| i.to_string()).unwrap_or_else(|| index.to_string())),
                    "mimeType": af.get("mimeType").cloned().unwrap_or(Value::Null),
                    "codec": af.get("codec").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null),
                    "metadata": af.get("metadata").cloned().unwrap_or(Value::Null),
                });
                start_offset += duration.as_f64().unwrap_or(f64::NAN);
                t
            })
            .collect();
        if start_time > start_offset {
            tracing::warn!("Start time {start_time} is greater than total duration {start_offset}");
            start_time = 0.0;
        }
        let share_session_id = cookie.clone().unwrap_or_else(models::new_id);
        let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or_default();
        let client = json!({"clientName": "Abs Web Share", "deviceId": share_session_id});
        let fresh = DeviceInfo::from_request(Some(client_ip(&headers, peer.0)), ua, &client, crate::abs::ABS_VERSION, None);
        let device = device::for_request(&st.abs, fresh)?;
        let library = item.library_id.as_deref().map(|l| models::find::<Library>(&st.abs, l)).transpose()?.flatten();
        let cover_aspect_ratio = library.and_then(|l| l.settings).and_then(|s| s.get("coverAspectRatio").cloned()).unwrap_or(Value::Null);
        let now = models::now_ms();
        let (day, weekday) = session::day_of(now);
        let s = Session {
            id: models::new_id(),
            user_id: None,
            library_id: item.library_id.clone(),
            library_item_id: item.id.clone(),
            book_id: b.book.id.clone(),
            episode_id: None,
            media_type: json!(item.media_type),
            media_metadata: Value::Object(b.metadata_json()),
            chapters: match &b.book.chapters {
                Some(c @ Value::Array(_)) => c.clone(),
                _ => json!([]),
            },
            display_title: json!(b.book.title),
            display_author: json!(b.author_name()),
            cover_path: json!(b.book.cover_path),
            duration: json!(b.book.duration),
            play_method: json!(session::DIRECT_PLAY),
            media_player: json!("web-share"),
            device_info: Some(device),
            server_version: json!(crate::abs::ABS_VERSION),
            date: json!(day),
            day_of_week: json!(weekday),
            time_listening: json!(0),
            start_time: json!(start_time),
            current_time: json!(start_time),
            started_at: json!(now),
            updated_at: json!(now),
            last_save: 0,
            audio_tracks: tracks,
        };
        Ok(Some(ShareSession { session: s, share_session_id, media_item_share_id: share_id.clone(), cover_aspect_ratio }))
    })();
    let opened = match opened {
        Ok(Some(o)) => o,
        Ok(None) => return with_cleared(send_text(StatusCode::NOT_FOUND, "Media item not found"), clear_cookie),
        Err(e) => {
            tracing::error!("The share session can't be opened: {}", e.detail);
            return with_cleared(send_text(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"), clear_cookie);
        }
    };
    share.insert("playbackSession".into(), opened.to_json_for_client());
    tracing::info!("Adding new open share playback session \"{}\"", opened.session.display_title.as_str().unwrap_or_default());
    let cookie_value = format!("share_session_id={}; Max-Age={COOKIE_MAX_AGE}; Path=/; HttpOnly", opened.share_session_id);
    shares.with_sessions(|list| list.push(opened));
    let mut r = with_cleared(Json(Value::Object(share)).into_response(), clear_cookie);
    if let Ok(v) = HeaderValue::from_str(&cookie_value) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

/// Adds the cookie removal to an answer when the visitor's cookie was not a session id (`res.clearCookie`).
fn with_cleared(mut r: Response, clear: bool) -> Response {
    if clear {
        r.headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_static("share_session_id=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"));
    }
    r
}

/// JavaScript's `parseInt` of text: the leading whole number, NaN (here 0) when there is none.
fn parse_int(t: &str) -> f64 {
    let t = t.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    let d: String = digits.chars().take_while(char::is_ascii_digit).collect();
    d.parse::<f64>().map(|n| sign * n).unwrap_or(0.0)
}

/// The share and the visitor's session for the public file routes; the original's answer when either is
/// missing.
fn share_and_session(
    st: &AppState,
    headers: &HeaderMap,
    slug: &str,
) -> std::result::Result<(Map<String, Value>, ShareSession), Box<Response>> {
    let Some(cookie) = share_cookie(headers) else {
        return Err(Box::new(send_text(StatusCode::NOT_FOUND, "Share session not set")));
    };
    let Some(share) = st.abs_server.shares.find_by_slug(slug) else {
        return Err(Box::new(send_status(StatusCode::NOT_FOUND)));
    };
    match st.abs_server.shares.session_of(&cookie) {
        Some(s) if Some(s.media_item_share_id.as_str()) == share.get("id").and_then(Value::as_str) => Ok((share, s)),
        _ => Err(Box::new(send_text(StatusCode::NOT_FOUND, "Share session not found"))),
    }
}

/// Sends a file like Express's `sendFile`, with ranges.
async fn send_file(path: &Path, req: Request) -> Response {
    match tower_http::services::ServeFile::new(path).oneshot(req).await {
        Ok(r) => r.map(Body::new),
        Err(e) => kit_server_core::err(500, format!("The file {} can't be read: {e}", path.display())).into_response(),
    }
}

/// `GET /public/share/:slug/track/:index`: one audio file of the shared book (`getMediaItemShareAudioTrack`).
async fn track(State(st): State<AppState>, UrlPath((slug, index)): UrlPath<(String, String)>, req: Request) -> Response {
    let (_, s) = match share_and_session(&st, req.headers(), &slug) {
        Ok(found) => found,
        Err(r) => return *r,
    };
    let wanted = parse_int(&index);
    let starts_with_digit = index.trim_start().trim_start_matches(['-', '+']).starts_with(|c: char| c.is_ascii_digit());
    let found = s.session.audio_tracks.iter().find(|t| starts_with_digit && t.get("index").and_then(Value::as_f64) == Some(wanted));
    let Some(path) = found.and_then(|t| t.pointer("/metadata/path")).and_then(Value::as_str).map(PathBuf::from) else {
        return send_text(StatusCode::NOT_FOUND, "Track not found");
    };
    let mut r = send_file(&path, req).await;
    if let Some(m) = crate::abs::sessions::audio_mime(&path.display().to_string()) {
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(m));
    }
    r
}

/// `GET /public/share/:slug/cover`: the shared book's cover (`getMediaItemShareCoverImage`).
async fn cover(State(st): State<AppState>, UrlPath(slug): UrlPath<String>, req: Request) -> Response {
    let (_, s) = match share_and_session(&st, req.headers(), &slug) {
        Ok(found) => found,
        Err(r) => return *r,
    };
    let Some(path) = s.session.cover_path.as_str().filter(|p| !p.is_empty()).map(PathBuf::from) else {
        return send_text(StatusCode::NOT_FOUND, "Cover image not found");
    };
    send_file(&path, req).await
}

/// `GET /public/share/:slug/download`: the shared book's file, or its folder as a zip, when the share allows
/// downloads (`downloadMediaItemShare`).
async fn download(State(st): State<AppState>, UrlPath(slug): UrlPath<String>, req: Request) -> Response {
    if share_cookie(req.headers()).is_none() {
        return send_text(StatusCode::NOT_FOUND, "Share session not set");
    }
    let Some(share) = st.abs_server.shares.find_by_slug(&slug) else {
        return send_status(StatusCode::NOT_FOUND);
    };
    if !share.get("isDownloadable").is_some_and(|v| truthy(Some(v))) {
        return send_text(StatusCode::FORBIDDEN, "Download is not allowed for this item");
    }
    let (_, s) = match share_and_session(&st, req.headers(), &slug) {
        Ok(found) => found,
        Err(r) => return *r,
    };
    let item = match s.session.library_item_id.as_deref().map(|id| models::find::<LibraryItem>(&st.abs, id)).transpose() {
        Ok(Some(Some(item))) => item,
        Ok(_) => return send_text(StatusCode::NOT_FOUND, "Library item not found"),
        Err(e) => return e.into_response(),
    };
    let item_path = PathBuf::from(item.path.clone().unwrap_or_default());
    let title = s.session.display_title.as_str().unwrap_or_default().to_string();
    tracing::info!("Requested download for book \"{title}\" at \"{}\"", item_path.display());
    let (file, name, zipped) = if item.is_file.unwrap_or(false) {
        (item_path.clone(), item.rel_path.clone().unwrap_or_default(), false)
    } else {
        let tmp = st.abs_server.metadata_path.join("tmp");
        let zip = tmp.join(format!("share-{}.zip", models::new_id()));
        let made = {
            let (dir, zip) = (item_path.clone(), zip.clone());
            tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&tmp).map_err(|e| e.to_string()).and_then(|_| zip_folder(&dir, &zip))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        };
        if let Err(e) = made {
            tracing::error!("Download failed for item \"{title}\" at \"{}\": {e}", item_path.display());
            let _ = std::fs::remove_file(&zip);
            return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Failed to download the item");
        }
        (zip, format!("{title}.zip"), true)
    };
    if !file.is_file() {
        tracing::error!("Download failed for item \"{title}\" at \"{}\": the file is missing", item_path.display());
        return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Failed to download the item");
    }
    let mut r = send_file(&file, req).await;
    if zipped {
        // the zip is already open for sending, so its name can go now; the system frees it when the send ends
        let _ = std::fs::remove_file(&file);
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    } else if let Some(m) = crate::abs::sessions::audio_mime(&file.display().to_string()) {
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(m));
    }
    if let Ok(v) = HeaderValue::from_str(&attachment(&name)) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    tracing::info!("Downloaded item \"{title}\" at \"{}\"", item_path.display());
    r
}

/// A `Content-Disposition` for a download named `name`, like Express's `attachment`: a plain-text name, plus the
/// exact name encoded when it has other characters.
fn attachment(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    let plain: String = base.chars().map(|c| if c.is_ascii() && c != '"' && !c.is_ascii_control() { c } else { '?' }).collect();
    if plain == base {
        format!("attachment; filename=\"{base}\"")
    } else {
        let encoded: String = base
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect();
        format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
    }
}

/// Writes a folder's files, without compressing them, into a zip (`zipDirectoryPipe`).
fn zip_folder(dir: &Path, out: &Path) -> std::result::Result<(), String> {
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.strip_prefix(dir).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            let mut f = std::fs::File::open(&p).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// `PATCH /public/share/:slug/progress`: where the visitor is in the book (`updateMediaItemShareProgress`).
async fn progress(State(st): State<AppState>, UrlPath(slug): UrlPath<String>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(cookie) = share_cookie(&headers) else {
        return send_text(StatusCode::NOT_FOUND, "Share session not set");
    };
    let b = body_fields(&headers, &body);
    let current = match b.get("currentTime") {
        Some(Value::Null) => None,
        v => js_number(v).filter(|n| *n >= 0.0),
    };
    let Some(current) = current else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid current time");
    };
    let (_, _) = match share_and_session(&st, &headers, &slug) {
        Ok(found) => found,
        Err(r) => return *r,
    };
    st.abs_server.shares.with_sessions(|list| {
        if let Some(s) = list.iter_mut().find(|s| s.share_session_id == cookie) {
            let duration = s.session.duration.as_f64().unwrap_or(f64::NAN);
            let t = if duration.is_nan() { f64::NAN } else { current.min(duration) };
            s.session.current_time = crate::abs::library::util::js_num(t);
            s.session.updated_at = json!(models::now_ms());
            tracing::debug!("Update share playback session {cookie} currentTime: {t}");
        }
    });
    StatusCode::NO_CONTENT.into_response()
}
