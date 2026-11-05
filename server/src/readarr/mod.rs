//! The port of Readarr (its final `develop`, 0.4.19.2811, see UPSTREAM.lock): its database and its API v1, module
//! by module, so that apps and scripts made for Readarr work against Folio unchanged. docs/PARITY.md lists every
//! endpoint, setting and job (rows `R`), and docs/TRACKS.md says this is track D's.
//!
//! The data lives in a database of its own, `<data>/readarr.db`, with exactly the original's schema and the rows
//! Readarr makes on first start ([`SCHEMA`], recorded from its last image). An existing Readarr database can
//! therefore be used as it is. Folio's own library tables (phase 1) stay the library of record for files, the
//! wanted list and Lantern; the Readarr modules read and write the Readarr tables the way Readarr does.
//!
//! Answers keep Readarr's shapes, errors included: a missing item is ASP.NET's "Not Found" problem, a failed
//! lookup Readarr's `{message, description}`, a request without the API key a bare 401. Readarr's own clients
//! read these, so the port answers like the original instead of with Folio's `{"detail"}`.

pub mod authors;
pub mod bookfiles;
pub mod books;
pub mod delete;
pub mod disk;
pub mod editions;
pub mod json;
pub mod quality;
pub mod series;
pub mod validation;

use crate::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use kit_server_core::{Db, SqliteDb};
use serde_json::{json, Value};
use std::path::Path;

/// The original's schema at the pinned version (migration 158): tables, indexes, the `VersionInfo` rows and the
/// profiles and quality definitions Readarr makes on first start.
pub const SCHEMA: &str = include_str!("schema.sql");

/// The original's version this port answers as, from UPSTREAM.lock's `readarr` pin.
pub const READARR_VERSION: &str = "0.4.19.2811";

/// Opens (and on first use creates) the Readarr database in the data folder.
pub fn open(data: &Path) -> anyhow::Result<SqliteDb> {
    let db = SqliteDb::open(&data.join("readarr.db")).map_err(|e| anyhow::anyhow!("The Readarr database can't be opened: {}", e.detail))?;
    create(&db).map_err(|e| anyhow::anyhow!("The Readarr database can't be made: {}", e.detail))?;
    Ok(db)
}

/// Creates the original's tables in an empty database; a database that has them already (made by Folio or by
/// Readarr itself) is left as it is.
pub fn create(db: &SqliteDb) -> kit_server_core::Result<()> {
    let made = db.one("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'Authors'", vec![])?;
    if made.is_none() {
        db.execute_batch(SCHEMA)?;
    }
    Ok(())
}

/// Every ported route.
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(authors::router())
        .merge(books::router())
        .merge(editions::router())
        .merge(series::router())
        .merge(bookfiles::router())
}

/// A request Readarr lets in: its API key in `X-Api-Key` or `?apikey=` (Folio's API key, the `api_key` setting),
/// or a signed-in admin of Folio. Anything else is Readarr's bare 401.
pub struct Key;

impl FromRequestParts<AppState> for Key {
    type Rejection = Fail;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> Result<Self, Fail> {
        let want = st.settings.str("api_key");
        let given =
            parts.headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_string).or_else(|| {
                json::query_pairs(parts.uri.query()).into_iter().find(|(k, _)| k.eq_ignore_ascii_case("apikey")).map(|(_, v)| v)
            });
        if let Some(key) = given {
            return if !want.is_empty() && crate::auth::ct_eq(&key, &want) { Ok(Key) } else { Err(Fail::unauthorized()) };
        }
        match crate::auth::signed_in(st, parts) {
            Ok(a) if a.admin => Ok(Key),
            _ => Err(Fail::unauthorized()),
        }
    }
}

/// A Readarr error answer: its status and body (none for the 401).
#[derive(Debug)]
pub struct Fail {
    /// the HTTP status
    pub status: StatusCode,
    /// the JSON body, or nothing
    pub body: Option<Value>,
}

impl Fail {
    /// Readarr's answer to a request without the API key: 401 with no body.
    pub fn unauthorized() -> Fail {
        Fail { status: StatusCode::UNAUTHORIZED, body: None }
    }

    /// ASP.NET's "Not Found" problem, which Readarr answers for a missing item asked by its id.
    pub fn not_found() -> Fail {
        let trace = format!("00-{}-{}-00", hex(16), hex(8));
        Fail {
            status: StatusCode::NOT_FOUND,
            body: Some(json!({
                "type": "https://tools.ietf.org/html/rfc7231#section-6.5.4",
                "title": "Not Found",
                "status": 404,
                "traceId": trace,
            })),
        }
    }

    /// Readarr's answer to an exception: `{message, description}`, where the description names the exception's
    /// type (`kind`, as .NET names it) before the message. Readarr adds the stack trace, which Folio has none of.
    pub fn exception(status: u16, kind: &str, message: impl Into<String>) -> Fail {
        let message = message.into();
        Fail {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body: Some(json!({ "message": message, "description": format!("{kind}: {message}") })),
        }
    }

    /// Readarr's `ModelNotFoundException` (a 404 with its message), e.g. `Author with ID 9 does not exist`.
    pub fn model_not_found(model: &str, id: i64) -> Fail {
        Fail::exception(404, "NzbDrone.Core.Datastore.ModelNotFoundException", format!("{model} with ID {id} does not exist"))
    }
}

impl From<kit_server_core::Error> for Fail {
    fn from(e: kit_server_core::Error) -> Fail {
        Fail::exception(e.status.as_u16(), "System.Exception", e.detail)
    }
}

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        match self.body {
            Some(b) => (self.status, [(header::CONTENT_TYPE, "application/json")], b.to_string()).into_response(),
            None => self.status.into_response(),
        }
    }
}

/// What a Readarr route answers.
pub type Answer<T = axum::Json<Value>> = Result<T, Fail>;

/// A request's JSON body; Readarr's `Request body can't be empty` when there is none, 400 when it isn't JSON.
pub fn body(bytes: &[u8]) -> Answer<Value> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(Fail::exception(500, "NzbDrone.Core.Exceptions.BadRequestException", "Request body can't be empty"));
    }
    serde_json::from_slice(bytes).map_err(|e| Fail::exception(400, "System.Text.Json.JsonException", e.to_string()))
}

/// `n` random bytes as lowercase hex (trace ids).
fn hex(n: usize) -> String {
    use rand::Rng;
    let mut b = vec![0u8; n];
    rand::rng().fill(&mut b[..]);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kit_server_core::RowExt;

    #[test]
    fn the_schema_makes_every_table_once_with_the_first_start_rows() {
        let db = SqliteDb::memory();
        create(&db).unwrap();
        create(&db).unwrap();
        let tables = db.query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'", vec![]).unwrap();
        assert_eq!(tables.len(), 39, "{tables:?}");
        let v = db.one("SELECT MAX(Version) AS v FROM VersionInfo", vec![]).unwrap().unwrap();
        assert_eq!(v.i("v"), 158);
        let profiles = db.query("SELECT Name FROM QualityProfiles ORDER BY Id", vec![]).unwrap();
        assert_eq!(profiles.iter().map(|r| r.s("Name")).collect::<Vec<_>>(), ["eBook", "Spoken"]);
    }
}
