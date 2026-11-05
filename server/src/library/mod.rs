//! The library catalogue: works and their exact editions, with authors and series, and the routes that look
//! editions up and keep them.
//!
//! ```text
//! GET    /api/library/lookup?asin=…|isbn=…   (admins) the edition as the sources describe it, not kept
//! POST   /api/library/editions {asin}|{isbn} (admins) look the edition up and keep it → {work, edition, created}
//! GET    /api/library/works?q=&format=       the works, newest first: {works: [...]}
//! GET    /api/library/works/{id}             one work with its authors, series and editions
//! GET    /api/library/editions/{id}          one edition
//! POST   /api/library/editions/{id}/refresh  (admins) ask the sources again
//! DELETE /api/library/editions/{id}          (admins) forget the edition (and its work when it was the last)
//! GET    /api/library/authors                every author: {authors: [...]}
//! GET    /api/library/series                 every series with its works in order: {series: [...]}
//! ```

pub mod ids;
pub mod store;
pub mod text;

use crate::metadata::{EditionMeta, Format, Providers};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{err, Admin, Result, SignedIn};
use serde::Deserialize;
use serde_json::{json, Value};

/// An edition to look up or add: an Audible ASIN (an audiobook) or an ISBN (an ebook).
#[derive(Debug, Deserialize, Default)]
pub struct EditionId {
    /// the Audible ASIN
    #[serde(default)]
    pub asin: String,
    /// the ISBN, 10 or 13 digits
    #[serde(default)]
    pub isbn: String,
}

/// The filters of the works list.
#[derive(Debug, Deserialize, Default)]
pub struct WorksQuery {
    /// words of the title or an author's name
    #[serde(default)]
    pub q: String,
    /// `audiobook` or `ebook`: only works with an edition in it
    #[serde(default)]
    pub format: String,
}

/// The library's routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/library/lookup", get(lookup))
        .route("/api/library/editions", post(add))
        .route("/api/library/works", get(list_works))
        .route("/api/library/works/{id}", get(get_work))
        .route("/api/library/editions/{id}", get(get_edition).delete(delete_edition))
        .route("/api/library/editions/{id}/refresh", post(refresh))
        .route("/api/library/authors", get(list_authors))
        .route("/api/library/series", get(list_series))
}

/// The sources as the settings say now.
pub fn providers(st: &AppState) -> Providers {
    Providers::from_settings(st.http.clone(), &st.settings)
}

/// Looks the edition up: by ASIN when one is given, else by ISBN.
pub async fn look_up(st: &AppState, id: &EditionId) -> Result<EditionMeta> {
    let p = providers(st);
    match (id.asin.trim(), id.isbn.trim()) {
        (asin, _) if !asin.is_empty() => p.audiobook(asin).await,
        (_, isbn) if !isbn.is_empty() => p.ebook(isbn).await,
        _ => Err(err(400, "Give an Audible ASIN for an audiobook, or an ISBN for an ebook.")),
    }
}

/// `GET /api/library/lookup`: the edition as the sources describe it, with the work it would join.
async fn lookup(State(st): State<AppState>, _: Admin, Query(id): Query<EditionId>) -> Result<Json<Value>> {
    let m = look_up(&st, &id).await?;
    let kept = store::find_edition(&st.db, &m)?.map(|(e, w)| json!({ "edition_id": e, "work_id": w }));
    Ok(Json(json!({ "edition": m, "kept": kept })))
}

/// `POST /api/library/editions`: looks the edition up and keeps it.
async fn add(State(st): State<AppState>, _: Admin, Json(id): Json<EditionId>) -> Result<Json<Value>> {
    let m = look_up(&st, &id).await?;
    let saved = store::save(&st.db, &m)?;
    Ok(Json(json!({ "work": store::work(&st.db, saved.work_id)?, "edition_id": saved.edition_id, "created": saved.created })))
}

/// `GET /api/library/works`.
async fn list_works(State(st): State<AppState>, _: SignedIn, Query(q): Query<WorksQuery>) -> Result<Json<Value>> {
    let format = if q.format.is_empty() {
        None
    } else {
        Some(Format::parse(&q.format).ok_or_else(|| err(400, "The format is audiobook or ebook."))?)
    };
    Ok(Json(json!({ "works": store::works(&st.db, &q.q, format)? })))
}

/// `GET /api/library/works/{id}`.
async fn get_work(State(st): State<AppState>, _: SignedIn, Path(id): Path<i64>) -> Result<Json<Value>> {
    Ok(Json(store::work(&st.db, id)?))
}

/// `GET /api/library/editions/{id}`.
async fn get_edition(State(st): State<AppState>, _: SignedIn, Path(id): Path<i64>) -> Result<Json<Value>> {
    let (row, _) = store::edition(&st.db, id)?;
    Ok(Json(store::edition_json(&row)))
}

/// `POST /api/library/editions/{id}/refresh`: asks the edition's source again and keeps the new answer.
async fn refresh(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    let (_, old) = store::edition(&st.db, id)?;
    let ask = match old.format {
        Format::Audiobook => EditionId { asin: old.asin.clone(), ..Default::default() },
        Format::Ebook => EditionId { isbn: old.isbn13.clone(), ..Default::default() },
    };
    let m = look_up(&st, &ask).await?;
    let saved = store::save(&st.db, &m)?;
    let (row, _) = store::edition(&st.db, saved.edition_id)?;
    Ok(Json(store::edition_json(&row)))
}

/// `DELETE /api/library/editions/{id}`.
async fn delete_edition(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    store::delete_edition(&st.db, id)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/library/authors`.
async fn list_authors(State(st): State<AppState>, _: SignedIn) -> Result<Json<Value>> {
    Ok(Json(json!({ "authors": store::authors(&st.db)? })))
}

/// `GET /api/library/series`.
async fn list_series(State(st): State<AppState>, _: SignedIn) -> Result<Json<Value>> {
    Ok(Json(json!({ "series": store::series(&st.db)? })))
}
