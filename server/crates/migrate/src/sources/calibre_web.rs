//! Calibre-Web's own database (`app.db`): who read which Calibre book, and how far.
//!
//! Calibre-Web keeps no books of its own; its marks point at the Calibre library's book ids, so each progress
//! entry names the book as `calibre:<id>`, the same id reading that Calibre library gives it. Read the Calibre
//! library too, so the books are there to bring the progress onto.
//!
//! Three tables say how far someone got: `book_read_link` (unread, reading or finished), `bookmark` (the web
//! reader's place, an EPUB CFI) and the Kobo sync's `kobo_reading_state` and `kobo_bookmark` (percent read).

use super::{columns, has_table, rows};
use crate::plan::{epoch, Format, Plan, Progress};
use crate::{Result, Source};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};

/// Calibre-Web's "finished" read status.
const FINISHED: i64 = 1;

/// Calibre-Web's "reading" read status.
const READING: i64 = 2;

/// Reads every user's read status and places into `plan`.
pub fn read(conn: &Connection, plan: &mut Plan) -> Result<()> {
    let users: HashMap<String, String> = rows(conn, "SELECT id, name FROM user")?.into_iter().map(|r| (r.s("id"), r.s("name"))).collect();
    // (user, book) → progress, ordered so the plan reads the same every time
    let mut out: BTreeMap<(String, i64), Progress> = BTreeMap::new();
    let mut entry = |user: &str, book: i64| -> Option<(String, i64)> {
        let name = users.get(user).filter(|n| !n.is_empty())?.clone();
        let key = (name.clone(), book);
        out.entry(key.clone()).or_insert_with(|| Progress {
            user: name,
            book: format!("{}:{book}", Source::Calibre.key()),
            format: Format::Ebook,
            position_ms: None,
            fraction: None,
            cfi: String::new(),
            finished: false,
            updated_at: None,
        });
        Some(key)
    };
    let mut keys: Vec<((String, i64), Change)> = vec![];
    let cols = columns(conn, "book_read_link")?;
    let modified = if cols.contains("last_modified") { "last_modified" } else { "NULL" };
    for r in rows(conn, &format!("SELECT user_id, book_id, read_status, {modified} AS updated FROM book_read_link"))? {
        let status = r.i("read_status").unwrap_or(0);
        if status != FINISHED && status != READING {
            continue;
        }
        if let Some(k) = entry(&r.s("user_id"), r.i("book_id").unwrap_or(0)) {
            keys.push((k, Change { finished: status == FINISHED, fraction: None, cfi: None, updated: epoch(&r.s("updated")) }));
        }
    }
    if has_table(conn, "bookmark")? {
        for r in rows(conn, "SELECT user_id, book_id, format, bookmark_key FROM bookmark")? {
            let cfi = r.s("bookmark_key");
            if !r.s("format").eq_ignore_ascii_case("epub") || !cfi.starts_with("epubcfi(") {
                continue;
            }
            if let Some(k) = entry(&r.s("user_id"), r.i("book_id").unwrap_or(0)) {
                keys.push((k, Change { finished: false, fraction: None, cfi: Some(cfi), updated: None }));
            }
        }
    }
    if has_table(conn, "kobo_reading_state")? && has_table(conn, "kobo_bookmark")? {
        let sql = "SELECT s.user_id AS user_id, s.book_id AS book_id, b.progress_percent AS percent, b.last_modified AS updated
                   FROM kobo_reading_state s JOIN kobo_bookmark b ON b.kobo_reading_state_id = s.id";
        for r in rows(conn, sql)? {
            let Some(p) = r.f("percent").filter(|p| *p > 0.0) else { continue };
            if let Some(k) = entry(&r.s("user_id"), r.i("book_id").unwrap_or(0)) {
                keys.push((
                    k,
                    Change { finished: false, fraction: Some((p / 100.0).clamp(0.0, 1.0)), cfi: None, updated: epoch(&r.s("updated")) },
                ));
            }
        }
    }
    for (k, c) in keys {
        let p = out.get_mut(&k).expect("every key was made by entry");
        p.finished |= c.finished;
        if let Some(f) = c.fraction {
            p.fraction = Some(p.fraction.map_or(f, |old: f64| old.max(f)));
        }
        if let Some(cfi) = c.cfi {
            p.cfi = cfi;
        }
        if let Some(u) = c.updated {
            p.updated_at = Some(p.updated_at.map_or(u, |old: f64| old.max(u)));
        }
    }
    for (_, mut p) in out {
        if p.finished {
            p.fraction = Some(1.0);
        }
        plan.progress.push(p);
    }
    Ok(())
}

/// One thing a table said about a person's place in a book.
struct Change {
    /// they finished it
    finished: bool,
    /// how much they read, 0 to 1
    fraction: Option<f64>,
    /// their place as an EPUB CFI
    cfi: Option<String>,
    /// when it changed
    updated: Option<f64>,
}
