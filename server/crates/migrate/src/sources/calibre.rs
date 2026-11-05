//! A Calibre library: `metadata.db` and a folder per book beside it (`Author/Title (id)/Title - Author.epub`).
//!
//! Each Calibre book becomes one ebook (`calibre:<id>`) with its ebook files, best format first (EPUB, KEPUB,
//! AZW3, MOBI, PDF, then the rest), and one audiobook (`calibre:<id>:audio`) when it also has audio files.
//! The ISBN comes from the `isbn` identifier, the ASIN from `audible`, `audible_asin` or `asin`.

use super::{columns, has_table, pick, rows};
use crate::plan::{year, Book, Format, Plan, SeriesPlace};
use crate::Result;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Ebook formats, best first; formats not named come after them.
const EBOOK_ORDER: &[&str] = &["EPUB", "KEPUB", "AZW3", "AZW", "MOBI", "PDF", "FB2", "CBZ", "CBR", "TXT", "RTF", "DOCX"];

/// Audio formats Calibre may hold.
const AUDIO: &[&str] = &["M4B", "M4A", "MP3", "OGG", "OPUS", "FLAC", "AAC", "WMA"];

/// Reads the library at `library` (the folder that holds `metadata.db`) into `plan`.
pub fn read(conn: &Connection, library: &Path, plan: &mut Plan) -> Result<()> {
    let bc = columns(conn, "books")?;
    let sql = format!(
        "SELECT b.id AS id, b.title AS title, b.path AS path, {}, {}, {}, {} FROM books b ORDER BY b.id",
        pick(&bc, "b", "pubdate", "pubdate"),
        pick(&bc, "b", "series_index", "series_index"),
        pick(&bc, "b", "has_cover", "has_cover"),
        pick(&bc, "b", "isbn", "old_isbn"),
    );
    let authors =
        list(conn, "SELECT l.book AS book, a.name AS v FROM books_authors_link l JOIN authors a ON a.id = l.author ORDER BY l.id")?;
    let series = if has_table(conn, "books_series_link")? {
        list(conn, "SELECT l.book AS book, s.name AS v FROM books_series_link l JOIN series s ON s.id = l.series ORDER BY l.id")?
    } else {
        HashMap::new()
    };
    let languages = if has_table(conn, "books_languages_link")? {
        list(conn, "SELECT l.book AS book, g.lang_code AS v FROM books_languages_link l JOIN languages g ON g.id = l.lang_code ORDER BY l.item_order, l.id")?
    } else {
        HashMap::new()
    };
    let mut ids: HashMap<String, HashMap<String, String>> = HashMap::new();
    if has_table(conn, "identifiers")? {
        for r in rows(conn, "SELECT book, type, val FROM identifiers")? {
            ids.entry(r.s("book")).or_default().insert(r.s("type").to_ascii_lowercase(), r.s("val"));
        }
    }
    let mut files: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in rows(conn, "SELECT book, format, name FROM data ORDER BY id")? {
        files.entry(r.s("book")).or_default().push((r.s("format").to_ascii_uppercase(), r.s("name")));
    }
    for r in rows(conn, &sql)? {
        let id = r.s("id");
        let title = r.s("title");
        let folder = library.join(r.s("path"));
        let mut ebook: Vec<(usize, PathBuf)> = vec![];
        let mut audio: Vec<PathBuf> = vec![];
        for (format, name) in files.get(&id).into_iter().flatten() {
            let file = folder.join(format!("{name}.{}", format.to_ascii_lowercase()));
            if AUDIO.contains(&format.as_str()) {
                audio.push(file);
            } else {
                ebook.push((EBOOK_ORDER.iter().position(|f| f == format).unwrap_or(EBOOK_ORDER.len()), file));
            }
        }
        if ebook.is_empty() && audio.is_empty() {
            plan.skip(plan.id(&id), format!("\u{201c}{title}\u{201d} has no book files in Calibre."));
            continue;
        }
        ebook.sort_by_key(|(rank, _)| *rank);
        let idents = ids.remove(&id).unwrap_or_default();
        let get = |keys: &[&str]| keys.iter().find_map(|k| idents.get(*k).filter(|v| !v.is_empty()).cloned()).unwrap_or_default();
        let position =
            r.f("series_index").map(|f| if f.fract() == 0.0 { format!("{}", f as i64) } else { f.to_string() }).unwrap_or_default();
        let base = Book {
            source_id: plan.id(&id),
            format: Format::Ebook,
            title: title.clone(),
            subtitle: String::new(),
            authors: authors.get(&id).cloned().unwrap_or_default(),
            narrators: vec![],
            series: series
                .get(&id)
                .into_iter()
                .flatten()
                .map(|name| SeriesPlace { name: name.clone(), position: position.clone() })
                .collect(),
            asin: get(&["audible", "audible_asin", "asin"]),
            isbn: Some(get(&["isbn"])).filter(|s| !s.is_empty()).unwrap_or_else(|| r.s("old_isbn")),
            language: languages.get(&id).and_then(|l| l.first()).cloned().unwrap_or_default(),
            year: year(&r.s("pubdate")),
            duration_secs: None,
            abridged: None,
            folder: Some(folder.clone()),
            files: vec![],
            cover: (r.b("has_cover") == Some(true)).then(|| folder.join("cover.jpg")),
        };
        if !audio.is_empty() {
            let mut a = Book { source_id: plan.id(format!("{id}:audio")), format: Format::Audiobook, files: audio, ..base.clone() };
            a.isbn.clear();
            plan.books.push(a);
        }
        if !ebook.is_empty() {
            // an ebook's ASIN would be a Kindle one, not Audible's
            plan.books.push(Book { files: ebook.into_iter().map(|(_, f)| f).collect(), asin: String::new(), ..base });
        }
    }
    Ok(())
}

/// Runs `sql` (columns `book` and `v`) and gathers the values per book, in order.
fn list(conn: &Connection, sql: &str) -> Result<HashMap<String, Vec<String>>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for r in rows(conn, sql)? {
        let v = r.s("v");
        if !v.is_empty() {
            out.entry(r.s("book")).or_default().push(v);
        }
    }
    Ok(out)
}
