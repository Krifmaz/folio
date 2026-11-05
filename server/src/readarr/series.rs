//! Readarr's series (`Readarr.Api.V1/Series`): the series an author's books are in, with every book of each, as
//! `GET /api/v1/series` sends them.
//!
//! ```text
//! GET /api/v1/series?authorId=1   the author's series (none for an author Readarr doesn't have)
//! ```

use super::json::{object, Query};
use super::{Answer, Key};
use crate::AppState;
use axum::extract::{RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, RowExt};
use serde_json::{json, Value};

/// The series routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/series", get(all))
}

/// `GET /api/v1/series`.
async fn all(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let author = Query::of(q.as_deref()).int("authorId").unwrap_or(0);
    let db = &st.readarr;
    let series = db.query(
        "SELECT DISTINCT s.* FROM Series s JOIN SeriesBookLink l ON s.Id = l.SeriesId JOIN Books b ON l.BookId = b.Id
         JOIN Authors a ON b.AuthorMetadataId = a.AuthorMetadataId WHERE a.Id = ?",
        vec![json!(author)],
    )?;
    let mut out = Vec::new();
    for s in series {
        let links = db.query("SELECT * FROM SeriesBookLink WHERE SeriesId = ? ORDER BY Id", vec![json!(s.i("Id"))])?;
        let links: Vec<Value> = links
            .iter()
            .map(|l| {
                object([
                    ("position", l.get("Position").cloned().unwrap_or(Value::Null)),
                    ("seriesPosition", json!(l.i("SeriesPosition"))),
                    ("seriesId", json!(l.i("SeriesId"))),
                    ("bookId", json!(l.i("BookId"))),
                    ("id", json!(l.i("Id"))),
                ])
            })
            .collect();
        out.push(object([
            ("title", s.get("Title").cloned().unwrap_or(Value::Null)),
            ("description", s.get("Description").cloned().unwrap_or(Value::Null)),
            ("links", Value::Array(links)),
            ("id", json!(s.i("Id"))),
        ]));
    }
    Ok(Json(Value::Array(out)))
}
