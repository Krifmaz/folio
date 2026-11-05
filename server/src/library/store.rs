//! The catalogue in the database: authors, series, works and their exact editions.
//!
//! A **work** is the book itself (*Project Hail Mary* by Andy Weir); an **edition** is one exact version of it
//! you can get: the Audible audiobook `B08G9PRS1K` narrated by Ray Porter, or the ebook with ISBN
//! `9780593135204`. Editions of one work are tied together by Open Library's work key when the source has one,
//! otherwise by the title (without subtitle) and the first author ([`text::title_key`], [`text::person_key`]).
//!
//! Each edition keeps everything the sources said ([`EditionMeta`]) as JSON in `meta`, with the columns Folio
//! searches on beside it.

use super::text;
use crate::metadata::{EditionMeta, Format};
use kit_server_core::db::now;
use kit_server_core::{err, p, Db, Migrations, Result, Row, RowExt, SqliteDb};
use serde::Serialize;
use serde_json::{json, Value};

/// The catalogue's tables, step by step (a released step is never changed).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-library",
    steps: &[
        "CREATE TABLE authors (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            sort_name TEXT NOT NULL,
            name_key TEXT NOT NULL UNIQUE,
            asin TEXT NOT NULL DEFAULT '',
            ol_key TEXT NOT NULL DEFAULT '',
            created_at REAL NOT NULL
        );
        CREATE TABLE series (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            name_key TEXT NOT NULL UNIQUE,
            asin TEXT NOT NULL DEFAULT '',
            created_at REAL NOT NULL
        );
        CREATE TABLE works (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            title_key TEXT NOT NULL,
            author_key TEXT NOT NULL,
            ol_work TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            cover_url TEXT NOT NULL DEFAULT '',
            year INTEGER,
            created_at REAL NOT NULL,
            updated_at REAL NOT NULL,
            UNIQUE (title_key, author_key)
        );
        CREATE INDEX works_ol_work ON works (ol_work);
        CREATE TABLE work_authors (
            work_id INTEGER NOT NULL,
            author_id INTEGER NOT NULL,
            position INTEGER NOT NULL,
            PRIMARY KEY (work_id, author_id)
        );
        CREATE INDEX work_authors_author ON work_authors (author_id);
        CREATE TABLE work_series (
            work_id INTEGER NOT NULL,
            series_id INTEGER NOT NULL,
            position TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (work_id, series_id)
        );
        CREATE INDEX work_series_series ON work_series (series_id);
        CREATE TABLE editions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            work_id INTEGER NOT NULL,
            format TEXT NOT NULL,
            asin TEXT NOT NULL DEFAULT '',
            isbn13 TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT '',
            meta TEXT NOT NULL,
            created_at REAL NOT NULL,
            refreshed_at REAL NOT NULL
        );
        CREATE INDEX editions_work ON editions (work_id);
        CREATE UNIQUE INDEX editions_asin ON editions (asin) WHERE asin <> '';
        CREATE UNIQUE INDEX editions_isbn ON editions (format, isbn13) WHERE isbn13 <> '';",
        // 2: followed authors and series ("" not followed, audiobook, ebook or both)
        "ALTER TABLE authors ADD COLUMN follow TEXT NOT NULL DEFAULT '';
        ALTER TABLE series ADD COLUMN follow TEXT NOT NULL DEFAULT '';",
        // 3: an edition's files in the library, written by the import (docs/TRACKS.md): path under the library
        // folder, kind audio, epub, pdf, azw3 or cover, the play order, and for audio the length and chapters
        // ([{title, start_ms, end_ms}] from the start of this file)
        "CREATE TABLE edition_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            edition_id INTEGER NOT NULL,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            mime TEXT NOT NULL,
            size INTEGER NOT NULL DEFAULT 0,
            position INTEGER NOT NULL DEFAULT 0,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            chapters TEXT NOT NULL DEFAULT '[]',
            created_at REAL NOT NULL
        );
        CREATE INDEX edition_files_edition ON edition_files (edition_id, position);",
    ],
};

/// Makes the catalogue's tables (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The library tables can't be made: {}", e.detail))
}

/// What saving an edition did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Saved {
    /// the work the edition belongs to
    pub work_id: i64,
    /// the edition
    pub edition_id: i64,
    /// whether the edition is new (false: it was there and its data was refreshed)
    pub created: bool,
}

/// The edition already kept with `m`'s ASIN (audiobooks) or ISBN (ebooks): its id and work.
pub fn find_edition(db: &SqliteDb, m: &EditionMeta) -> Result<Option<(i64, i64)>> {
    let row = match m.format {
        Format::Audiobook if !m.asin.is_empty() => db.one("SELECT id, work_id FROM editions WHERE asin=?", p![m.asin])?,
        Format::Ebook if !m.isbn13.is_empty() => {
            db.one("SELECT id, work_id FROM editions WHERE format='ebook' AND isbn13=?", p![m.isbn13])?
        }
        _ => None,
    };
    Ok(row.map(|r| (r.i("id"), r.i("work_id"))))
}

/// Keeps the edition `m`: refreshes it when its ASIN or ISBN is already kept, otherwise adds it to its work
/// (found by Open Library's work key, or by title and first author; made when there is none), with its
/// authors and series.
pub fn save(db: &SqliteDb, m: &EditionMeta) -> Result<Saved> {
    if m.title.trim().is_empty() {
        return Err(err(400, "This edition has no title, so it can't be added."));
    }
    if m.format == Format::Audiobook && m.asin.is_empty() || m.format == Format::Ebook && m.isbn13.is_empty() {
        return Err(err(400, "This edition has no ASIN or ISBN, so it can't be told apart from others."));
    }
    let t = now();
    let meta = serde_json::to_string(m).map_err(|e| err(500, format!("The edition can't be saved: {e}")))?;
    if let Some((id, work_id)) = find_edition(db, m)? {
        db.execute("UPDATE editions SET title=?, language=?, meta=?, refreshed_at=? WHERE id=?", p![m.title, m.language, meta, t, id])?;
        link_people(db, work_id, m)?;
        fill_work(db, work_id, m)?;
        return Ok(Saved { work_id, edition_id: id, created: false });
    }
    let work_id = match find_work(db, m)? {
        Some(w) => w,
        None => db.execute(
            "INSERT INTO works (title, title_key, author_key, ol_work, description, cover_url, year, created_at, updated_at) VALUES (?,?,?,?,?,?,?,?,?)",
            p![m.title.trim(), text::title_key(&m.title), text::person_key(m.author()), m.ol_work, m.description, m.cover_url, m.year(), t, t],
        )?,
    };
    let edition_id = db.execute(
        "INSERT INTO editions (work_id, format, asin, isbn13, title, language, meta, created_at, refreshed_at) VALUES (?,?,?,?,?,?,?,?,?)",
        p![
            work_id,
            m.format.as_str(),
            if m.format == Format::Audiobook { m.asin.as_str() } else { "" },
            m.isbn13,
            m.title,
            m.language,
            meta,
            t,
            t
        ],
    )?;
    link_people(db, work_id, m)?;
    fill_work(db, work_id, m)?;
    Ok(Saved { work_id, edition_id, created: true })
}

/// The work `m` is an edition of: by Open Library work key first, then by title and first author.
fn find_work(db: &SqliteDb, m: &EditionMeta) -> Result<Option<i64>> {
    if !m.ol_work.is_empty() {
        if let Some(r) = db.one("SELECT id FROM works WHERE ol_work=?", p![m.ol_work])? {
            return Ok(Some(r.i("id")));
        }
    }
    Ok(db
        .one("SELECT id FROM works WHERE title_key=? AND author_key=?", p![text::title_key(&m.title), text::person_key(m.author())])?
        .map(|r| r.i("id")))
}

/// Fills the work's empty fields from `m` (blurb, cover, year, Open Library key).
fn fill_work(db: &SqliteDb, work_id: i64, m: &EditionMeta) -> Result<()> {
    db.execute(
        "UPDATE works SET
            description = CASE WHEN length(description) < length(?1) THEN ?1 ELSE description END,
            cover_url = CASE WHEN cover_url = '' THEN ?2 ELSE cover_url END,
            year = CASE WHEN year IS NULL OR (?3 IS NOT NULL AND ?3 < year) THEN ?3 ELSE year END,
            ol_work = CASE WHEN ol_work = '' THEN ?4 ELSE ol_work END,
            updated_at = ?5
         WHERE id = ?6",
        p![m.description, m.cover_url, m.year(), m.ol_work, now(), work_id],
    )?;
    Ok(())
}

/// Adds `m`'s authors and series to the work (each person and series kept once, by its key).
fn link_people(db: &SqliteDb, work_id: i64, m: &EditionMeta) -> Result<()> {
    for (i, a) in m.authors.iter().enumerate() {
        let key = text::person_key(&a.name);
        if key.is_empty() {
            continue;
        }
        let id = match db.one("SELECT id FROM authors WHERE name_key=?", p![key])? {
            Some(r) => {
                db.execute(
                    "UPDATE authors SET asin = CASE WHEN asin='' THEN ? ELSE asin END, ol_key = CASE WHEN ol_key='' THEN ? ELSE ol_key END WHERE id=?",
                    p![a.asin, a.ol_key, r.i("id")],
                )?;
                r.i("id")
            }
            None => db.execute(
                "INSERT INTO authors (name, sort_name, name_key, asin, ol_key, created_at) VALUES (?,?,?,?,?,?)",
                p![a.name.trim(), text::sort_name(&a.name), key, a.asin, a.ol_key, now()],
            )?,
        };
        db.execute("INSERT OR IGNORE INTO work_authors (work_id, author_id, position) VALUES (?,?,?)", p![work_id, id, i as i64])?;
    }
    for s in &m.series {
        let key = text::fold(&s.name);
        if key.is_empty() {
            continue;
        }
        let id = match db.one("SELECT id FROM series WHERE name_key=?", p![key])? {
            Some(r) => {
                db.execute("UPDATE series SET asin = CASE WHEN asin='' THEN ? ELSE asin END WHERE id=?", p![s.asin, r.i("id")])?;
                r.i("id")
            }
            None => {
                db.execute("INSERT INTO series (name, name_key, asin, created_at) VALUES (?,?,?,?)", p![s.name.trim(), key, s.asin, now()])?
            }
        };
        db.execute(
            "INSERT INTO work_series (work_id, series_id, position) VALUES (?,?,?)
             ON CONFLICT (work_id, series_id) DO UPDATE SET position = CASE WHEN position='' THEN excluded.position ELSE position END",
            p![work_id, id, s.position],
        )?;
    }
    Ok(())
}

/// One edition as clients get it: everything the sources said, its id and its work.
pub fn edition_json(r: &Row) -> Value {
    let mut v: Value = serde_json::from_str(&r.s("meta")).unwrap_or_else(|_| json!({}));
    if let Some(o) = v.as_object_mut() {
        o.insert("id".into(), json!(r.i("id")));
        o.insert("work_id".into(), json!(r.i("work_id")));
        o.insert("added_at".into(), json!(r.f("created_at")));
        o.insert("refreshed_at".into(), json!(r.f("refreshed_at")));
    }
    v
}

/// The edition `id` as it was saved (404 when there is none).
pub fn edition(db: &SqliteDb, id: i64) -> Result<(Row, EditionMeta)> {
    let r = db.one("SELECT * FROM editions WHERE id=?", p![id])?.ok_or_else(|| err(404, "That edition isn't in the library any more."))?;
    let m: EditionMeta =
        serde_json::from_str(&r.s("meta")).map_err(|e| err(500, format!("The edition's saved data can't be read: {e}")))?;
    Ok((r, m))
}

/// The authors of a work, in order: `[{id, name}]`.
fn work_authors(db: &SqliteDb, work_id: i64) -> Result<Vec<Value>> {
    Ok(db
        .query(
            "SELECT a.id, a.name FROM work_authors wa JOIN authors a ON a.id=wa.author_id WHERE wa.work_id=? ORDER BY wa.position",
            p![work_id],
        )?
        .iter()
        .map(|r| json!({ "id": r.i("id"), "name": r.s("name") }))
        .collect())
}

/// The series of a work: `[{id, name, position}]`.
fn work_series(db: &SqliteDb, work_id: i64) -> Result<Vec<Value>> {
    Ok(db
        .query(
            "SELECT s.id, s.name, ws.position FROM work_series ws JOIN series s ON s.id=ws.series_id WHERE ws.work_id=? ORDER BY s.name",
            p![work_id],
        )?
        .iter()
        .map(|r| json!({ "id": r.i("id"), "name": r.s("name"), "position": r.s("position") }))
        .collect())
}

/// A work's summary for lists: title, authors, series, cover, year and the formats it has editions in.
fn work_summary(db: &SqliteDb, r: &Row) -> Result<Value> {
    let id = r.i("id");
    let formats: Vec<String> =
        db.query("SELECT DISTINCT format FROM editions WHERE work_id=? ORDER BY format", p![id])?.iter().map(|f| f.s("format")).collect();
    let editions = db.one("SELECT COUNT(*) AS n FROM editions WHERE work_id=?", p![id])?.map(|c| c.i("n")).unwrap_or(0);
    Ok(json!({
        "id": id,
        "title": r.s("title"),
        "authors": work_authors(db, id)?,
        "series": work_series(db, id)?,
        "cover_url": r.s("cover_url"),
        "year": r.get("year").cloned().unwrap_or(Value::Null),
        "formats": formats,
        "editions": editions,
    }))
}

/// The works, newest first, whose title or author name has `q` in it (all when empty), with an edition in
/// `format` when given.
pub fn works(db: &SqliteDb, q: &str, format: Option<Format>) -> Result<Vec<Value>> {
    let like = format!("%{}%", text::fold(q).replace(' ', "%"));
    let rows = db.query(
        "SELECT w.* FROM works w
         WHERE (?1 = '%%' OR w.title_key LIKE ?1 OR EXISTS (SELECT 1 FROM work_authors wa JOIN authors a ON a.id=wa.author_id WHERE wa.work_id=w.id AND a.name_key LIKE replace(?1, ' ', '')))
           AND (?2 = '' OR EXISTS (SELECT 1 FROM editions e WHERE e.work_id=w.id AND e.format=?2))
         ORDER BY w.created_at DESC, w.id DESC",
        p![like, format.map(Format::as_str).unwrap_or("")],
    )?;
    rows.iter().map(|r| work_summary(db, r)).collect()
}

/// The work `id` with its authors, series and every edition (404 when there is none).
pub fn work(db: &SqliteDb, id: i64) -> Result<Value> {
    let r = db.one("SELECT * FROM works WHERE id=?", p![id])?.ok_or_else(|| err(404, "That book isn't in the library any more."))?;
    let mut v = work_summary(db, &r)?;
    let editions: Vec<Value> =
        db.query("SELECT * FROM editions WHERE work_id=? ORDER BY format, created_at", p![id])?.iter().map(edition_json).collect();
    v["description"] = json!(r.s("description"));
    v["ol_work"] = json!(r.s("ol_work"));
    v["editions"] = json!(editions);
    Ok(v)
}

/// Removes the edition `id`; its work goes too when it has no other edition. 404 when there is none.
pub fn delete_edition(db: &SqliteDb, id: i64) -> Result<()> {
    let (r, _) = edition(db, id)?;
    let work_id = r.i("work_id");
    db.execute("DELETE FROM editions WHERE id=?", p![id])?;
    let left = db.one("SELECT COUNT(*) AS n FROM editions WHERE work_id=?", p![work_id])?.map(|c| c.i("n")).unwrap_or(0);
    if left == 0 {
        db.execute("DELETE FROM work_authors WHERE work_id=?", p![work_id])?;
        db.execute("DELETE FROM work_series WHERE work_id=?", p![work_id])?;
        db.execute("DELETE FROM works WHERE id=?", p![work_id])?;
    }
    Ok(())
}

/// Every author with how many works of theirs are kept, A to Z by last name.
pub fn authors(db: &SqliteDb) -> Result<Vec<Value>> {
    Ok(db
        .query("SELECT a.*, (SELECT COUNT(*) FROM work_authors wa WHERE wa.author_id=a.id) AS works FROM authors a ORDER BY a.sort_name COLLATE NOCASE", p![])?
        .iter()
        .map(|r| json!({ "id": r.i("id"), "name": r.s("name"), "sort_name": r.s("sort_name"), "asin": r.s("asin"), "ol_key": r.s("ol_key"), "works": r.i("works"), "follow": r.s("follow") }))
        .collect())
}

/// Every series with its works in reading order (by position as a number, then as written).
pub fn series(db: &SqliteDb) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for s in db.query("SELECT * FROM series ORDER BY name COLLATE NOCASE", p![])? {
        let mut books: Vec<(f64, String, Value)> = db
            .query(
                "SELECT w.id, w.title, ws.position FROM work_series ws JOIN works w ON w.id=ws.work_id WHERE ws.series_id=?",
                p![s.i("id")],
            )?
            .iter()
            .map(|r| {
                let pos = r.s("position");
                let n = pos.split(['-', ' ']).next().and_then(|x| x.parse::<f64>().ok()).unwrap_or(f64::MAX);
                (n, pos.clone(), json!({ "id": r.i("id"), "title": r.s("title"), "position": pos }))
            })
            .collect();
        books.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out.push(json!({ "id": s.i("id"), "name": s.s("name"), "asin": s.s("asin"), "follow": s.s("follow"), "works": books.into_iter().map(|b| b.2).collect::<Vec<_>>() }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Person, SeriesRef};

    fn db() -> SqliteDb {
        let db = SqliteDb::memory();
        migrate(&db).unwrap();
        db
    }

    fn audiobook() -> EditionMeta {
        EditionMeta {
            format: Format::Audiobook,
            asin: "B08G9PRS1K".into(),
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), asin: "B00G0WYW92".into(), ..Default::default() }],
            narrators: vec!["Ray Porter".into()],
            runtime_min: Some(970),
            abridged: Some(false),
            language: "en".into(),
            release_date: "2021-05-04".into(),
            ..Default::default()
        }
    }

    fn ebook() -> EditionMeta {
        EditionMeta {
            format: Format::Ebook,
            isbn13: "9780593135204".into(),
            title: "Project Hail Mary: A Novel".into(),
            authors: vec![Person { name: "Weir, Andy".into(), ol_key: "OL7234434A".into(), ..Default::default() }],
            language: "en".into(),
            ol_work: "OL21745884W".into(),
            description: "A lone astronaut.".into(),
            release_date: "2021".into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_audiobook_and_an_ebook_of_one_book_share_a_work() {
        let db = db();
        let a = save(&db, &audiobook()).unwrap();
        let e = save(&db, &ebook()).unwrap();
        assert!(a.created && e.created);
        assert_eq!(a.work_id, e.work_id, "same title and author");
        let w = work(&db, a.work_id).unwrap();
        assert_eq!(w["formats"], json!(["audiobook", "ebook"]));
        assert_eq!(w["authors"].as_array().unwrap().len(), 1, "one Andy Weir");
        assert_eq!(w["ol_work"], "OL21745884W");
        assert_eq!(w["year"], 2021);
        let au = authors(&db).unwrap();
        assert_eq!(
            (au[0]["asin"].as_str(), au[0]["ol_key"].as_str(), au[0]["sort_name"].as_str()),
            (Some("B00G0WYW92"), Some("OL7234434A"), Some("Weir, Andy"))
        );
    }

    #[test]
    fn saving_again_refreshes_the_same_edition() {
        let db = db();
        let first = save(&db, &audiobook()).unwrap();
        let mut m = audiobook();
        m.runtime_min = Some(971);
        let again = save(&db, &m).unwrap();
        assert_eq!((again.edition_id, again.created), (first.edition_id, false));
        let (_, kept) = edition(&db, first.edition_id).unwrap();
        assert_eq!(kept.runtime_min, Some(971));
        assert_eq!(works(&db, "", None).unwrap().len(), 1);
    }

    #[test]
    fn works_are_searched_by_title_author_and_format() {
        let db = db();
        save(&db, &audiobook()).unwrap();
        let mut kings = audiobook();
        kings.asin = "B003ZWFO7E".into();
        kings.title = "The Way of Kings".into();
        kings.authors = vec![Person { name: "Brandon Sanderson".into(), ..Default::default() }];
        kings.series = vec![SeriesRef { name: "The Stormlight Archive".into(), position: "1".into(), asin: String::new() }];
        save(&db, &kings).unwrap();
        assert_eq!(works(&db, "", None).unwrap().len(), 2);
        assert_eq!(works(&db, "hail", None).unwrap()[0]["title"], "Project Hail Mary");
        assert_eq!(works(&db, "sanderson", None).unwrap()[0]["title"], "The Way of Kings");
        assert_eq!(works(&db, "way kings", None).unwrap().len(), 1);
        assert!(works(&db, "", Some(Format::Ebook)).unwrap().is_empty());
        let s = series(&db).unwrap();
        assert_eq!(s[0]["works"][0]["position"], "1");
    }

    #[test]
    fn removing_the_last_edition_removes_the_work() {
        let db = db();
        let a = save(&db, &audiobook()).unwrap();
        let e = save(&db, &ebook()).unwrap();
        delete_edition(&db, a.edition_id).unwrap();
        assert_eq!(work(&db, e.work_id).unwrap()["formats"], json!(["ebook"]));
        delete_edition(&db, e.edition_id).unwrap();
        assert_eq!(work(&db, e.work_id).unwrap_err().status.as_u16(), 404);
        assert_eq!(delete_edition(&db, e.edition_id).unwrap_err().status.as_u16(), 404);
    }

    #[test]
    fn an_edition_needs_its_id() {
        let db = db();
        let mut m = audiobook();
        m.asin.clear();
        assert_eq!(save(&db, &m).unwrap_err().status.as_u16(), 400);
    }
}
