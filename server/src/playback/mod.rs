//! Listening and reading: an edition's files sent to players and readers, and each person's place in each book and
//! their bookmarks, kept on the server so every device carries on where another left off (docs/TRACKS.md).
//!
//! ```text
//! GET    /api/library/editions/{id}/files   {files: [{id, name, kind, mime, size, position, duration_ms, chapters}]}
//! GET    /api/files/{id}                    the file's bytes, whole or a Range (206), with ETag and Content-Type
//! GET    /api/progress?since=               the person's places, most recent first: {progress: [...]}
//! GET    /api/progress/{edition_id}         the person's place in one edition; 404 when never opened
//! PUT    /api/progress/{edition_id}         move there (the newest updated_at wins); the place that stands
//! DELETE /api/progress/{edition_id}         start again
//! GET    /api/editions/{id}/bookmarks       the person's bookmarks in it, in book order: {bookmarks: [...]}
//! POST   /api/editions/{id}/bookmarks       {position_ms?, locator?, note}: a new bookmark
//! PATCH  /api/bookmarks/{id}                {note}: change its note
//! DELETE /api/bookmarks/{id}                delete it
//! ```
//!
//! A place is [`store::Progress`]; a bookmark is [`store::Bookmark`]. `GET /api/progress` adds what a shelf shows
//! to each place: the edition's `title`, `format`, `work_id` and `cover_url`.

pub mod files;
pub mod store;

use crate::AppState;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::Request;
use axum::response::Response;
use axum::routing::{get, patch};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SignedIn, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// Makes this part's tables: progress and bookmarks.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    store::migrate(db)
}

/// The filter of the progress list.
#[derive(Debug, Deserialize, Default)]
pub struct ProgressQuery {
    /// Unix seconds: only places changed after it
    #[serde(default)]
    pub since: Option<f64>,
}

/// A change to a bookmark.
#[derive(Debug, Deserialize)]
pub struct BookmarkChange {
    /// the new note
    #[serde(default)]
    pub note: String,
}

/// The routes of files, progress and bookmarks.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/library/editions/{id}/files", get(list_files))
        .route("/api/files/{id}", get(send_file))
        .route("/api/progress", get(list_progress))
        .route("/api/progress/{edition}", get(get_progress).put(put_progress).delete(clear_progress))
        .route("/api/editions/{edition}/bookmarks", get(list_bookmarks).post(add_bookmark))
        .route("/api/bookmarks/{id}", patch(edit_bookmark).delete(delete_bookmark))
}

/// `GET /api/library/editions/{id}/files`.
async fn list_files(State(st): State<AppState>, _: SignedIn, Path(id): Path<i64>) -> Result<Json<Value>> {
    store::edition_exists(&st.db, id)?;
    Ok(Json(json!({ "files": files::edition_files(&st.db, id)? })))
}

/// `GET /api/files/{id}`.
async fn send_file(State(st): State<AppState>, _: SignedIn, Path(id): Path<i64>, req: Request<Body>) -> Result<Response> {
    files::send(&st, id, req).await
}

/// `GET /api/progress`: each place with the edition's title, format, work and cover.
async fn list_progress(State(st): State<AppState>, SignedIn(a): SignedIn, Query(q): Query<ProgressQuery>) -> Result<Json<Value>> {
    let list = store::all_progress(&st.db, a.id, q.since)?;
    let mut out = Vec::with_capacity(list.len());
    for p in list {
        let mut v = json!(p);
        if let Some(r) = st.db.one(
            "SELECT e.title, e.format, e.work_id, w.cover_url FROM editions e JOIN works w ON w.id = e.work_id WHERE e.id=?",
            p![p.edition_id],
        )? {
            v["title"] = json!(r.s("title"));
            v["format"] = json!(r.s("format"));
            v["work_id"] = json!(r.i("work_id"));
            v["cover_url"] = json!(r.s("cover_url"));
        }
        out.push(v);
    }
    Ok(Json(json!({ "progress": out })))
}

/// `GET /api/progress/{edition_id}`.
async fn get_progress(State(st): State<AppState>, SignedIn(a): SignedIn, Path(edition): Path<i64>) -> Result<Json<store::Progress>> {
    store::edition_exists(&st.db, edition)?;
    store::progress(&st.db, a.id, edition)?.map(Json).ok_or_else(|| err(404, "You haven't opened this book yet."))
}

/// `PUT /api/progress/{edition_id}`.
async fn put_progress(
    State(st): State<AppState>,
    SignedIn(a): SignedIn,
    Path(edition): Path<i64>,
    Json(p): Json<store::Progress>,
) -> Result<Json<store::Progress>> {
    Ok(Json(store::save_progress(&st.db, a.id, edition, p)?))
}

/// `DELETE /api/progress/{edition_id}`.
async fn clear_progress(State(st): State<AppState>, SignedIn(a): SignedIn, Path(edition): Path<i64>) -> Result<Json<Value>> {
    store::clear_progress(&st.db, a.id, edition)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/editions/{id}/bookmarks`.
async fn list_bookmarks(State(st): State<AppState>, SignedIn(a): SignedIn, Path(edition): Path<i64>) -> Result<Json<Value>> {
    store::edition_exists(&st.db, edition)?;
    Ok(Json(json!({ "bookmarks": store::bookmarks(&st.db, a.id, edition)? })))
}

/// `POST /api/editions/{id}/bookmarks`.
async fn add_bookmark(
    State(st): State<AppState>,
    SignedIn(a): SignedIn,
    Path(edition): Path<i64>,
    Json(b): Json<store::Bookmark>,
) -> Result<Json<store::Bookmark>> {
    Ok(Json(store::add_bookmark(&st.db, a.id, edition, b)?))
}

/// `PATCH /api/bookmarks/{id}`.
async fn edit_bookmark(
    State(st): State<AppState>,
    SignedIn(a): SignedIn,
    Path(id): Path<i64>,
    Json(c): Json<BookmarkChange>,
) -> Result<Json<store::Bookmark>> {
    Ok(Json(store::edit_bookmark(&st.db, a.id, id, &c.note)?))
}

/// `DELETE /api/bookmarks/{id}`.
async fn delete_bookmark(State(st): State<AppState>, SignedIn(a): SignedIn, Path(id): Path<i64>) -> Result<Json<Value>> {
    store::delete_bookmark(&st.db, a.id, id)?;
    Ok(Json(json!({ "ok": true })))
}
