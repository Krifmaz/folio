//! The migration's own tables: each run with its report, which source book became which edition, and the files of
//! migrated books waiting for the import to file them in the library.
//!
//! - **`migration_runs`**: one row per run (dry or real): its source and path, how far it got, and the report.
//! - **`migration_ids`**: a source's book (`abs:<item>`, `calibre:42`) → the edition it became. A later run uses it,
//!   so Calibre-Web's progress lands on the books a Calibre run brought in.
//! - **`migration_files`**: a migrated book's files where the old app keeps them, waiting for the import to verify
//!   and file them (`waiting`); the source files are never moved or changed.

use kit_server_core::db::now;
use kit_server_core::{err, p, Db, Migrations, Result, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// The migration's tables, step by step (a released step is never changed).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-migration",
    steps: &["CREATE TABLE migration_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            path TEXT NOT NULL,
            dry_run INTEGER NOT NULL,
            status TEXT NOT NULL,
            done INTEGER NOT NULL DEFAULT 0,
            total INTEGER NOT NULL DEFAULT 0,
            report TEXT NOT NULL DEFAULT '',
            error TEXT NOT NULL DEFAULT '',
            user_id INTEGER NOT NULL,
            created_at REAL NOT NULL,
            finished_at REAL
        );
        CREATE TABLE migration_ids (
            source_id TEXT PRIMARY KEY,
            edition_id INTEGER NOT NULL,
            run_id INTEGER NOT NULL
        );
        CREATE TABLE migration_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id TEXT NOT NULL UNIQUE,
            edition_id INTEGER NOT NULL,
            format TEXT NOT NULL,
            folder TEXT NOT NULL DEFAULT '',
            files TEXT NOT NULL,
            cover TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'waiting',
            run_id INTEGER NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX migration_files_status ON migration_files (status, id);"],
};

/// Makes the migration's tables (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The migration tables can't be made: {}", e.detail))
}

/// Starts a run of `source` at `path` by the account `user_id`; its id. 409 while another run is going.
pub fn start(db: &SqliteDb, source: &str, path: &str, dry_run: bool, user_id: i64) -> Result<i64> {
    if db.one("SELECT id FROM migration_runs WHERE status='running'", p![])?.is_some() {
        return Err(err(409, "A migration is running already. Wait for it to finish, then start this one."));
    }
    db.execute(
        "INSERT INTO migration_runs (source, path, dry_run, status, user_id, created_at) VALUES (?,?,?,?,?,?)",
        p![source, path, dry_run, "running", user_id, now()],
    )
}

/// Notes how far the run got: `done` of `total` books.
pub fn step(db: &SqliteDb, id: i64, done: usize, total: usize) -> Result<()> {
    db.execute("UPDATE migration_runs SET done=?, total=? WHERE id=?", p![done as i64, total as i64, id])?;
    Ok(())
}

/// Ends the run with its report.
pub fn finish(db: &SqliteDb, id: i64, report: &Value) -> Result<()> {
    db.execute(
        "UPDATE migration_runs SET status='done', done=total, report=?, finished_at=? WHERE id=?",
        p![report.to_string(), now(), id],
    )?;
    Ok(())
}

/// Ends the run that stopped on `error` (a plain sentence).
pub fn fail(db: &SqliteDb, id: i64, error: &str) -> Result<()> {
    db.execute("UPDATE migration_runs SET status='failed', error=?, finished_at=? WHERE id=?", p![error, now(), id])?;
    Ok(())
}

/// Marks runs left `running` by a server that stopped as failed (at start).
pub fn end_stale(db: &SqliteDb) -> Result<()> {
    db.execute(
        "UPDATE migration_runs SET status='failed', error=?, finished_at=? WHERE status='running'",
        p!["Folio stopped while this migration ran. Start it again: what was brought in already is kept and not added twice.", now()],
    )?;
    Ok(())
}

/// A run row as clients get it (`with_report`: with the whole report).
fn run_json(r: &Row, with_report: bool) -> Value {
    let report: Value = serde_json::from_str(&r.s("report")).unwrap_or(Value::Null);
    let mut v = json!({
        "id": r.i("id"),
        "source": r.s("source"),
        "path": r.s("path"),
        "dry_run": r.b("dry_run"),
        "status": r.s("status"),
        "done": r.i("done"),
        "total": r.i("total"),
        "error": r.s("error"),
        "created_at": r.f("created_at"),
        "finished_at": r.get("finished_at").cloned().unwrap_or(Value::Null),
        "counts": report.get("counts").cloned().unwrap_or(Value::Null),
    });
    if with_report {
        v["report"] = report;
    }
    v
}

/// The run `id` with its report; 404 when there is none.
pub fn run(db: &SqliteDb, id: i64) -> Result<Value> {
    db.one("SELECT * FROM migration_runs WHERE id=?", p![id])?
        .map(|r| run_json(&r, true))
        .ok_or_else(|| err(404, "That migration run isn't there any more."))
}

/// Every run, newest first, without their reports.
pub fn runs(db: &SqliteDb) -> Result<Vec<Value>> {
    Ok(db.query("SELECT * FROM migration_runs ORDER BY id DESC LIMIT 200", p![])?.iter().map(|r| run_json(r, false)).collect())
}

/// The edition a source's book became in an earlier run.
pub fn edition_of(db: &SqliteDb, source_id: &str) -> Result<Option<i64>> {
    Ok(db
        .one("SELECT m.edition_id FROM migration_ids m JOIN editions e ON e.id = m.edition_id WHERE m.source_id=?", p![source_id])?
        .map(|r| r.i("edition_id")))
}

/// Remembers that the source's book became `edition_id`.
pub fn remember(db: &SqliteDb, source_id: &str, edition_id: i64, run_id: i64) -> Result<()> {
    db.execute(
        "INSERT INTO migration_ids (source_id, edition_id, run_id) VALUES (?,?,?)
         ON CONFLICT (source_id) DO UPDATE SET edition_id=excluded.edition_id, run_id=excluded.run_id",
        p![source_id, edition_id, run_id],
    )?;
    Ok(())
}

/// Keeps a migrated book's files for the import (`waiting`); the same book again replaces its waiting files.
/// Answers `false` when the edition already has files in the library (nothing to import).
pub fn queue_files(db: &SqliteDb, book: &migrate::Book, edition_id: i64, run_id: i64) -> Result<bool> {
    if db.one("SELECT id FROM edition_files WHERE edition_id=? LIMIT 1", p![edition_id])?.is_some() {
        return Ok(false);
    }
    let files: Vec<String> = book.files.iter().map(|f| f.to_string_lossy().into_owned()).collect();
    let text = |p: &Option<std::path::PathBuf>| p.as_ref().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let format = match book.format {
        migrate::Format::Audiobook => "audiobook",
        migrate::Format::Ebook => "ebook",
    };
    db.execute(
        "INSERT INTO migration_files (source_id, edition_id, format, folder, files, cover, status, run_id, created_at)
         VALUES (?,?,?,?,?,?,'waiting',?,?)
         ON CONFLICT (source_id) DO UPDATE SET edition_id=excluded.edition_id, format=excluded.format, folder=excluded.folder,
         files=excluded.files, cover=excluded.cover, run_id=excluded.run_id
         WHERE migration_files.status='waiting'",
        p![book.source_id, edition_id, format, text(&book.folder), json!(files).to_string(), text(&book.cover), run_id, now()],
    )?;
    Ok(true)
}

/// The migrated books whose files wait for the import, oldest first:
/// `[{id, source_id, edition_id, format, folder, files: [path], cover}]`.
pub fn waiting(db: &SqliteDb) -> Result<Vec<Value>> {
    Ok(db
        .query("SELECT * FROM migration_files WHERE status='waiting' ORDER BY id", p![])?
        .iter()
        .map(|r| {
            json!({
                "id": r.i("id"),
                "source_id": r.s("source_id"),
                "edition_id": r.i("edition_id"),
                "format": r.s("format"),
                "folder": r.s("folder"),
                "files": serde_json::from_str::<Value>(&r.s("files")).unwrap_or(json!([])),
                "cover": r.s("cover"),
            })
        })
        .collect())
}

/// Marks a waiting book's files as handled by the import: `status` is `imported` or `review` (or another word the
/// import uses).
pub fn set_file_status(db: &SqliteDb, id: i64, status: &str) -> Result<()> {
    db.execute("UPDATE migration_files SET status=? WHERE id=?", p![status, id])?;
    Ok(())
}
