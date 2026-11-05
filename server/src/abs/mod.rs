//! The port of Audiobookshelf's server (v2.37.2, see UPSTREAM.lock): its database, its API and its socket,
//! module by module, so that Audiobookshelf's own web and mobile apps (forked in `abs-web/` and `abs-app/`) run
//! against Folio unchanged. docs/PARITY.md lists every endpoint, setting and job, and docs/TRACKS.md which track
//! ports which module.
//!
//! The data lives in a database of its own, `<data>/absdatabase.sqlite`, with exactly the original's schema
//! ([`SCHEMA`], recorded from the original's image). An existing Audiobookshelf database can therefore be used as
//! it is.

pub mod models;
pub mod socket;

use crate::AppState;
use axum::Router;
use kit_server_core::{Db, SqliteDb};
use std::path::Path;

/// The original's schema at the pinned version: tables, indexes, triggers and the `migrationsMeta` rows.
pub const SCHEMA: &str = include_str!("schema.sql");

/// The original's version this port answers as (`serverVersion`), from UPSTREAM.lock's `abs-server` pin.
pub const ABS_VERSION: &str = "2.37.2";

/// Opens (and on first use creates) the Audiobookshelf database in the data folder.
pub fn open(data: &Path) -> anyhow::Result<SqliteDb> {
    let db = SqliteDb::open(&data.join("absdatabase.sqlite"))
        .map_err(|e| anyhow::anyhow!("The Audiobookshelf database can't be opened: {}", e.detail))?;
    create(&db).map_err(|e| anyhow::anyhow!("The Audiobookshelf database can't be made: {}", e.detail))?;
    Ok(db)
}

/// Creates the original's tables in an empty database; a database that has them already (made by Folio or by
/// Audiobookshelf itself) is left as it is.
pub fn create(db: &SqliteDb) -> kit_server_core::Result<()> {
    let made = db.one("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'libraryItems'", vec![])?;
    if made.is_none() {
        db.execute_batch(SCHEMA)?;
    }
    Ok(())
}

/// The user a socket's token signs in as, or the original's message for `auth_failed`. Tokens are checked by the
/// port of `TokenManager` (track core, PARITY P2); until it lands every token is refused.
pub async fn socket_user(_st: &AppState, _token: &str) -> Result<models::User, &'static str> {
    Err("Invalid token")
}

/// Every ported route. Each track adds its module's router here (one line each, docs/TRACKS.md).
pub fn router() -> Router<AppState> {
    Router::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kit_server_core::RowExt;

    #[test]
    fn the_schema_makes_every_table_once() {
        let db = SqliteDb::memory();
        create(&db).unwrap();
        create(&db).unwrap();
        let tables = db.query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name", vec![]).unwrap();
        let names: Vec<String> = tables.iter().map(|r| r.s("name")).collect();
        assert_eq!(names.len(), 26, "{names:?}");
        for t in ["libraryItems", "users", "mediaProgresses", "playbackSessions", "feeds", "apiKeys"] {
            assert!(names.iter().any(|n| n == t), "{t} missing");
        }
        let v = db.one("SELECT value FROM migrationsMeta WHERE key = 'version'", vec![]).unwrap().unwrap();
        assert_eq!(v.s("value"), ABS_VERSION);
    }
}
