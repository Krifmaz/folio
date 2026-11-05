//! LazyLibrarian (`lazylibrarian.db`): its wanted lists. A book whose `Status` is Wanted is a wanted ebook; one
//! whose `AudioStatus` is Wanted is a wanted audiobook (a book can be both).

use super::{columns, pick, rows};
use crate::plan::{names, Format, Plan, Wanted};
use crate::Result;
use rusqlite::Connection;

/// Reads LazyLibrarian's wanted ebooks and audiobooks into `plan`.
pub fn read(conn: &Connection, plan: &mut Plan) -> Result<()> {
    let bc = columns(conn, "books")?;
    let sql = format!(
        "SELECT b.BookID AS id, b.BookName AS title, a.AuthorName AS author, {}, {}, {}, {}, {} FROM books b
         LEFT JOIN authors a ON a.AuthorID = b.AuthorID ORDER BY b.rowid",
        pick(&bc, "b", "BookIsbn", "isbn"),
        pick(&bc, "b", "BookLang", "language"),
        pick(&bc, "b", "Narrator", "narrator"),
        pick(&bc, "b", "Status", "status"),
        pick(&bc, "b", "AudioStatus", "audio_status"),
    );
    for r in rows(conn, &sql)? {
        let id = r.s("id");
        for (format, status, suffix) in [(Format::Ebook, r.s("status"), ""), (Format::Audiobook, r.s("audio_status"), ":audio")] {
            if !status.eq_ignore_ascii_case("wanted") {
                continue;
            }
            let title = r.s("title");
            if title.is_empty() {
                plan.skip(plan.id(&id), "LazyLibrarian has this book without a title.");
                continue;
            }
            // LazyLibrarian's ISBN is the paper or ebook one; an audiobook's edition is found by title and narrator
            let isbn = if format == Format::Ebook { r.s("isbn") } else { String::new() };
            plan.wanted.push(Wanted {
                source_id: plan.id(format!("{id}{suffix}")),
                format,
                title,
                authors: Some(r.s("author")).filter(|a| !a.is_empty()).into_iter().collect(),
                narrators: if format == Format::Audiobook { names(&r.s("narrator")) } else { vec![] },
                asin: String::new(),
                isbn,
                language: r.s("language"),
            });
        }
    }
    Ok(())
}
