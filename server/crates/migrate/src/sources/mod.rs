//! One reader per source, and the small tools they share for reading databases whose tables change between
//! versions: a column a version doesn't have reads as empty instead of failing.

pub mod abs;
pub mod calibre;
pub mod calibre_web;
pub mod lazylibrarian;
pub mod listenarr;
pub mod readarr;

use crate::{Error, Result, Source};
use rusqlite::types::Value;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// The tables that tell each source's database apart (all must be there).
fn tables_of(source: Source) -> &'static [&'static str] {
    match source {
        Source::Audiobookshelf => &["libraryItems", "books", "mediaProgresses"],
        Source::Calibre => &["books", "authors", "books_authors_link", "data"],
        Source::CalibreWeb => &["user", "book_read_link"],
        Source::Readarr => &["Books", "Editions", "AuthorMetadata", "BookFiles"],
        Source::LazyLibrarian => &["books", "authors", "member", "seriesauthors"],
        Source::Listenarr => &["Audiobooks", "AudiobookFiles"],
    }
}

/// Checks that `conn` is `source`'s database; the error says which database it seems to be instead.
pub fn expect(source: Source, conn: &Connection) -> Result<()> {
    let has = table_names(conn)?;
    let fits = |s: Source| tables_of(s).iter().all(|t| has.contains(&t.to_ascii_lowercase()));
    if fits(source) {
        return Ok(());
    }
    match Source::ALL.into_iter().find(|s| fits(*s)) {
        Some(other) => Err(Error::new(format!(
            "This looks like a {other} database, not {source}. Choose {other} as the source, or {source}'s {} file.",
            source.file_name()
        ))),
        None => Err(Error::new(format!("This isn't a {source} database Folio knows. Choose {source}'s {} file.", source.file_name()))),
    }
}

/// The names of the tables in `conn`, in lower case.
fn table_names(conn: &Connection) -> Result<HashSet<String>> {
    let mut st = conn.prepare("SELECT name FROM sqlite_master WHERE type='table'").map_err(read_error)?;
    let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(read_error)?;
    Ok(rows.filter_map(|r| r.ok()).map(|n| n.to_ascii_lowercase()).collect())
}

/// Whether `conn` has the table `name`.
pub fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(table_names(conn)?.contains(&name.to_ascii_lowercase()))
}

/// The columns of `table`, in lower case.
pub fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut st = conn.prepare(&format!("SELECT name FROM pragma_table_info('{}')", table.replace('\'', ""))).map_err(read_error)?;
    let rows = st.query_map([], |r| r.get::<_, String>(0)).map_err(read_error)?;
    Ok(rows.filter_map(|r| r.ok()).map(|n| n.to_ascii_lowercase()).collect())
}

/// `alias.column AS name` when `table` has `column`, else `NULL AS name`: for columns some versions lack.
pub fn pick(cols: &HashSet<String>, alias: &str, column: &str, name: &str) -> String {
    if cols.contains(&column.to_ascii_lowercase()) {
        format!("{alias}.\"{column}\" AS \"{name}\"")
    } else {
        format!("NULL AS \"{name}\"")
    }
}

/// One row, by column name.
pub struct Rec(HashMap<String, Value>);

impl Rec {
    /// The column as text (numbers written out); empty when missing or null.
    pub fn s(&self, k: &str) -> String {
        match self.0.get(k) {
            Some(Value::Text(t)) => t.trim().to_string(),
            Some(Value::Integer(i)) => i.to_string(),
            Some(Value::Real(f)) => f.to_string(),
            Some(Value::Blob(b)) => String::from_utf8_lossy(b).trim().to_string(),
            _ => String::new(),
        }
    }

    /// The column as a number; `None` when missing, null or not a number.
    pub fn f(&self, k: &str) -> Option<f64> {
        match self.0.get(k) {
            Some(Value::Integer(i)) => Some(*i as f64),
            Some(Value::Real(f)) => Some(*f),
            Some(Value::Text(t)) => t.trim().parse().ok(),
            _ => None,
        }
    }

    /// The column as a whole number; `None` when missing, null or not a number.
    pub fn i(&self, k: &str) -> Option<i64> {
        self.f(k).map(|f| f as i64)
    }

    /// The column as yes or no (`1`, `true`); `None` when missing or null.
    pub fn b(&self, k: &str) -> Option<bool> {
        match self.0.get(k) {
            Some(Value::Integer(i)) => Some(*i != 0),
            Some(Value::Real(f)) => Some(*f != 0.0),
            Some(Value::Text(t)) => Some(matches!(t.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes")),
            _ => None,
        }
    }
}

/// Runs `sql` and gives every row.
pub fn rows(conn: &Connection, sql: &str) -> Result<Vec<Rec>> {
    let mut st = conn.prepare(sql).map_err(read_error)?;
    let names: Vec<String> = st.column_names().into_iter().map(str::to_string).collect();
    let rows = st
        .query_map([], |r| {
            let mut m = HashMap::with_capacity(names.len());
            for (i, n) in names.iter().enumerate() {
                m.insert(n.clone(), r.get::<_, Value>(i)?);
            }
            Ok(Rec(m))
        })
        .map_err(read_error)?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(read_error)
}

/// A database read that failed, in plain words.
pub fn read_error(e: rusqlite::Error) -> Error {
    Error::new(format!(
        "Part of the database can't be read ({e}). It may be from a version Folio doesn't know yet; send a bug report with this message."
    ))
}
