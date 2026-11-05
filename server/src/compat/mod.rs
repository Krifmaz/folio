//! Other apps' protocols: the Audiobookshelf API, OPDS, KOReader sync and Kobo sync (the compatibility track, docs/TRACKS.md).
//!
//! A placeholder until the track's first PR: its tables go under the migrations component `folio-compat` and its routes
//! into [`router`].

use crate::AppState;
use axum::Router;
use kit_server_core::SqliteDb;

/// Makes this part's tables (none yet).
pub fn migrate(_db: &SqliteDb) -> anyhow::Result<()> {
    Ok(())
}

/// This part's routes (none yet).
pub fn router() -> Router<AppState> {
    Router::new()
}
