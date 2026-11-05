//! Each person's place in each edition (progress) and the places they marked (bookmarks), in the database.
//!
//! An audiobook's place is a time from the start of the whole book (`position_ms`, across its files); an ebook's
//! place is a [locator](Progress::locator) `{href, progression, total_progression, cfi?}`. `fraction` says how
//! much of the whole book is behind the place (0 to 1) for both. Times are Unix seconds; the newest change wins,
//! by the time the device says it read or listened (docs/TRACKS.md).

use kit_server_core::{err, p, Db, Migrations, Result, Row, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The tables, step by step (a released step is never changed).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-playback",
    steps: &["CREATE TABLE progress (
            user_id INTEGER NOT NULL,
            edition_id INTEGER NOT NULL,
            position_ms INTEGER NOT NULL DEFAULT 0,
            locator TEXT NOT NULL DEFAULT '',
            fraction REAL NOT NULL DEFAULT 0,
            finished INTEGER NOT NULL DEFAULT 0,
            device TEXT NOT NULL DEFAULT '',
            updated_at REAL NOT NULL,
            PRIMARY KEY (user_id, edition_id)
        );
        CREATE INDEX progress_recent ON progress (user_id, updated_at);
        CREATE TABLE bookmarks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            edition_id INTEGER NOT NULL,
            position_ms INTEGER NOT NULL DEFAULT 0,
            locator TEXT NOT NULL DEFAULT '',
            note TEXT NOT NULL DEFAULT '',
            created_at REAL NOT NULL
        );
        CREATE INDEX bookmarks_edition ON bookmarks (user_id, edition_id);"],
};

/// Makes the tables (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The progress and bookmark tables can't be made: {}", e.detail))
}

/// A person's place in one edition: what a device sends, and what the server keeps and answers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// the edition (taken from the address when a device sends it)
    #[serde(default)]
    pub edition_id: i64,
    /// an audiobook's place: milliseconds from the start of the whole book
    #[serde(default)]
    pub position_ms: i64,
    /// an ebook's place: `{href, progression, total_progression, cfi?}`, or `null`
    #[serde(default)]
    pub locator: Value,
    /// how much of the whole book is behind the place, 0 to 1
    #[serde(default)]
    pub fraction: f64,
    /// whether the person finished the book
    #[serde(default)]
    pub finished: bool,
    /// which app or device moved there: `web`, `android:<id>`, `koreader:<name>` …
    #[serde(default)]
    pub device: String,
    /// when the device says it moved there, Unix seconds; now when 0 or missing
    #[serde(default)]
    pub updated_at: f64,
}

/// A place a person marked in an edition, with their note.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    /// the bookmark's id (0 for a new one)
    #[serde(default)]
    pub id: i64,
    /// the edition (taken from the address when a device sends it)
    #[serde(default)]
    pub edition_id: i64,
    /// an audiobook's place: milliseconds from the start of the whole book
    #[serde(default)]
    pub position_ms: i64,
    /// an ebook's place, like [`Progress::locator`]
    #[serde(default)]
    pub locator: Value,
    /// the person's note ("" for none)
    #[serde(default)]
    pub note: String,
    /// when it was made, Unix seconds; now when 0 or missing
    #[serde(default)]
    pub created_at: f64,
}

/// Checks a place: no time before the start, a locator that is an object (or nothing).
fn check_place(position_ms: i64, locator: &Value) -> Result<()> {
    if position_ms < 0 {
        return Err(err(400, "The place in the book can't be before its start."));
    }
    if !(locator.is_object() || locator.is_null()) {
        return Err(err(400, "The place in the ebook isn't in the form Folio reads."));
    }
    Ok(())
}

/// A locator as stored text ("" for none).
fn locator_text(v: &Value) -> String {
    if v.is_object() {
        v.to_string()
    } else {
        String::new()
    }
}

/// A row's locator column back as JSON (`null` when none).
fn locator(r: &Row) -> Value {
    serde_json::from_str(&r.s("locator")).unwrap_or(Value::Null)
}

/// A progress row as a [`Progress`].
fn progress_of(r: &Row) -> Progress {
    Progress {
        edition_id: r.i("edition_id"),
        position_ms: r.i("position_ms"),
        locator: locator(r),
        fraction: r.f("fraction"),
        finished: r.b("finished"),
        device: r.s("device"),
        updated_at: r.f("updated_at"),
    }
}

/// A bookmark row as a [`Bookmark`].
fn bookmark_of(r: &Row) -> Bookmark {
    Bookmark {
        id: r.i("id"),
        edition_id: r.i("edition_id"),
        position_ms: r.i("position_ms"),
        locator: locator(r),
        note: r.s("note"),
        created_at: r.f("created_at"),
    }
}

/// 404 when the edition `id` is not in the library.
pub fn edition_exists(db: &SqliteDb, id: i64) -> Result<()> {
    match db.one("SELECT id FROM editions WHERE id=?", p![id])? {
        Some(_) => Ok(()),
        None => Err(err(404, "That book isn't in the library any more.")),
    }
}

/// The person's place in the edition, `None` when they never opened it.
pub fn progress(db: &SqliteDb, user_id: i64, edition_id: i64) -> Result<Option<Progress>> {
    Ok(db.one("SELECT * FROM progress WHERE user_id=? AND edition_id=?", p![user_id, edition_id])?.map(|r| progress_of(&r)))
}

/// Every place the person keeps, the most recent first; with `since` (Unix seconds) only those changed after it.
pub fn all_progress(db: &SqliteDb, user_id: i64, since: Option<f64>) -> Result<Vec<Progress>> {
    let rows = db.query(
        "SELECT * FROM progress WHERE user_id=? AND updated_at > ? ORDER BY updated_at DESC",
        p![user_id, since.unwrap_or(f64::MIN)],
    )?;
    Ok(rows.iter().map(progress_of).collect())
}

/// Keeps the person's place in the edition unless a newer one is kept already (another device moved on since).
/// Answers the place that stands: this one, or the newer one.
pub fn save_progress(db: &SqliteDb, user_id: i64, edition_id: i64, mut p: Progress) -> Result<Progress> {
    edition_exists(db, edition_id)?;
    check_place(p.position_ms, &p.locator)?;
    if !p.fraction.is_finite() {
        return Err(err(400, "How far into the book is not a number."));
    }
    p.fraction = p.fraction.clamp(0.0, 1.0);
    p.device = p.device.chars().take(120).collect();
    if p.updated_at <= 0.0 || !p.updated_at.is_finite() {
        p.updated_at = kit_server_core::db::now();
    }
    if let Some(kept) = progress(db, user_id, edition_id)? {
        if kept.updated_at > p.updated_at {
            return Ok(kept);
        }
    }
    db.execute(
        "INSERT INTO progress (user_id, edition_id, position_ms, locator, fraction, finished, device, updated_at)
         VALUES (?,?,?,?,?,?,?,?)
         ON CONFLICT (user_id, edition_id) DO UPDATE SET position_ms=excluded.position_ms, locator=excluded.locator,
         fraction=excluded.fraction, finished=excluded.finished, device=excluded.device, updated_at=excluded.updated_at",
        p![user_id, edition_id, p.position_ms, locator_text(&p.locator), p.fraction, p.finished, p.device, p.updated_at],
    )?;
    progress(db, user_id, edition_id)?.ok_or_else(|| err(500, "The place was not kept. Try again."))
}

/// Forgets the person's place in the edition (start again).
pub fn clear_progress(db: &SqliteDb, user_id: i64, edition_id: i64) -> Result<()> {
    db.execute("DELETE FROM progress WHERE user_id=? AND edition_id=?", p![user_id, edition_id])?;
    Ok(())
}

/// The person's bookmarks in the edition, in book order.
pub fn bookmarks(db: &SqliteDb, user_id: i64, edition_id: i64) -> Result<Vec<Bookmark>> {
    let mut list: Vec<Bookmark> = db
        .query("SELECT * FROM bookmarks WHERE user_id=? AND edition_id=? ORDER BY position_ms, id", p![user_id, edition_id])?
        .iter()
        .map(bookmark_of)
        .collect();
    // ebooks have no time: their order is how far into the book
    let far = |b: &Bookmark| b.locator.get("total_progression").and_then(Value::as_f64).unwrap_or(0.0);
    list.sort_by(|a, b| a.position_ms.cmp(&b.position_ms).then(far(a).total_cmp(&far(b))).then(a.id.cmp(&b.id)));
    Ok(list)
}

/// Keeps a new bookmark in the edition; the bookmark.
pub fn add_bookmark(db: &SqliteDb, user_id: i64, edition_id: i64, b: Bookmark) -> Result<Bookmark> {
    edition_exists(db, edition_id)?;
    check_place(b.position_ms, &b.locator)?;
    let note: String = b.note.chars().take(5000).collect();
    let at = if b.created_at > 0.0 && b.created_at.is_finite() { b.created_at } else { kit_server_core::db::now() };
    let id = db.execute(
        "INSERT INTO bookmarks (user_id, edition_id, position_ms, locator, note, created_at) VALUES (?,?,?,?,?,?)",
        p![user_id, edition_id, b.position_ms, locator_text(&b.locator), note, at],
    )?;
    one_bookmark(db, user_id, id)
}

/// The person's bookmark `id`; 404 when it isn't theirs or is gone.
fn one_bookmark(db: &SqliteDb, user_id: i64, id: i64) -> Result<Bookmark> {
    db.one("SELECT * FROM bookmarks WHERE id=? AND user_id=?", p![id, user_id])?
        .map(|r| bookmark_of(&r))
        .ok_or_else(|| err(404, "That bookmark isn't there any more."))
}

/// Changes the note of a bookmark of the person's; the bookmark.
pub fn edit_bookmark(db: &SqliteDb, user_id: i64, id: i64, note: &str) -> Result<Bookmark> {
    one_bookmark(db, user_id, id)?;
    let note: String = note.chars().take(5000).collect();
    db.execute("UPDATE bookmarks SET note=? WHERE id=? AND user_id=?", p![note, id, user_id])?;
    one_bookmark(db, user_id, id)
}

/// Deletes a bookmark of the person's. One that is gone already is fine (another device deleted it).
pub fn delete_bookmark(db: &SqliteDb, user_id: i64, id: i64) -> Result<()> {
    db.execute("DELETE FROM bookmarks WHERE id=? AND user_id=?", p![id, user_id])?;
    Ok(())
}
