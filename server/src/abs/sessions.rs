//! The port of the original's playback session routes: starting a session on an item
//! (`LibraryItemController.startPlaybackSession`), the open session's sync and close and the offline sessions
//! from phones (`SessionController.js`), the stored session lists for admins, and each user's listening sessions
//! and stats (`MeController.js`, `UserController.js`, `ApiRouter`'s listening helpers).

use crate::abs::auth::routes::{body_fields, Peer};
use crate::abs::auth::{client_ip, send_status, AbsUser};
use crate::abs::library::util::{js_numbers, Sorting};
use crate::abs::models::{self, library_item, media_progress::js_number, user, User};
use crate::abs::playback::{self, device, session, session::Session, StartOptions};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/items/{id}/play", post(play).fallback(unknown))
        .route("/api/items/{id}/play/{episode}", post(play_episode).fallback(unknown))
        .route("/api/sessions", get(all).fallback(unknown))
        .route("/api/sessions/open", get(open).fallback(unknown))
        .route("/api/sessions/batch/delete", post(batch_delete).fallback(unknown))
        .route("/api/sessions/{id}", delete(remove).fallback(unknown))
        .route("/api/session/local", post(local).fallback(unknown))
        .route("/api/session/local-all", post(local_all).fallback(unknown))
        .route("/api/session/{id}", get(open_one).fallback(unknown))
        .route("/api/session/{id}/sync", post(sync).fallback(unknown))
        .route("/api/session/{id}/close", post(close).fallback(unknown))
        .route("/api/me/listening-sessions", get(my_sessions).fallback(unknown))
        .route("/api/me/item/listening-sessions/{item}", get(my_item_sessions).fallback(unknown))
        .route("/api/me/item/listening-sessions/{item}/{episode}", get(my_episode_sessions).fallback(unknown))
        .route("/api/me/listening-stats", get(my_stats).fallback(unknown))
        .route("/api/users/{id}/listening-sessions", get(user_sessions).fallback(unknown))
        .route("/api/users/{id}/listening-stats", get(user_stats).fallback(unknown))
}

/// Another method on one of these addresses: Express's "Cannot METHOD /path" for a signed-in user, like the
/// original's API router (rather than the web app's page).
async fn unknown(_me: AbsUser, method: axum::http::Method, axum::extract::OriginalUri(uri): axum::extract::OriginalUri) -> Response {
    crate::abs::auth::routes::not_found(&method, uri.path())
}

/// A server error as an answer.
fn failed(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// Runs a handler body that may fail.
fn answer(run: impl FnOnce() -> Result<Response>) -> Response {
    run().unwrap_or_else(failed)
}

/// Whether text is a UUID (`isUUID`, the `uuid` package's `validate`).
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 || [8, 13, 18, 23].iter().any(|&i| b[i] != b'-') {
        return false;
    }
    if !s.chars().enumerate().all(|(i, c)| [8, 13, 18, 23].contains(&i) || c.is_ascii_hexdigit()) {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "00000000-0000-0000-0000-000000000000" || lower == "ffffffff-ffff-ffff-ffff-ffffffffffff" {
        return true;
    }
    matches!(b[14], b'1'..=b'8') && matches!(lower.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}

/// The original's `toNumber(value, fallback)` of a query value.
fn to_number(v: Option<&String>, fallback: f64) -> f64 {
    match v {
        None => fallback,
        Some(t) => {
            let t = t.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().ok().filter(|n| !n.is_nan()).unwrap_or(fallback)
            }
        }
    }
}

/// The device a request comes from (`getDeviceInfo`), stored or brought up to date.
fn device_of(st: &AppState, headers: &HeaderMap, peer: &Peer, u: &User, client: Option<&Value>) -> Result<device::DeviceInfo> {
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let ip = client_ip(headers, peer.0);
    let fresh = device::DeviceInfo::from_request(Some(ip), ua, client.unwrap_or(&Value::Null), crate::abs::ABS_VERSION, u.id.clone());
    device::for_request(&st.abs, fresh)
}

/// The expanded library item JSON a player gets with its session, if the item is still there.
fn item_json(st: &AppState, id: Option<&str>) -> Result<Option<Value>> {
    let Some(id) = id else { return Ok(None) };
    let sorting = Sorting::of(&st.abs_server.settings());
    Ok(crate::abs::items::expanded_by_id(&st.abs, id)?.map(|(item, b)| Value::Object(library_item::expanded(&item, &b, &sorting))))
}

/// `POST /api/items/:id/play` (A0043, `startPlaybackSession`): starts a session; 404 when the item has no audio.
async fn play(State(st): State<AppState>, me: AbsUser, peer: Peer, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    answer(|| {
        let Some((item, b)) = crate::abs::items::expanded_by_id(&st.abs, &id)? else {
            return Ok(send_status(StatusCode::NOT_FOUND));
        };
        if !crate::abs::items::can_access(&me.user, &item, &b) {
            return Ok(send_status(StatusCode::FORBIDDEN));
        }
        if b.included_audio_files().is_empty() {
            tracing::error!("Starting a session: item {id} has nothing to play");
            return Ok(send_status(StatusCode::NOT_FOUND));
        }
        let device = device_of(&st, &headers, &peer, &me.user, fields.get("deviceInfo"))?;
        let s = playback::start(&st, &me.user, device, &item, &b, &StartOptions::from_body(&fields))?;
        let sorting = Sorting::of(&st.abs_server.settings());
        Ok(Json(s.to_json_for_client(Some(Value::Object(library_item::expanded(&item, &b, &sorting))))).into_response())
    })
}

/// `POST /api/items/:id/play/:episodeId` (A0044, `startEpisodePlaybackSession`): 400 for an item that isn't a
/// podcast. Podcast libraries are not ported yet, so a podcast item is not found here.
async fn play_episode(State(st): State<AppState>, me: AbsUser, Path((id, _episode)): Path<(String, String)>) -> Response {
    answer(|| {
        let Some((item, b)) = crate::abs::items::expanded_by_id(&st.abs, &id)? else {
            return Ok(send_status(StatusCode::NOT_FOUND));
        };
        if !crate::abs::items::can_access(&me.user, &item, &b) {
            return Ok(send_status(StatusCode::FORBIDDEN));
        }
        tracing::error!("Starting an episode session: item {id} is not a podcast");
        Ok(send_status(StatusCode::BAD_REQUEST))
    })
}

/// The open session a route names, after the original's checks (`openSessionMiddleware`): 404 when it isn't
/// open, 403 when it is someone else's and the user isn't an admin.
fn open_for(st: &AppState, u: &User, id: &str) -> std::result::Result<Session, Box<Response>> {
    let Some(s) = playback::open_session(st, id) else { return Err(Box::new(send_status(StatusCode::NOT_FOUND))) };
    if s.user_id != u.id && !user::is_admin_or_up(u) {
        tracing::error!("{} tried to use another user's session {id}", u.username.as_deref().unwrap_or_default());
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    Ok(s)
}

/// `GET /api/session/:id` (A0130, `getOpenSession`): the open session as its player sees it.
async fn open_one(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let s = match open_for(&st, &me.user, &id) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    answer(|| Ok(Json(s.to_json_for_client(item_json(&st, s.library_item_id.as_deref())?)).into_response()))
}

/// `POST /api/session/:id/sync` (A0131, `sync`): where the listener is and how long they listened.
async fn sync(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let mut s = match open_for(&st, &me.user, &id) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let fields = body_fields(&headers, &body);
    answer(|| {
        Ok(if playback::sync(&st, &me.user, &mut s, &fields)? {
            send_status(StatusCode::OK)
        } else {
            send_status(StatusCode::INTERNAL_SERVER_ERROR)
        })
    })
}

/// `POST /api/session/:id/close` (A0132, `close`): a last sync when the player sends one, then the session ends.
async fn close(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let mut s = match open_for(&st, &me.user, &id) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let fields = body_fields(&headers, &body);
    let data = (!fields.is_empty()).then_some(&fields);
    answer(|| {
        playback::close(&st, &me.user, &mut s, data)?;
        Ok(send_status(StatusCode::OK))
    })
}

/// `POST /api/session/local` (A0128, `syncLocal`): one session a phone played offline; 500 with the reason when
/// it can't be taken in.
async fn local(State(st): State<AppState>, me: AbsUser, peer: Peer, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    answer(|| {
        let device = device_of(&st, &headers, &peer, &me.user, fields.get("deviceInfo"))?;
        let result = playback::sync_local(&st, &me.user, fields, &device)?;
        Ok(match result.get("error").and_then(Value::as_str) {
            Some(error) => send_text(StatusCode::INTERNAL_SERVER_ERROR, error),
            None => send_status(StatusCode::OK),
        })
    })
}

/// `POST /api/session/local-all` (A0129, `syncLocalSessions`): many offline sessions; answers each one's result.
async fn local_all(State(st): State<AppState>, me: AbsUser, peer: Peer, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    answer(|| {
        let device = device_of(&st, &headers, &peer, &me.user, fields.get("deviceInfo"))?;
        let list = fields.get("sessions").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut results = Vec::with_capacity(list.len());
        for one in list {
            let one = one.as_object().cloned().unwrap_or_default();
            results.push(playback::sync_local(&st, &me.user, one, &device)?);
        }
        Ok(Json(json!({ "results": results })).into_response())
    })
}

/// `GET /api/sessions` (A0124, `getAllWithUserData`): the stored sessions of everyone (or one user), sorted and
/// paged, each with its user's name; 404 for anyone but an admin.
async fn all(State(st): State<AppState>, me: AbsUser, Query(q): Query<HashMap<String, String>>) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::NOT_FOUND);
    }
    let user_id = q.get("user").filter(|u| is_uuid(u)).cloned();
    const SORTS: &[&str] =
        &["displayTitle", "duration", "playMethod", "startTime", "currentTime", "timeListening", "updatedAt", "createdAt"];
    let key = q.get("sort").map(String::as_str).filter(|k| SORTS.contains(k)).unwrap_or("updatedAt");
    let dir = if q.get("desc").map(String::as_str) == Some("1") { "DESC" } else { "ASC" };
    let mut per_page = to_number(q.get("itemsPerPage"), 10.0);
    if per_page == 0.0 || per_page.is_nan() {
        per_page = 10.0;
    }
    if per_page < 1.0 {
        per_page = 10.0;
    }
    let mut page = to_number(q.get("page"), 0.0);
    if page < 0.0 {
        page = 0.0;
    }
    answer(|| {
        let (filter, params) = match &user_id {
            Some(u) => ("WHERE `userId` = ?", vec![json!(u)]),
            None => ("", vec![]),
        };
        let total = st
            .abs
            .one(&format!("SELECT COUNT(*) AS n FROM `playbackSessions` {filter}"), params.clone())?
            .and_then(|r| r.get("n").and_then(Value::as_i64))
            .unwrap_or(0);
        let mut p = params;
        p.push(json!(per_page as i64));
        p.push(json!((per_page * page) as i64));
        let rows = st.abs.query(&format!("SELECT * FROM `playbackSessions` {filter} ORDER BY `{key}` {dir} LIMIT ? OFFSET ?"), p)?;
        let mut sessions = Vec::with_capacity(rows.len());
        for s in session::from_rows(&st.abs, rows)? {
            let owner = match &s.user_id {
                Some(id) => user::by_id(&st.abs, id)?,
                None => None,
            };
            sessions.push(match owner {
                Some(u) => {
                    let mut m = s.to_spread_json();
                    m.insert("user".into(), json!({"id": u.id, "username": u.username}));
                    Value::Object(m)
                }
                None => s.to_json(),
            });
        }
        let mut payload = json!({
            "total": total,
            "numPages": (total as f64 / per_page).ceil(),
            "page": page,
            "itemsPerPage": per_page,
            "sessions": sessions,
        });
        if let Some(u) = user_id {
            payload["userId"] = json!(u);
        }
        Ok(Json(js_numbers(payload)).into_response())
    })
}

/// `GET /api/sessions/open` (A0126, `getOpenSessions`): every open session with its user; 404 for anyone but an
/// admin. Shared listening links are not ported yet, so `shareSessions` is empty.
async fn open(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::NOT_FOUND);
    }
    answer(|| {
        let users = models::list::<User>(&st.abs, "1", "", vec![])?;
        let list = playback::with_open(&st, |l| l.clone());
        let sessions: Vec<Value> = list
            .iter()
            .map(|s| {
                let mut j = s.to_json();
                let owner = users.iter().find(|u| u.id.is_some() && u.id == s.user_id);
                j["user"] = owner.map_or(Value::Null, |u| json!({"id": u.id, "username": u.username}));
                j
            })
            .collect();
        Ok(Json(json!({ "sessions": sessions, "shareSessions": [] })).into_response())
    })
}

/// `DELETE /api/sessions/:id` (A0125, `delete`): removes a stored session (and closes it when open); 404 when
/// there is no such session, 403 without the delete permission.
async fn remove(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    answer(|| {
        if session::by_id(&st.abs, &id)?.is_none() {
            tracing::error!("There is no playback session {id}");
            return Ok(send_status(StatusCode::NOT_FOUND));
        }
        if !user::can(&me.user, "delete") {
            return Ok(send_status(StatusCode::FORBIDDEN));
        }
        playback::remove_open(&st, &id);
        session::remove(&st.abs, &id)?;
        Ok(send_status(StatusCode::OK))
    })
}

/// `POST /api/sessions/batch/delete` (A0127, `batchDelete`): removes many sessions; 403 for anyone but an admin,
/// 400 without a list of session ids.
async fn batch_delete(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let fields = body_fields(&headers, &body);
    let ids: Option<Vec<String>> = fields
        .get("sessions")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .and_then(|a| a.iter().map(|v| v.as_str().filter(|s| is_uuid(s)).map(String::from)).collect());
    let Some(ids) = ids else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid request body. \"sessions\" array of session id strings is required.");
    };
    answer(|| {
        for id in &ids {
            playback::remove_open(&st, id);
            session::remove(&st.abs, id)?;
        }
        Ok(send_status(StatusCode::OK))
    })
}

/// A page of sessions as the listening session routes answer it.
fn page_of(list: Vec<Value>, q: &HashMap<String, String>) -> Value {
    let mut per_page = to_number(q.get("itemsPerPage"), 10.0);
    if per_page == 0.0 || per_page.is_nan() {
        per_page = 10.0;
    }
    let page = to_number(q.get("page"), 0.0);
    let start = (page * per_page).max(0.0) as usize;
    let end = (page * per_page + per_page).max(0.0) as usize;
    let total = list.len();
    let sessions: Vec<Value> = list.into_iter().skip(start).take(end.saturating_sub(start)).collect();
    js_numbers(json!({
        "total": total,
        "numPages": (total as f64 / per_page).ceil(),
        "page": page,
        "itemsPerPage": per_page,
        "sessions": sessions,
    }))
}

/// `GET /api/me/listening-sessions` (A0089, `getListeningSessions`): the user's stored sessions, newest first.
async fn my_sessions(State(st): State<AppState>, me: AbsUser, Query(q): Query<HashMap<String, String>>) -> Response {
    answer(|| {
        let list = session::of_user(&st.abs, me.user.id.as_deref().unwrap_or_default(), None)?;
        Ok(Json(page_of(list.iter().map(Session::to_json).collect(), &q)).into_response())
    })
}

/// The sessions of one item for the user (`getItemListeningSessions`): 404 when the item isn't there, 403 when
/// the user may not see it.
fn item_sessions(st: &AppState, u: &User, item: &str, q: &HashMap<String, String>) -> Result<Response> {
    let Some((item, b)) = crate::abs::items::expanded_by_id(&st.abs, item)? else {
        return Ok(send_status(StatusCode::NOT_FOUND));
    };
    if !crate::abs::items::can_access(u, &item, &b) {
        return Ok(send_status(StatusCode::FORBIDDEN));
    }
    let media = item.media_id.clone().unwrap_or_default();
    let list = session::of_user(&st.abs, u.id.as_deref().unwrap_or_default(), Some(&media))?;
    Ok(Json(page_of(list.iter().map(Session::to_json).collect(), q)).into_response())
}

/// `GET /api/me/item/listening-sessions/:libraryItemId` (A0090).
async fn my_item_sessions(
    State(st): State<AppState>,
    me: AbsUser,
    Path(item): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    answer(|| item_sessions(&st, &me.user, &item, &q))
}

/// `GET /api/me/item/listening-sessions/:libraryItemId/:episodeId` (A0090). Podcasts are not ported yet, so the
/// episode doesn't narrow a book's sessions, like the original for a book.
async fn my_episode_sessions(
    State(st): State<AppState>,
    me: AbsUser,
    Path((item, _episode)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    answer(|| item_sessions(&st, &me.user, &item, &q))
}

/// JavaScript's `a + b` for listening times (a missing time counts as 0).
fn add(a: &Value, b: f64) -> Value {
    json!(js_number(Some(a)).unwrap_or(0.0) + b)
}

/// A user's listening stats (`getUserListeningStatsHelpers`): total time, per item, per day and weekday, today,
/// and the ten latest sessions.
fn stats_of(st: &AppState, user_id: &str) -> Result<Value> {
    let today = session::day_of(models::now_ms()).0;
    let list = session::of_user(&st.abs, user_id, None)?;
    let (mut total, mut today_total) = (0.0, 0.0);
    let (mut items, mut days, mut weekdays) = (Map::new(), Map::new(), Map::new());
    for s in &list {
        let time = js_number(Some(&s.time_listening)).unwrap_or(0.0);
        if let Some(w) = s.day_of_week.as_str().filter(|w| !w.is_empty()) {
            let now = add(weekdays.get(w).unwrap_or(&json!(0)), time);
            weekdays.insert(w.into(), now);
        }
        if let Some(d) = s.date.as_str().filter(|d| !d.is_empty()) {
            if time > 0.0 {
                let now = add(days.get(d).unwrap_or(&json!(0)), time);
                days.insert(d.into(), now);
                if d == today {
                    today_total += time;
                }
            }
        }
        let key = s.library_item_id.clone().unwrap_or_else(|| "null".into());
        match items.get_mut(&key) {
            Some(Value::Object(entry)) => {
                let now = add(entry.get("timeListening").unwrap_or(&Value::Null), time);
                entry.insert("timeListening".into(), now);
            }
            _ => {
                items.insert(key, json!({"id": s.library_item_id, "timeListening": s.time_listening, "mediaMetadata": s.media_metadata}));
            }
        }
        total += time;
    }
    let recent: Vec<Value> = list.iter().take(10).map(Session::to_json).collect();
    Ok(js_numbers(json!({
        "totalTime": total,
        "items": items,
        "days": days,
        "dayOfWeek": weekdays,
        "today": today_total,
        "recentSessions": recent,
    })))
}

/// `GET /api/me/listening-stats` (A0091, `getListeningStats`).
async fn my_stats(State(st): State<AppState>, me: AbsUser) -> Response {
    answer(|| Ok(Json(stats_of(&st, me.user.id.as_deref().unwrap_or_default())?).into_response()))
}

/// The user a `/api/users/:id` route names (`UserController.middleware`): 403 for another user unless an admin,
/// 404 when there is no such user.
fn user_for(st: &AppState, me: &User, id: &str) -> Result<std::result::Result<User, Response>> {
    if !user::is_admin_or_up(me) && me.id.as_deref() != Some(id) {
        return Ok(Err(send_status(StatusCode::FORBIDDEN)));
    }
    Ok(match user::by_id(&st.abs, id)? {
        Some(u) => Ok(u),
        None => Err(send_status(StatusCode::NOT_FOUND)),
    })
}

/// `GET /api/users/:id/listening-sessions` (A0062): a user's sessions, each with the user's name.
async fn user_sessions(
    State(st): State<AppState>,
    me: AbsUser,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    answer(|| {
        let u = match user_for(&st, &me.user, &id)? {
            Ok(u) => u,
            Err(r) => return Ok(r),
        };
        let list = session::of_user(&st.abs, &id, None)?;
        let sessions = list
            .iter()
            .map(|s| {
                let mut m = s.to_spread_json();
                m.insert("user".into(), json!({"id": u.id, "username": u.username}));
                Value::Object(m)
            })
            .collect();
        Ok(Json(page_of(sessions, &q)).into_response())
    })
}

/// `GET /api/users/:id/listening-stats` (A0063).
async fn user_stats(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    answer(|| {
        if let Err(r) = user_for(&st, &me.user, &id)? {
            return Ok(r);
        }
        Ok(Json(stats_of(&st, &id)?).into_response())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_checked_like_the_original() {
        assert!(is_uuid("6c1f8d0e-3a52-4b7e-9d1a-2f4e5b6c7d80"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!is_uuid("00000000-0000-0000-0000-00000000000"));
        assert!(!is_uuid("nope"));
        assert!(!is_uuid("6c1f8d0e-3a52-0b7e-9d1a-2f4e5b6c7d80"));
    }

    #[test]
    fn query_numbers_read_like_the_original() {
        assert_eq!(to_number(None, 10.0), 10.0);
        assert_eq!(to_number(Some(&"2".to_string()), 10.0), 2.0);
        assert_eq!(to_number(Some(&"x".to_string()), 10.0), 10.0);
    }
}
