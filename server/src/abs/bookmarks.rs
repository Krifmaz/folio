//! The port of the original's bookmark routes (`server/controllers/MeController.js`, kept on the user as in
//! `server/models/User.js`): a user's bookmarks, all or for one item, and adding, renaming and removing one. A
//! bookmark is `{libraryItemId, time, title, createdAt}`, found by item and time.

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, user, Book, LibraryItem, Podcast, User};
use crate::abs::progress::{send_text, tell_user_updated};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Value};

/// The routes, under `/api/me`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/me/bookmarks", get(all))
        .route("/api/me/bookmarks/{item}", get(for_item))
        .route("/api/me/item/{item}/bookmark", post(create).patch(rename))
        .route("/api/me/item/{item}/bookmark/{time}", delete(remove))
}

/// The user's bookmarks as stored.
fn list_of(u: &User) -> Vec<Value> {
    u.bookmarks.as_ref().and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Whether a bookmark is the one for this item at this time (`findBookmark`, JavaScript's `==` on the time).
fn is_at(b: &Value, item: &str, time: &Value) -> bool {
    let num = |v: &Value| crate::abs::models::media_progress::js_number(Some(v));
    b.get("libraryItemId").and_then(Value::as_str) == Some(item)
        && match (b.get("time"), num(time)) {
            (Some(t), Some(n)) => num(t) == Some(n),
            (Some(t), None) => t == time,
            _ => false,
        }
}

/// The library item a request names, if the user may see it: the original's 404 or 403 otherwise.
pub fn item_for(db: &dyn Db, u: &User, id: &str) -> Result<std::result::Result<LibraryItem, Response>> {
    let Some(item) = models::find::<LibraryItem>(db, id)? else { return Ok(Err(send_status(StatusCode::NOT_FOUND))) };
    let media = item.media_id.clone().unwrap_or_default();
    let (explicit, tags) = if item.media_type.as_deref() == Some("podcast") {
        let p = models::find::<Podcast>(db, &media)?.unwrap_or_default();
        (p.explicit.unwrap_or(false), p.tags)
    } else {
        let b = models::find::<Book>(db, &media)?.unwrap_or_default();
        (b.explicit.unwrap_or(false), b.tags)
    };
    let tags: Vec<String> =
        tags.as_ref().and_then(Value::as_array).into_iter().flatten().filter_map(|t| t.as_str().map(str::to_string)).collect();
    if !user::can_access_item(u, item.library_id.as_deref().unwrap_or_default(), explicit, &tags) {
        return Ok(Err(send_status(StatusCode::FORBIDDEN)));
    }
    Ok(Ok(item))
}

/// The user as stored now (bookmarks may have changed since the request's sign-in check).
fn fresh(st: &AppState, me: &AbsUser) -> Result<User> {
    Ok(user::by_id(&st.abs, me.user.id.as_deref().unwrap_or_default())?.unwrap_or_else(|| me.user.clone()))
}

/// Stores the user's bookmarks.
fn save(st: &AppState, u: &mut User, list: Vec<Value>) -> Result<()> {
    u.bookmarks = Some(Value::Array(list));
    u.updated_at = Some(models::date_now());
    models::update(&st.abs, u)
}

/// `GET /api/me/bookmarks` (`getAllBookmarks`).
async fn all(State(st): State<AppState>, me: AbsUser) -> Response {
    match fresh(&st, &me) {
        Ok(u) => Json(json!({ "bookmarks": list_of(&u) })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/me/bookmarks/:libraryItemId` (`getBookmarksForLibraryItem`).
async fn for_item(State(st): State<AppState>, me: AbsUser, Path(item): Path<String>) -> Response {
    let run = || -> Result<Response> {
        let u = fresh(&st, &me)?;
        let item = match item_for(&st.abs, &u, &item)? {
            Ok(i) => i,
            Err(r) => return Ok(r),
        };
        let id = item.id.unwrap_or_default();
        let list: Vec<Value> =
            list_of(&u).into_iter().filter(|b| b.get("libraryItemId").and_then(Value::as_str) == Some(id.as_str())).collect();
        Ok(Json(json!({ "bookmarks": list })).into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

/// The time and title of a request, or the original's text for its 400.
fn time_and_title(body: &serde_json::Map<String, Value>, what: &str) -> std::result::Result<(Value, String), &'static str> {
    let time = body.get("time").cloned().unwrap_or(Value::Null);
    if crate::abs::models::media_progress::js_number(Some(&time)).is_none() {
        tracing::warn!("{what}: the time isn't a number");
        return Err("Invalid time");
    }
    match body.get("title") {
        Some(Value::String(t)) if !t.is_empty() => Ok((time, t.clone())),
        _ => Err("Invalid title"),
    }
}

/// `POST /api/me/item/:id/bookmark` (`createBookmark`): a new bookmark; one at the same time is renamed instead.
async fn create(State(st): State<AppState>, me: AbsUser, Path(item): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let fields = crate::abs::auth::routes::body_fields(&headers, &body);
    let run = || -> Result<Response> {
        let mut u = fresh(&st, &me)?;
        if let Err(r) = item_for(&st.abs, &u, &item)? {
            return Ok(r);
        }
        let (time, title) = match time_and_title(&fields, "createBookmark") {
            Ok(v) => v,
            Err(text) => return Ok(send_text(StatusCode::BAD_REQUEST, text)),
        };
        let mut list = list_of(&u);
        let bookmark = if let Some(b) = list.iter_mut().find(|b| is_at(b, &item, &time)) {
            if b.get("title").and_then(Value::as_str) != Some(title.as_str()) {
                b["title"] = json!(title);
            }
            b.clone()
        } else {
            let b = json!({ "libraryItemId": item, "time": time, "title": title, "createdAt": models::now_ms() });
            list.push(b.clone());
            b
        };
        save(&st, &mut u, list)?;
        tell_user_updated(&st, &u)?;
        Ok(Json(bookmark).into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

/// `PATCH /api/me/item/:id/bookmark` (`updateBookmark`): a new title for the bookmark at that time.
async fn rename(State(st): State<AppState>, me: AbsUser, Path(item): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let fields = crate::abs::auth::routes::body_fields(&headers, &body);
    let run = || -> Result<Response> {
        let mut u = fresh(&st, &me)?;
        if let Err(r) = item_for(&st.abs, &u, &item)? {
            return Ok(r);
        }
        let (time, title) = match time_and_title(&fields, "updateBookmark") {
            Ok(v) => v,
            Err(text) => return Ok(send_text(StatusCode::BAD_REQUEST, text)),
        };
        let mut list = list_of(&u);
        let Some(b) = list.iter_mut().find(|b| is_at(b, &item, &time)) else { return Ok(send_status(StatusCode::NOT_FOUND)) };
        b["title"] = json!(title);
        let bookmark = b.clone();
        save(&st, &mut u, list)?;
        tell_user_updated(&st, &u)?;
        Ok(Json(bookmark).into_response())
    };
    run().unwrap_or_else(IntoResponse::into_response)
}

/// `DELETE /api/me/item/:id/bookmark/:time` (`removeBookmark`).
async fn remove(State(st): State<AppState>, me: AbsUser, Path((item, time)): Path<(String, String)>) -> Response {
    let run = || -> Result<Response> {
        let mut u = fresh(&st, &me)?;
        if let Err(r) = item_for(&st.abs, &u, &item)? {
            return Ok(r);
        }
        let Some(time) = crate::abs::models::media_progress::js_number(Some(&json!(time))) else {
            return Ok(send_text(StatusCode::BAD_REQUEST, "Invalid time"));
        };
        let time = json!(time);
        let list = list_of(&u);
        if !list.iter().any(|b| is_at(b, &item, &time)) {
            return Ok(send_status(StatusCode::NOT_FOUND));
        }
        let kept = list.into_iter().filter(|b| !is_at(b, &item, &time)).collect();
        save(&st, &mut u, kept)?;
        tell_user_updated(&st, &u)?;
        Ok(send_status(StatusCode::OK))
    };
    run().unwrap_or_else(IntoResponse::into_response)
}
