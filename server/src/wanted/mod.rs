//! The wanted list, the preferences that say what a good copy is, and following authors and series.
//!
//! ```text
//! GET    /api/wanted?format=&status=          the wanted list, newest first: {wanted: [...]}
//! POST   /api/wanted {edition_id, profile_id?} (admins) want an edition (400 with the reason when the preferences refuse it)
//! PATCH  /api/wanted/{id} {profile_id}        (admins) other preferences for it
//! DELETE /api/wanted/{id}                     (admins) take it off the list
//! GET    /api/profiles                        the preferences: {profiles: [...], file_formats: {audiobook: [...], ebook: [...]}}
//! PUT    /api/profiles/{id}                   (admins) change them
//! PUT    /api/library/authors/{id}/follow {follow}  (admins) "", "audiobook", "ebook" or "both"
//! PUT    /api/library/series/{id}/follow {follow}   (admins) the same for a series
//! POST   /api/follow/check                    (admins) check the followed authors and series now: {added, looked_at, problems}
//! ```

pub mod follow;
pub mod store;

use crate::metadata::Format;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kit_server_core::{err, p, Admin, Db, Result, SignedIn};
use serde::Deserialize;
use serde_json::{json, Value};

/// The wanted list's filters.
#[derive(Debug, Deserialize, Default)]
pub struct WantedQuery {
    /// `audiobook` or `ebook`
    #[serde(default)]
    pub format: String,
    /// one status (`wanted`, `searching` …)
    #[serde(default)]
    pub status: String,
}

/// A new wanted entry.
#[derive(Debug, Deserialize)]
pub struct Want {
    /// the edition
    pub edition_id: i64,
    /// the preferences; the format's first when missing
    #[serde(default)]
    pub profile_id: Option<i64>,
}

/// A change to a wanted entry.
#[derive(Debug, Deserialize)]
pub struct Change {
    /// other preferences
    pub profile_id: i64,
}

/// Following an author or a series.
#[derive(Debug, Deserialize)]
pub struct Follow {
    /// `""` (stop), `audiobook`, `ebook` or `both`
    #[serde(default)]
    pub follow: String,
}

/// The wanted list's routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/wanted", get(list).post(add))
        .route("/api/wanted/{id}", axum::routing::patch(change).delete(remove))
        .route("/api/profiles", get(profiles))
        .route("/api/profiles/{id}", put(update_profile))
        .route("/api/library/authors/{id}/follow", put(follow_author))
        .route("/api/library/series/{id}/follow", put(follow_series))
        .route("/api/follow/check", post(check))
}

/// The format of a query, `None` when empty; 400 for another word.
fn format_of(s: &str) -> Result<Option<Format>> {
    if s.is_empty() {
        return Ok(None);
    }
    Format::parse(s).map(Some).ok_or_else(|| err(400, "The format is audiobook or ebook."))
}

/// `GET /api/wanted`.
async fn list(State(st): State<AppState>, _: SignedIn, Query(q): Query<WantedQuery>) -> Result<Json<Value>> {
    Ok(Json(json!({ "wanted": store::wanted(&st.db, format_of(&q.format)?, &q.status)? })))
}

/// `POST /api/wanted`.
async fn add(State(st): State<AppState>, Admin(a): Admin, Json(w): Json<Want>) -> Result<Json<Value>> {
    Ok(Json(store::want(&st.db, w.edition_id, w.profile_id, a.id, "manual")?))
}

/// `PATCH /api/wanted/{id}`.
async fn change(State(st): State<AppState>, _: Admin, Path(id): Path<i64>, Json(c): Json<Change>) -> Result<Json<Value>> {
    Ok(Json(store::set_profile(&st.db, id, c.profile_id)?))
}

/// `DELETE /api/wanted/{id}`.
async fn remove(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    store::unwant(&st.db, id)?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/profiles`.
async fn profiles(State(st): State<AppState>, _: SignedIn) -> Result<Json<Value>> {
    Ok(Json(json!({
        "profiles": store::profiles(&st.db)?,
        "file_formats": { "audiobook": store::known_file_formats(Format::Audiobook), "ebook": store::known_file_formats(Format::Ebook) },
    })))
}

/// `PUT /api/profiles/{id}`.
async fn update_profile(State(st): State<AppState>, _: Admin, Path(id): Path<i64>, Json(p): Json<store::Profile>) -> Result<Json<Value>> {
    Ok(Json(json!(store::update_profile(&st.db, id, p)?)))
}

/// Sets `follow` on the row `id` of `table` (authors or series).
fn set_follow(st: &AppState, table: &str, id: i64, follow: &str) -> Result<Json<Value>> {
    if !["", "audiobook", "ebook", "both"].contains(&follow) {
        return Err(err(400, "Follow audiobook, ebook or both, or nothing to stop."));
    }
    let what = if table == "authors" { "author" } else { "series" };
    if st.db.one(&format!("SELECT id FROM {table} WHERE id=?"), p![id])?.is_none() {
        return Err(err(404, format!("That {what} isn't in the library any more.")));
    }
    st.db.execute(&format!("UPDATE {table} SET follow=? WHERE id=?"), p![follow, id])?;
    Ok(Json(json!({ "id": id, "follow": follow })))
}

/// `PUT /api/library/authors/{id}/follow`.
async fn follow_author(State(st): State<AppState>, _: Admin, Path(id): Path<i64>, Json(f): Json<Follow>) -> Result<Json<Value>> {
    set_follow(&st, "authors", id, &f.follow)
}

/// `PUT /api/library/series/{id}/follow`.
async fn follow_series(State(st): State<AppState>, _: Admin, Path(id): Path<i64>, Json(f): Json<Follow>) -> Result<Json<Value>> {
    set_follow(&st, "series", id, &f.follow)
}

/// `POST /api/follow/check`.
async fn check(State(st): State<AppState>, _: Admin) -> Result<Json<Value>> {
    Ok(Json(json!(follow::check(&st).await?)))
}
