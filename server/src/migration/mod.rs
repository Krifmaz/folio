//! Moving in from Audiobookshelf, Calibre (and Calibre-Web), Readarr, LazyLibrarian and Listenarr, with a dry run
//! first (the migration track, docs/TRACKS.md).
//!
//! The source is read by the `folio-migrate` crate, which never changes it; [`apply`] matches what it read to
//! exact editions and brings it in.
//!
//! ```text
//! GET  /api/migrate/sources        (admins) the sources and what each gives: {sources: [{key, name, file_name, gives}]}
//! POST /api/migrate/runs {source, path, dry_run = true, users?: {name: account id}}
//!                                  (admins) read the source now (400 with the reason when it can't be), then check
//!                                  it (dry run) or bring it in, in the background: the new run
//! GET  /api/migrate/runs           (admins) every run, newest first: {runs: [...]}
//! GET  /api/migrate/runs/{id}      (admins) one run, how far it got, and its report when done
//! GET  /api/migrate/waiting        (admins) migrated books whose files wait for the import: {books: [...]}
//! ```

pub mod apply;
pub mod store;

use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{err, Admin, Result, SqliteDb};
use migrate::Source;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// Makes the migration's tables, and ends runs a stopped server left going.
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    store::migrate(db)?;
    store::end_stale(db).map_err(|e| anyhow::anyhow!("The migration runs can't be read: {}", e.detail))
}

/// The migration's routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/migrate/sources", get(sources))
        .route("/api/migrate/runs", get(list).post(start))
        .route("/api/migrate/runs/{id}", get(one))
        .route("/api/migrate/waiting", get(waiting))
}

/// A new run.
#[derive(Debug, Deserialize)]
pub struct Start {
    /// the source's key or name (`abs`, `Calibre`)
    pub source: String,
    /// its database file, or the folder that holds it, as Folio sees it
    pub path: String,
    /// only look (the default) or bring everything in
    #[serde(default = "yes")]
    pub dry_run: bool,
    /// a source user name → the Folio account their progress goes to (0: leave it out)
    #[serde(default)]
    pub users: HashMap<String, i64>,
}

/// `true`, the default of [`Start::dry_run`].
fn yes() -> bool {
    true
}

/// What each source gives, in plain words.
fn gives(s: Source) -> &'static str {
    match s {
        Source::Audiobookshelf => "Audiobooks and ebooks with their files, everyone's listening and reading progress, and bookmarks.",
        Source::Calibre => "Ebooks (and any audiobooks) with their files. Bring this in before Calibre-Web.",
        Source::CalibreWeb => "Who finished or is reading which Calibre book, and how far.",
        Source::Readarr => "The wanted list: monitored books that have no file yet.",
        Source::LazyLibrarian => "The wanted lists of ebooks and audiobooks.",
        Source::Listenarr => "The wanted list of audiobooks.",
    }
}

/// `GET /api/migrate/sources`.
async fn sources(_: Admin) -> Json<Value> {
    let list: Vec<Value> =
        Source::ALL.iter().map(|s| json!({ "key": s.key(), "name": s.name(), "file_name": s.file_name(), "gives": gives(*s) })).collect();
    Json(json!({ "sources": list }))
}

/// `POST /api/migrate/runs`: reads the source (so a wrong path is answered at once), then runs in the background.
async fn start(State(st): State<AppState>, Admin(a): Admin, Json(s): Json<Start>) -> Result<Json<Value>> {
    let source = Source::parse(&s.source).ok_or_else(|| {
        err(400, "Choose the app to bring books in from: Audiobookshelf, Calibre, Calibre-Web, Readarr, LazyLibrarian or Listenarr.")
    })?;
    let path = PathBuf::from(s.path.trim());
    if s.path.trim().is_empty() {
        return Err(err(400, format!("Give the place of {source}'s {} file, as Folio sees it.", source.file_name())));
    }
    let scratch = st.data_dir.join("tmp").join("migrate");
    let read_path = path.clone();
    let plan = tokio::task::spawn_blocking(move || migrate::read(source, &read_path, &scratch))
        .await
        .map_err(|e| err(500, format!("Reading the source stopped: {e}. Try again.")))?
        .map_err(|e| err(400, e.0))?;
    let id = store::start(&st.db, source.key(), &path.to_string_lossy(), s.dry_run, a.id)?;
    let opts = apply::Options { dry_run: s.dry_run, users: s.users };
    let bg = st.clone();
    tokio::spawn(async move {
        let outcome = apply::apply(&bg, id, plan, &opts, a.id).await;
        let saved = match outcome {
            Ok(report) => store::finish(&bg.db, id, &serde_json::to_value(&report).unwrap_or(Value::Null)),
            Err(e) => store::fail(&bg.db, id, &e.detail),
        };
        if let Err(e) = saved {
            tracing::error!("migration run {id} can't be saved: {}", e.detail);
        }
    });
    Ok(Json(store::run(&st.db, id)?))
}

/// `GET /api/migrate/runs`.
async fn list(State(st): State<AppState>, _: Admin) -> Result<Json<Value>> {
    Ok(Json(json!({ "runs": store::runs(&st.db)? })))
}

/// `GET /api/migrate/runs/{id}`.
async fn one(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    Ok(Json(store::run(&st.db, id)?))
}

/// `GET /api/migrate/waiting`.
async fn waiting(State(st): State<AppState>, _: Admin) -> Result<Json<Value>> {
    Ok(Json(json!({ "books": store::waiting(&st.db)? })))
}
