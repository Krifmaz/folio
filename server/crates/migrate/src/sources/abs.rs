//! Audiobookshelf (2.3 and later keeps everything in `absdatabase.sqlite`).
//!
//! Each book library item becomes an audiobook (`abs:<item id>`) when it has audio files and an ebook
//! (`abs:<item id>:ebook`) when it has an ebook file; an item can be both. Podcasts are left out. Progress comes
//! from `mediaProgresses` (the listening time, and the ebook's CFI and fraction), bookmarks from each user's
//! `bookmarks`.

use super::{columns, pick, rows, Rec};
use crate::plan::{epoch, names, year, Book, Bookmark, Format, Plan, Progress, SeriesPlace};
use crate::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Reads the book library items, their progress and bookmarks into `plan`.
pub fn read(conn: &Connection, plan: &mut Plan) -> Result<()> {
    let li = columns(conn, "libraryItems")?;
    let bk = columns(conn, "books")?;
    let sql = format!(
        "SELECT li.id AS item, li.mediaType AS media_type, {}, {}, b.id AS book, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
         FROM libraryItems li LEFT JOIN books b ON b.id = li.mediaId ORDER BY li.id",
        pick(&li, "li", "path", "path"),
        pick(&li, "li", "isMissing", "missing"),
        pick(&bk, "b", "title", "title"),
        pick(&bk, "b", "subtitle", "subtitle"),
        pick(&bk, "b", "publishedYear", "year"),
        pick(&bk, "b", "isbn", "isbn"),
        pick(&bk, "b", "asin", "asin"),
        pick(&bk, "b", "language", "language"),
        pick(&bk, "b", "abridged", "abridged"),
        pick(&bk, "b", "duration", "duration"),
        pick(&bk, "b", "coverPath", "cover"),
        pick(&bk, "b", "narrators", "narrators"),
        pick(&bk, "b", "audioFiles", "audio_files"),
        pick(&bk, "b", "ebookFile", "ebook_file"),
    );
    let authors = people(conn, "bookAuthors", "authorId", "authors")?;
    let series = series(conn)?;
    // which source ids the media (book) ids became, for the progress
    let mut media: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    let mut podcasts = 0;
    for r in rows(conn, &sql)? {
        let item = r.s("item");
        if r.s("media_type") != "book" {
            podcasts += 1;
            continue;
        }
        if r.s("book").is_empty() {
            plan.skip(plan.id(&item), "Audiobookshelf has this item but no book details for it.");
            continue;
        }
        let title = r.s("title");
        if r.b("missing") == Some(true) {
            plan.skip(plan.id(&item), format!("Audiobookshelf marks the files of \u{201c}{title}\u{201d} as missing."));
            continue;
        }
        let book = r.s("book");
        let folder = Some(r.s("path")).filter(|p| !p.is_empty()).map(PathBuf::from);
        let base = Book {
            source_id: String::new(),
            format: Format::Audiobook,
            title: title.clone(),
            subtitle: r.s("subtitle"),
            authors: authors.get(&book).cloned().unwrap_or_default(),
            narrators: names(&r.s("narrators")),
            series: series.get(&book).cloned().unwrap_or_default(),
            asin: r.s("asin"),
            isbn: r.s("isbn"),
            language: r.s("language"),
            year: year(&r.s("year")),
            duration_secs: None,
            abridged: r.b("abridged"),
            folder,
            files: vec![],
            cover: Some(r.s("cover")).filter(|p| !p.is_empty()).map(PathBuf::from),
        };
        let audio = audio_files(&r);
        let ebook = ebook_file(&r);
        let mut ids = (None, None);
        if !audio.is_empty() {
            let id = plan.id(&item);
            ids.0 = Some(id.clone());
            plan.books.push(Book { source_id: id, files: audio, duration_secs: r.f("duration").filter(|d| *d > 0.0), ..base.clone() });
        }
        if let Some(f) = ebook {
            let id = plan.id(format!("{item}:ebook"));
            ids.1 = Some(id.clone());
            plan.books.push(Book { source_id: id, format: Format::Ebook, files: vec![f], narrators: vec![], abridged: None, ..base });
        }
        if ids == (None, None) {
            plan.skip(plan.id(&item), format!("\u{201c}{title}\u{201d} has no audio or ebook files in Audiobookshelf."));
            continue;
        }
        media.insert(book, ids);
    }
    if podcasts > 0 {
        plan.skip(
            "podcasts",
            format!("{podcasts} podcast{} left out: Folio is for audiobooks and ebooks.", if podcasts == 1 { " is" } else { "s are" }),
        );
    }
    let users = users(conn)?;
    progress(conn, plan, &media, &users)?;
    bookmarks(plan, &users);
    Ok(())
}

/// The audio files of a book row, in playing order, without the ones ABS was told to leave out.
fn audio_files(r: &Rec) -> Vec<PathBuf> {
    let Ok(Value::Array(files)) = serde_json::from_str::<Value>(&r.s("audio_files")) else { return vec![] };
    let mut files: Vec<&Value> = files.iter().filter(|f| f["exclude"].as_bool() != Some(true)).collect();
    files.sort_by_key(|f| f["index"].as_i64().unwrap_or(i64::MAX));
    files.iter().filter_map(|f| f["metadata"]["path"].as_str()).filter(|p| !p.is_empty()).map(PathBuf::from).collect()
}

/// The ebook file of a book row, when it has one.
fn ebook_file(r: &Rec) -> Option<PathBuf> {
    let v: Value = serde_json::from_str(&r.s("ebook_file")).ok()?;
    v["metadata"]["path"].as_str().filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// The names linked to each book through `link` (`bookAuthors`), in the order they were added.
fn people(conn: &Connection, link: &str, key: &str, table: &str) -> Result<HashMap<String, Vec<String>>> {
    let order = if columns(conn, link)?.contains("createdat") { "l.createdAt, l.rowid" } else { "l.rowid" };
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for r in
        rows(conn, &format!("SELECT l.bookId AS book, t.name AS name FROM {link} l JOIN {table} t ON t.id = l.{key} ORDER BY {order}"))?
    {
        let n = r.s("name");
        if !n.is_empty() {
            out.entry(r.s("book")).or_default().push(n);
        }
    }
    Ok(out)
}

/// Each book's series with its place.
fn series(conn: &Connection) -> Result<HashMap<String, Vec<SeriesPlace>>> {
    let mut out: HashMap<String, Vec<SeriesPlace>> = HashMap::new();
    for r in rows(conn, "SELECT l.bookId AS book, s.name AS name, l.sequence AS position FROM bookSeries l JOIN series s ON s.id = l.seriesId ORDER BY l.rowid")? {
        out.entry(r.s("book")).or_default().push(SeriesPlace { name: r.s("name"), position: r.s("position") });
    }
    Ok(out)
}

/// A user: their name and their bookmarks (as ABS keeps them).
struct User {
    /// the user name
    name: String,
    /// `users.bookmarks`: `[{libraryItemId, time, title}]`
    bookmarks: Value,
}

/// Every user by id.
fn users(conn: &Connection) -> Result<HashMap<String, User>> {
    let cols = columns(conn, "users")?;
    if cols.is_empty() {
        return Ok(HashMap::new());
    }
    let sql =
        format!("SELECT u.id AS id, {}, {} FROM users u", pick(&cols, "u", "username", "name"), pick(&cols, "u", "bookmarks", "bookmarks"));
    Ok(rows(conn, &sql)?
        .into_iter()
        .map(|r| (r.s("id"), User { name: r.s("name"), bookmarks: serde_json::from_str(&r.s("bookmarks")).unwrap_or(Value::Null) }))
        .collect())
}

/// The listening and reading progress of the books read.
fn progress(
    conn: &Connection,
    plan: &mut Plan,
    media: &HashMap<String, (Option<String>, Option<String>)>,
    users: &HashMap<String, User>,
) -> Result<()> {
    let cols = columns(conn, "mediaProgresses")?;
    let sql = format!(
        "SELECT p.userId AS user, p.mediaItemId AS media, {}, {}, {}, {}, {}, {}, {} FROM mediaProgresses p ORDER BY p.rowid",
        pick(&cols, "p", "mediaItemType", "kind"),
        pick(&cols, "p", "duration", "duration"),
        pick(&cols, "p", "currentTime", "time"),
        pick(&cols, "p", "isFinished", "finished"),
        pick(&cols, "p", "ebookLocation", "cfi"),
        pick(&cols, "p", "ebookProgress", "ebook_fraction"),
        pick(&cols, "p", "updatedAt", "updated"),
    );
    for r in rows(conn, &sql)? {
        if r.s("kind") == "podcastEpisode" {
            continue;
        }
        let Some((audio, ebook)) = media.get(&r.s("media")) else { continue };
        let Some(user) = users.get(&r.s("user")).map(|u| u.name.clone()).filter(|n| !n.is_empty()) else { continue };
        let finished = r.b("finished") == Some(true);
        let updated_at = epoch(&r.s("updated"));
        let time = r.f("time").filter(|t| *t > 0.0);
        let cfi = r.s("cfi");
        if let Some(book) = audio.clone().filter(|_| time.is_some() || finished || ebook.is_none()) {
            let fraction = match (time, r.f("duration").filter(|d| *d > 0.0)) {
                (Some(t), Some(d)) => Some((t / d).clamp(0.0, 1.0)),
                _ if finished => Some(1.0),
                _ => None,
            };
            plan.progress.push(Progress {
                user: user.clone(),
                book,
                format: Format::Audiobook,
                position_ms: time.map(|t| (t * 1000.0).round() as i64),
                fraction,
                cfi: String::new(),
                finished,
                updated_at,
            });
        }
        if let Some(book) = ebook.clone().filter(|_| !cfi.is_empty() || r.f("ebook_fraction").is_some_and(|f| f > 0.0) || audio.is_none()) {
            let fraction = r.f("ebook_fraction").map(|f| f.clamp(0.0, 1.0));
            plan.progress.push(Progress {
                user,
                book,
                format: Format::Ebook,
                position_ms: None,
                fraction: if finished && audio.is_none() { Some(1.0) } else { fraction },
                cfi,
                finished: finished && audio.is_none(),
                updated_at,
            });
        }
    }
    Ok(())
}

/// The users' bookmarks in the audiobooks read.
fn bookmarks(plan: &mut Plan, users: &HashMap<String, User>) {
    let audiobooks: std::collections::HashSet<String> =
        plan.books.iter().filter(|b| b.format == Format::Audiobook).map(|b| b.source_id.clone()).collect();
    let mut ids: Vec<&String> = users.keys().collect();
    ids.sort();
    for id in ids {
        let u = &users[id];
        let Some(list) = u.bookmarks.as_array() else { continue };
        for b in list {
            let book = plan.id(b["libraryItemId"].as_str().unwrap_or(""));
            let Some(time) = b["time"].as_f64() else { continue };
            if !audiobooks.contains(&book) || u.name.is_empty() {
                continue;
            }
            plan.bookmarks.push(Bookmark {
                user: u.name.clone(),
                book,
                position_ms: (time * 1000.0).round() as i64,
                title: b["title"].as_str().unwrap_or("").trim().to_string(),
            });
        }
    }
}
