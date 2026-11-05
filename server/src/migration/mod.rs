//! Moving in from Audiobookshelf, Calibre, Readarr, LazyLibrarian and Listenarr, with a dry run (the migration track, docs/TRACKS.md).
//!
//! A placeholder until the track's first PR: its tables go under the migrations component `folio-migration` and its routes
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
