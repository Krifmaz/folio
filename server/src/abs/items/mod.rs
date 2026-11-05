//! The port of `server/controllers/LibraryItemController.js` (PARITY rows A0029–A0054, track C): one library
//! item, its files, cover and media. Every item route checks the item first like the original's `middleware`.
//!
//! Done so far: `GET /api/items/:id`, plain or `expanded=1` with `include=progress`.

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::util::Sorting;
use crate::abs::models::{self, book, find, library_item, media_progress, LibraryItem, User};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::Result;
use serde_json::Value;
use std::collections::HashMap;

/// The item routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/items/{id}", get(find_one))
}

/// An item with its book loaded (`LibraryItem.getExpandedById`); `None` when there is no such item or it has no
/// media.
pub fn expanded_by_id(db: &dyn kit_server_core::Db, id: &str) -> Result<Option<(LibraryItem, book::BookFull)>> {
    let Some(item) = find::<LibraryItem>(db, id)? else { return Ok(None) };
    if item.media_type.as_deref() != Some("book") {
        return Ok(None);
    }
    let media_id = item.media_id.clone().unwrap_or_default();
    let mut books = book::load(db, std::slice::from_ref(&media_id))?;
    Ok(books.remove(&media_id).map(|b| (item, b)))
}

/// Whether the user may see this item (`checkCanAccessLibraryItem`).
pub fn can_access(u: &User, item: &LibraryItem, b: &book::BookFull) -> bool {
    let tags = book::strings(&b.book.tags);
    models::user::can_access_item(u, item.library_id.as_deref().unwrap_or_default(), b.book.explicit.unwrap_or(false), &tags)
}

/// The item a route is about, after the original's checks (`LibraryItemController.middleware`): 404 when it
/// doesn't exist, 403 when the user may not see it, or may not change or delete it on a write.
pub fn item_for(
    st: &AppState,
    u: &User,
    id: &str,
    method: &axum::http::Method,
) -> std::result::Result<(LibraryItem, book::BookFull), Box<Response>> {
    let Some((item, b)) = expanded_by_id(&st.abs, id).map_err(|e| Box::new(e.into_response()))? else {
        return Err(Box::new(send_status(StatusCode::NOT_FOUND)));
    };
    if !can_access(u, &item, &b) {
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    if method == axum::http::Method::DELETE && !models::user::can(u, "delete") {
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    if (method == axum::http::Method::PATCH || method == axum::http::Method::POST) && !models::user::can(u, "update") {
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    Ok((item, b))
}

/// `GET /api/items/:id` (A0034, `findOne`): the item, or with `expanded=1` the expanded item, with the user's
/// progress for `include=progress`.
async fn find_one(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let (item, b) = match item_for(&st, &user, &id, &axum::http::Method::GET) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let sorting = Sorting::of(&st.abs_server.settings());
    let include: Vec<&str> = q.get("include").map(|i| i.split(',').collect()).unwrap_or_default();
    if q.get("expanded").and_then(|v| crate::abs::library::js_number(v)) == Some(1.0) {
        let mut j = library_item::expanded(&item, &b, &sorting);
        if include.contains(&"progress") {
            let episode = q.get("episode").filter(|e| !e.is_empty()).cloned();
            let progress = models::user::media_progresses(&st.abs, user.id.as_deref().unwrap_or_default()).map(|all| {
                all.into_iter()
                    .find(|mp| {
                        if episode.as_ref().is_some_and(|e| mp.media_item_id.as_ref() != Some(e)) {
                            return false;
                        }
                        mp.extra_data.as_ref().and_then(|x| x.get("libraryItemId")).and_then(Value::as_str) == item.id.as_deref()
                    })
                    .map(|mp| crate::abs::library::util::js_numbers(media_progress::old_json(&mp)))
            });
            match progress {
                Ok(p) => {
                    j.insert("userMediaProgress".into(), p.unwrap_or(Value::Null));
                }
                Err(e) => return e.into_response(),
            }
        }
        return Json(Value::Object(j)).into_response();
    }
    Json(library_item::old_json(&item, &b)).into_response()
}
