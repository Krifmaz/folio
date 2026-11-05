//! The port of the original's media progress routes (`server/controllers/MeController.js`): a user's listening
//! and reading progress, read, set from the apps (one item or a batch), removed and hidden from Continue
//! Listening. Each change is told to the user's other apps with `user_updated`, like the original.

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, media_progress, user, MediaProgress, User};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// The routes, under `/api/me`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me/progress", get(all))
        .route("/api/me/progress/{id}/remove-from-continue-listening", get(hide_from_continue_listening))
        .route("/api/me/progress/{id}", get(one).patch(set).delete(remove))
        .route("/api/me/progress/{id}/{episode}", get(one_episode).patch(set_episode))
        .route("/api/me/progress/batch/update", patch(set_batch))
}

/// An answer like Express's `res.status(code).send(text)`: the status and the text as HTML.
pub fn send_text(code: StatusCode, text: &str) -> Response {
    let mut r = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    *r.status_mut() = code;
    r
}

/// A server error as an answer.
fn failed(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// Tells the user's apps that the user changed (`user_updated` with `toOldJSONForBrowser`).
pub fn tell_user_updated(st: &AppState, u: &User) -> Result<()> {
    let id = u.id.clone().unwrap_or_default();
    let progress = user::media_progresses(&st.abs, &id)?;
    let fresh = user::by_id(&st.abs, &id)?.unwrap_or_else(|| u.clone());
    st.sockets.client_emitter(&id, "user_updated", &user::to_old_json_for_browser(&fresh, &progress, None, false, false));
    Ok(())
}

/// The request body's fields (JSON or a form, like Express's parsers).
fn fields(headers: &HeaderMap, body: &Bytes) -> Map<String, Value> {
    crate::abs::auth::routes::body_fields(headers, body)
}

/// `GET /api/me/progress`: every progress of the user (`getAllMediaProgress`).
async fn all(State(st): State<AppState>, me: AbsUser) -> Response {
    match user::media_progresses(&st.abs, me.user.id.as_deref().unwrap_or_default()) {
        Ok(list) => Json(json!({ "mediaProgress": list.iter().map(media_progress::old_json).collect::<Vec<_>>() })).into_response(),
        Err(e) => failed(e),
    }
}

/// The user's progress for a library item (and episode), found like `getOldMediaProgress`.
fn find_for_item(db: &dyn Db, user_id: &str, item_id: &str, episode: Option<&str>) -> Result<Option<MediaProgress>> {
    Ok(user::media_progresses(db, user_id)?.into_iter().find(|mp| {
        if episode.is_some_and(|e| !e.is_empty()) && mp.media_item_id.as_deref() != episode {
            return false;
        }
        mp.extra_data.as_ref().and_then(|x| x.get("libraryItemId")).and_then(Value::as_str) == Some(item_id)
    }))
}

/// `GET /api/me/progress/:id/:episodeId?`: the progress for one item (`getMediaProgress`).
async fn get_one(st: &AppState, me: &AbsUser, item: &str, episode: Option<&str>) -> Response {
    match find_for_item(&st.abs, me.user.id.as_deref().unwrap_or_default(), item, episode) {
        Ok(Some(mp)) => Json(media_progress::old_json(&mp)).into_response(),
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => failed(e),
    }
}

/// `GET /api/me/progress/:id`.
async fn one(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    get_one(&st, &me, &id, None).await
}

/// `GET /api/me/progress/:id/:episodeId`.
async fn one_episode(State(st): State<AppState>, me: AbsUser, Path((id, episode)): Path<(String, String)>) -> Response {
    get_one(&st, &me, &id, Some(&episode)).await
}

/// Why a progress update can't be made: the original's status and text.
pub struct Refused(pub StatusCode, pub &'static str);

/// Creates or updates the user's progress from a request (`createUpdateMediaProgressFromPayload`): an episode by
/// `episodeId`, else a book by `libraryItemId`; the original's 404 and 400 when neither is found or the item
/// isn't a book. Answers the stored progress.
pub fn create_update(db: &dyn Db, u: &User, payload: &Map<String, Value>) -> Result<std::result::Result<MediaProgress, Refused>> {
    let user_id = u.id.clone().unwrap_or_default();
    let progress = user::media_progresses(db, &user_id)?;
    let episode_id = payload.get("episodeId").filter(|v| crate::abs::settings::truthy(Some(v))).and_then(Value::as_str);
    let (media_item_id, podcast_id) = if let Some(episode_id) = episode_id {
        match models::find::<models::PodcastEpisode>(db, episode_id)? {
            Some(e) => (episode_id.to_string(), e.podcast_id.clone()),
            None => return Ok(Err(Refused(StatusCode::NOT_FOUND, "Episode not found"))),
        }
    } else {
        let item_id = payload.get("libraryItemId").and_then(Value::as_str).unwrap_or_default();
        match models::find::<models::LibraryItem>(db, item_id)? {
            None => return Ok(Err(Refused(StatusCode::NOT_FOUND, "Library item not found"))),
            Some(i) if i.media_type.as_deref() != Some("book") => {
                return Ok(Err(Refused(StatusCode::BAD_REQUEST, "Library item is not a book")))
            }
            Some(i) => (i.media_id.clone().unwrap_or_default(), None),
        }
    };
    let existing = progress.into_iter().find(|mp| mp.media_item_id.as_deref() == Some(media_item_id.as_str()));
    match existing {
        Some(mut mp) => {
            if media_progress::apply_update(&mut mp, payload) {
                models::update(db, &mp)?;
            }
            Ok(Ok(mp))
        }
        None => {
            let mp = media_progress::from_payload(&user_id, &media_item_id, podcast_id.as_deref(), payload);
            models::insert(db, &mp)?;
            Ok(Ok(mp))
        }
    }
}

/// `PATCH /api/me/progress/:libraryItemId/:episodeId?` (`createUpdateMediaProgress`).
async fn set_for(st: &AppState, me: &AbsUser, item: &str, episode: Option<&str>, mut payload: Map<String, Value>) -> Response {
    payload.insert("libraryItemId".into(), json!(item));
    payload.insert("episodeId".into(), episode.map(Value::from).unwrap_or(Value::Null));
    match create_update(&st.abs, &me.user, &payload) {
        Ok(Ok(_)) => match tell_user_updated(st, &me.user) {
            Ok(()) => send_status(StatusCode::OK),
            Err(e) => failed(e),
        },
        Ok(Err(Refused(code, text))) => send_text(code, text),
        Err(e) => failed(e),
    }
}

/// `PATCH /api/me/progress/:libraryItemId`.
async fn set(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    set_for(&st, &me, &id, None, fields(&headers, &body)).await
}

/// `PATCH /api/me/progress/:libraryItemId/:episodeId`.
async fn set_episode(
    State(st): State<AppState>,
    me: AbsUser,
    Path((id, episode)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    set_for(&st, &me, &id, Some(&episode), fields(&headers, &body)).await
}

/// `PATCH /api/me/progress/batch/update` (`batchUpdateMediaProgress`): a list of updates; the ones that can't
/// be made are skipped.
async fn set_batch(State(st): State<AppState>, me: AbsUser, body: Bytes) -> Response {
    let list: Vec<Value> = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    if list.is_empty() {
        return send_text(StatusCode::BAD_REQUEST, "Missing request payload");
    }
    let mut updated = false;
    for p in list {
        let payload = p.as_object().cloned().unwrap_or_default();
        match create_update(&st.abs, &me.user, &payload) {
            Ok(Ok(_)) => updated = true,
            Ok(Err(_)) => {}
            Err(e) => return failed(e),
        }
    }
    if updated {
        if let Err(e) = tell_user_updated(&st, &me.user) {
            return failed(e);
        }
    }
    send_status(StatusCode::OK)
}

/// `DELETE /api/me/progress/:id` (`removeMediaProgress`): by the progress's own id.
async fn remove(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let user_id = me.user.id.clone().unwrap_or_default();
    let found = match user::media_progresses(&st.abs, &user_id) {
        Ok(list) => list.into_iter().any(|mp| mp.id.as_deref() == Some(id.as_str())),
        Err(e) => return failed(e),
    };
    if !found {
        return send_status(StatusCode::NOT_FOUND);
    }
    if let Err(e) = models::delete::<MediaProgress>(&st.abs, &id).and_then(|_| tell_user_updated(&st, &me.user)) {
        return failed(e);
    }
    send_status(StatusCode::OK)
}

/// `GET /api/me/progress/:id/remove-from-continue-listening` (`removeItemFromContinueListening`): by the
/// progress's own id; answers the user.
async fn hide_from_continue_listening(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let user_id = me.user.id.clone().unwrap_or_default();
    let run = || -> Result<Option<Value>> {
        let Some(mut mp) = user::media_progresses(&st.abs, &user_id)?.into_iter().find(|mp| mp.id.as_deref() == Some(id.as_str())) else {
            return Ok(None);
        };
        if !mp.hide_from_continue_listening.unwrap_or(false) {
            mp.hide_from_continue_listening = Some(true);
            mp.updated_at = Some(models::date_now());
            models::update(&st.abs, &mp)?;
            tell_user_updated(&st, &me.user)?;
        }
        let progress = user::media_progresses(&st.abs, &user_id)?;
        Ok(Some(user::to_old_json_for_browser(&me.user, &progress, None, false, false)))
    };
    match run() {
        Ok(Some(j)) => Json(j).into_response(),
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => failed(e),
    }
}
