//! Readarr's editions (`Readarr.Api.V1/Editions`): the editions of some books, as `GET /api/v1/edition` sends
//! them.
//!
//! ```text
//! GET /api/v1/edition?bookId=1&bookId=2   the editions of these books (none without a bookId)
//! ```

use super::json::{date, list, marks, object, ratings, Query};
use super::{Answer, Key};
use crate::AppState;
use axum::extract::{RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Row, RowExt};
use serde_json::{json, Value};

/// The edition routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/edition", get(all))
}

/// An edition as `EditionResource.ToResource` makes it (covers as stored, an empty overview when it has none).
pub fn resource(e: &Row) -> Value {
    object([
        ("bookId", json!(e.i("BookId"))),
        ("foreignEditionId", e.get("ForeignEditionId").cloned().unwrap_or(Value::Null)),
        ("titleSlug", e.get("TitleSlug").cloned().unwrap_or(Value::Null)),
        ("isbn13", e.get("Isbn13").cloned().unwrap_or(Value::Null)),
        ("asin", e.get("Asin").cloned().unwrap_or(Value::Null)),
        ("title", e.get("Title").cloned().unwrap_or(Value::Null)),
        ("language", e.get("Language").cloned().unwrap_or(Value::Null)),
        ("overview", json!(e.s("Overview"))),
        ("format", e.get("Format").cloned().unwrap_or(Value::Null)),
        ("isEbook", json!(e.i("IsEbook") != 0)),
        ("disambiguation", e.get("Disambiguation").cloned().unwrap_or(Value::Null)),
        ("publisher", e.get("Publisher").cloned().unwrap_or(Value::Null)),
        ("pageCount", json!(e.i("PageCount"))),
        ("releaseDate", date(e.get("ReleaseDate"))),
        ("images", Value::Array(super::json::images(e.get("Images")))),
        ("links", list(e.get("Links"))),
        ("ratings", ratings(e.get("Ratings"))),
        ("monitored", json!(e.i("Monitored") != 0)),
        ("manualAdd", json!(e.i("ManualAdd") != 0)),
        ("grabbed", json!(false)),
        ("id", json!(e.i("Id"))),
    ])
}

/// `GET /api/v1/edition`.
async fn all(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let ids = Query::of(q.as_deref()).ints("bookId");
    if ids.is_empty() {
        return Ok(Json(json!([])));
    }
    let sql = format!("SELECT * FROM Editions WHERE BookId IN ({}) ORDER BY Id", marks(ids.len()));
    let rows = st.readarr.query(&sql, ids.iter().map(|i| json!(i)).collect())?;
    Ok(Json(Value::Array(rows.iter().map(resource).collect())))
}
