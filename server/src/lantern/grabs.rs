//! Searching Lantern for wanted editions, grabbing the best result, and following each grab until its files
//! arrive.
//!
//! ```text
//! GET  /api/wanted/{id}/releases           (admins) every result Lantern finds, scored with the reasons
//! POST /api/wanted/{id}/search             (admins) search and grab the best result if it scores high enough
//! POST /api/wanted/{id}/grab {guid, indexer_id, force?} (admins) grab this result (force: even one Folio refused)
//! GET  /api/grabs                          the grabs, newest first
//! POST /api/lantern/test                   (admins) reach Lantern, sign Folio up with it
//! POST /api/lantern/callback/{token}       Lantern tells Folio how a grab is going
//! ```
//!
//! Wanted entries move `wanted` → `searching` → `downloading` → `downloaded` (files arrived; verification
//! takes over) or back to `wanted` with a note when nothing good was found, or `failed` when the download failed.

use super::score::{self, Scored};
use super::{GrabState, Lantern};
use crate::metadata::{EditionMeta, Format};
use crate::wanted::store as wanted;
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::db::now;
use kit_server_core::{err, p, Admin, Db, Migrations, Result, Row, RowExt, SignedIn, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};

/// The grabs' table, step by step (a released step is never changed).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-grabs",
    steps: &[
        "CREATE TABLE grabs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            wanted_id INTEGER NOT NULL,
            lantern_id TEXT NOT NULL DEFAULT '',
            guid TEXT NOT NULL,
            indexer_id INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            indexer TEXT NOT NULL DEFAULT '',
            size INTEGER NOT NULL DEFAULT 0,
            score INTEGER NOT NULL DEFAULT 0,
            reasons TEXT NOT NULL DEFAULT '[]',
            file_format TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'queued',
            progress REAL NOT NULL DEFAULT 0,
            path TEXT NOT NULL DEFAULT '',
            files TEXT NOT NULL DEFAULT '[]',
            error TEXT NOT NULL DEFAULT '',
            created_at REAL NOT NULL,
            updated_at REAL NOT NULL
        );
        CREATE INDEX grabs_wanted ON grabs (wanted_id);
        CREATE INDEX grabs_status ON grabs (status);
        CREATE TABLE folio_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        // 2: the verdict on what arrived (verify), and the downloads someone rejected, never grabbed again for
        // the same book
        "ALTER TABLE grabs ADD COLUMN verdict TEXT NOT NULL DEFAULT '';
        ALTER TABLE grabs ADD COLUMN confidence INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE rejected_releases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            wanted_id INTEGER NOT NULL,
            guid TEXT NOT NULL,
            title TEXT NOT NULL DEFAULT '',
            reason TEXT NOT NULL DEFAULT '',
            user_id INTEGER NOT NULL DEFAULT 0,
            created_at REAL NOT NULL
        );
        CREATE INDEX rejected_releases_wanted ON rejected_releases (wanted_id, guid);",
    ],
};

/// Makes the grabs' table (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The grabs table can't be made: {}", e.detail))
}

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/wanted/{id}/releases", get(releases))
        .route("/api/wanted/{id}/search", post(search_now))
        .route("/api/wanted/{id}/grab", post(grab_one))
        .route("/api/grabs", get(list))
        .route("/api/lantern/test", post(test))
        .route("/api/lantern/callback/{token}", post(callback))
}

/// Lantern as the settings say now.
pub fn lantern(st: &AppState) -> Result<Lantern> {
    Lantern::from_settings(st.http.clone(), &st.settings)
}

/// The secret part of the address Lantern calls back to, made once and kept.
pub fn callback_token(db: &SqliteDb) -> Result<String> {
    if let Some(r) = db.one("SELECT value FROM folio_kv WHERE key='lantern_callback'", p![])? {
        return Ok(r.s("value"));
    }
    let t = crate::auth::random_token(24);
    db.execute("INSERT OR IGNORE INTO folio_kv (key, value) VALUES ('lantern_callback', ?)", p![t])?;
    Ok(db.one("SELECT value FROM folio_kv WHERE key='lantern_callback'", p![])?.map(|r| r.s("value")).unwrap_or(t))
}

/// The address Lantern calls back to: `<public_url>/api/lantern/callback/<token>`; "" when Folio's own
/// address isn't set (Folio then asks Lantern every minute).
pub fn callback_url(st: &AppState) -> Result<String> {
    let base = st.settings.str("public_url").trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Ok(String::new());
    }
    Ok(format!("{base}/api/lantern/callback/{}", callback_token(&st.db)?))
}

/// A wanted entry's edition and preferences.
pub fn wanted_parts(db: &SqliteDb, wanted_id: i64) -> Result<(Row, EditionMeta, wanted::Profile)> {
    let w = db
        .one("SELECT * FROM wanted WHERE id=?", p![wanted_id])?
        .ok_or_else(|| err(404, "That book isn't on the wanted list any more."))?;
    let (_, m) = crate::library::store::edition(db, w.i("edition_id"))?;
    let pr = wanted::profile(db, w.i("profile_id"))?;
    Ok((w, m, pr))
}

/// Every result Lantern has for the edition, from all its searches, scored best first. Searches that fail are
/// skipped while others answer; when all fail, the first failure.
/// Results someone rejected for the wanted entry stay refused.
pub async fn find(db: &SqliteDb, wanted_id: i64, l: &Lantern, m: &EditionMeta, pr: &wanted::Profile) -> Result<Vec<Scored>> {
    let mut all = Vec::new();
    let mut first_err = None;
    let mut answered = false;
    for q in score::queries(m) {
        match l.search(m.format, &q).await {
            Ok(r) => {
                answered = true;
                all.extend(r);
            }
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if !answered {
        if let Some(e) = first_err {
            return Err(e);
        }
    }
    let rejected: std::collections::HashSet<String> =
        db.query("SELECT guid FROM rejected_releases WHERE wanted_id=?", p![wanted_id])?.iter().map(|r| r.s("guid")).collect();
    let mut ranked = score::rank(m, pr, all);
    for s in ranked.iter_mut().filter(|s| rejected.contains(&s.release.guid)) {
        s.refused = Some("You rejected this download for this book before.".into());
    }
    ranked.sort_by_key(|s| s.refused.is_some());
    Ok(ranked)
}

/// Sends the result to Lantern and keeps the grab; the wanted entry becomes `downloading`.
async fn start_grab(st: &AppState, l: &Lantern, wanted_id: i64, format: Format, s: &Scored) -> Result<Value> {
    if let Ok(cb) = callback_url(st) {
        // signing up again is harmless; it keeps Lantern's callback address current
        let _ = l.register(&cb).await;
    }
    let g = l.grab(format, &s.release, &format!("wanted-{wanted_id}")).await?;
    let t = now();
    let id = st.db.execute(
        "INSERT INTO grabs (wanted_id, lantern_id, guid, indexer_id, title, indexer, size, score, reasons, file_format, status, progress, created_at, updated_at)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        p![wanted_id, g.id, s.release.guid, s.release.indexer_id, s.release.title, s.release.indexer, s.release.size as i64, s.score, json!(s.reasons).to_string(), s.file_format, if g.status.is_empty() { "queued" } else { g.status.as_str() }, g.progress, t, t],
    )?;
    wanted::set_status(
        &st.db,
        wanted_id,
        "downloading",
        &format!(
            "Downloading \"{}\" from {} (score {}).",
            s.release.title,
            if s.release.indexer.is_empty() { "Lantern" } else { &s.release.indexer },
            s.score
        ),
    )?;
    grab_json(&st.db, id)
}

/// Searches for the wanted entry and grabs the best result when it scores at least `min_score`; otherwise the
/// entry goes back to `wanted` with a note saying why. `{grab, best}`.
pub async fn search_and_grab(st: &AppState, wanted_id: i64) -> Result<Value> {
    let (w, m, pr) = wanted_parts(&st.db, wanted_id)?;
    match w.s("status").as_str() {
        "downloading" | "downloaded" | "verified" | "imported" => {
            return Err(err(409, "This book is already on its way or in the library."));
        }
        "review" => return Err(err(409, "A download of this book is waiting for a look under Review. Accept or reject it first.")),
        _ => {}
    }
    let l = lantern(st)?;
    wanted::set_status(&st.db, wanted_id, "searching", "")?;
    st.db.execute("UPDATE wanted SET searched_at=? WHERE id=?", p![now(), wanted_id])?;
    let found = match find(&st.db, wanted_id, &l, &m, &pr).await {
        Ok(f) => f,
        Err(e) => {
            wanted::set_status(&st.db, wanted_id, "wanted", &e.detail)?;
            return Err(e);
        }
    };
    let min = st.settings.int("min_score").clamp(1, 100) as u32;
    let best: Vec<&Scored> = found.iter().take(5).collect();
    match found.iter().find(|s| s.refused.is_none() && s.score >= min) {
        Some(s) => {
            let g = start_grab(st, &l, wanted_id, m.format, s).await?;
            Ok(json!({ "grab": g, "best": best }))
        }
        None => {
            let note = match found.iter().find(|s| s.refused.is_none()) {
                Some(s) => {
                    format!("Nothing good enough yet: the best result, \"{}\", scores {} of the {min} needed.", s.release.title, s.score)
                }
                None if found.is_empty() => "Lantern found nothing for this edition yet.".to_string(),
                None => format!(
                    "Lantern found {} results, and none of them will do. The closest: \"{}\" — {}",
                    found.len(),
                    found[0].release.title,
                    found[0].refused.clone().unwrap_or_default()
                ),
            };
            wanted::set_status(&st.db, wanted_id, "wanted", &note)?;
            Ok(json!({ "grab": null, "best": best, "note": note }))
        }
    }
}

/// A grab as clients get it.
pub fn grab_row_json(r: &Row) -> Value {
    json!({
        "id": r.i("id"),
        "wanted_id": r.i("wanted_id"),
        "lantern_id": r.s("lantern_id"),
        "title": r.s("title"),
        "indexer": r.s("indexer"),
        "size": r.i("size"),
        "score": r.i("score"),
        "reasons": serde_json::from_str::<Value>(&r.s("reasons")).unwrap_or(json!([])),
        "file_format": r.s("file_format"),
        "status": r.s("status"),
        "progress": r.f("progress"),
        "path": r.s("path"),
        "files": serde_json::from_str::<Value>(&r.s("files")).unwrap_or(json!([])),
        "error": r.s("error"),
        "confidence": r.i("confidence"),
        "verdict": serde_json::from_str::<Value>(&r.s("verdict")).unwrap_or(Value::Null),
        "created_at": r.f("created_at"),
        "updated_at": r.f("updated_at"),
    })
}

/// The grab `id`.
fn grab_json(db: &SqliteDb, id: i64) -> Result<Value> {
    db.one("SELECT * FROM grabs WHERE id=?", p![id])?
        .map(|r| grab_row_json(&r))
        .ok_or_else(|| err(404, "That download isn't known any more."))
}

/// Records what Lantern says about a grab; a finished one moves its wanted entry to `downloaded` (files
/// arrived, to be checked by [`crate::verify::pending`]) and a failed one to `failed` with Lantern's reason.
pub fn apply(db: &SqliteDb, g: &GrabState) -> Result<bool> {
    let Some(row) = db.one("SELECT * FROM grabs WHERE lantern_id=?", p![g.id])? else { return Ok(false) };
    // once checked (or accepted or rejected), what Lantern says doesn't change it again
    if matches!(row.s("status").as_str(), "verified" | "review" | "rejected" | "imported") {
        return Ok(true);
    }
    let status = match g.status.as_str() {
        "queued" | "downloading" | "completed" | "failed" => g.status.as_str(),
        _ => "downloading",
    };
    db.execute(
        "UPDATE grabs SET status=?, progress=?, path=?, files=?, error=?, updated_at=? WHERE id=?",
        p![status, g.progress.clamp(0.0, 1.0), g.path, json!(g.files).to_string(), g.error, now(), row.i("id")],
    )?;
    let wanted_id = row.i("wanted_id");
    match status {
        "completed" => wanted::set_status(
            db,
            wanted_id,
            "downloaded",
            &format!("\"{}\" arrived: {} files in {}.", row.s("title"), g.files.len(), g.path),
        )?,
        "failed" => wanted::set_status(
            db,
            wanted_id,
            "failed",
            &format!("Lantern couldn't download \"{}\": {}", row.s("title"), if g.error.is_empty() { "no reason given" } else { &g.error }),
        )?,
        _ => {}
    }
    Ok(true)
}

/// Asks Lantern about every grab still under way.
pub async fn poll(st: &AppState) -> Result<usize> {
    let Ok(l) = lantern(st) else { return Ok(0) };
    let mut n = 0;
    for r in st.db.query("SELECT lantern_id FROM grabs WHERE status IN ('queued','downloading') AND lantern_id <> ''", p![])? {
        if let Ok(g) = l.grab_state(&r.s("lantern_id")).await {
            if apply(&st.db, &g)? {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Searches every wanted entry not searched in the last `search_every_hours` hours, oldest first (20 at a
/// time).
pub async fn search_due(st: &AppState) -> Result<usize> {
    let hours = st.settings.int("search_every_hours");
    if hours <= 0 || lantern(st).is_err() {
        return Ok(0);
    }
    let due = now() - hours as f64 * 3600.0;
    let rows = st.db.query("SELECT id FROM wanted WHERE status='wanted' AND (searched_at IS NULL OR searched_at < ?) ORDER BY COALESCE(searched_at, 0), id LIMIT 20", p![due])?;
    let mut n = 0;
    for r in rows {
        if search_and_grab(st, r.i("id")).await.is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

/// Follows the grabs every minute and searches the wanted list when due. For the server binary.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if let Err(e) = poll(&st).await {
                tracing::warn!("downloads can't be followed: {}", e.detail);
            }
            if let Err(e) = crate::verify::pending(&st).await {
                tracing::warn!("downloads can't be checked: {}", e.detail);
            }
            if let Err(e) = crate::import::pending(&st).await {
                tracing::warn!("downloads can't be filed: {}", e.detail);
            }
            if let Err(e) = search_due(&st).await {
                tracing::warn!("the wanted list can't be searched: {}", e.detail);
            }
        }
    });
}

/// `GET /api/wanted/{id}/releases`.
async fn releases(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    let (_, m, pr) = wanted_parts(&st.db, id)?;
    let found = find(&st.db, id, &lantern(&st)?, &m, &pr).await?;
    Ok(Json(json!({ "results": found, "min_score": st.settings.int("min_score") })))
}

/// `POST /api/wanted/{id}/search`.
async fn search_now(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    Ok(Json(search_and_grab(&st, id).await?))
}

/// The result to grab.
#[derive(Debug, Deserialize)]
pub struct GrabThis {
    /// the result's guid
    pub guid: String,
    /// its indexer
    #[serde(default)]
    pub indexer_id: i64,
    /// grab it even though Folio refused it
    #[serde(default)]
    pub force: bool,
}

/// `POST /api/wanted/{id}/grab`.
async fn grab_one(State(st): State<AppState>, _: Admin, Path(id): Path<i64>, Json(g): Json<GrabThis>) -> Result<Json<Value>> {
    let (_, m, pr) = wanted_parts(&st.db, id)?;
    let l = lantern(&st)?;
    let found = find(&st.db, id, &l, &m, &pr).await?;
    let s = found
        .into_iter()
        .find(|s| s.release.guid == g.guid && (g.indexer_id == 0 || s.release.indexer_id == g.indexer_id))
        .ok_or_else(|| err(404, "Lantern doesn't have that result any more. Search again."))?;
    if let (Some(why), false) = (&s.refused, g.force) {
        return Err(err(400, format!("Folio won't take this result: {why}")));
    }
    Ok(Json(start_grab(&st, &l, id, m.format, &s).await?))
}

/// `GET /api/grabs`.
async fn list(State(st): State<AppState>, _: SignedIn) -> Result<Json<Value>> {
    let rows = st.db.query("SELECT * FROM grabs ORDER BY created_at DESC, id DESC LIMIT 200", p![])?;
    Ok(Json(json!({ "grabs": rows.iter().map(grab_row_json).collect::<Vec<_>>() })))
}

/// `POST /api/lantern/test`: Lantern answers a search and takes Folio's sign-up.
async fn test(State(st): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let l = lantern(&st)?;
    l.search(Format::Ebook, &score::SearchQuery { text: "test".into(), author: String::new(), title: String::new() }).await?;
    let cb = callback_url(&st)?;
    let hub = l.register(&cb).await;
    Ok(Json(json!({
        "ok": hub.is_ok(),
        "message": match &hub {
            Ok(_) if cb.is_empty() => "Lantern answers and Folio is signed up. Folio asks Lantern about downloads every minute; set Folio's own address under Settings → Downloads so Lantern can tell it at once.".to_string(),
            Ok(_) => "Lantern answers and Folio is signed up.".to_string(),
            Err(e) => format!("Lantern answers searches, but its download hub doesn't: {}", e.detail),
        },
    })))
}

/// `POST /api/lantern/callback/{token}`: a grab changed. The token is the secret part of the address.
async fn callback(State(st): State<AppState>, Path(token): Path<String>, Json(g): Json<GrabState>) -> Result<Json<Value>> {
    if !crate::auth::ct_eq(&token, &callback_token(&st.db)?) {
        return Err(err(404, "Not found"));
    }
    let known = apply(&st.db, &g)?;
    if known && g.status == "completed" {
        crate::import::spawn_pending(&st);
    }
    Ok(Json(json!({ "ok": true, "known": known })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_callback_token_is_made_once() {
        let db = SqliteDb::memory();
        migrate(&db).unwrap();
        let a = callback_token(&db).unwrap();
        assert!(a.len() >= 30);
        assert_eq!(callback_token(&db).unwrap(), a);
    }

    #[test]
    fn hours_read_plainly() {
        assert_eq!(super::super::hours(970), "16 h 10 min");
        assert_eq!(super::super::hours(60), "1 h");
    }
}
