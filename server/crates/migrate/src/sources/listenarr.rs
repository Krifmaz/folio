//! Listenarr (`listenarr.db`): its wanted audiobooks, the monitored ones with no file yet (no row in
//! `AudiobookFiles` and no `FilePath`), each with its Audible ASIN and narrators. Lists are kept as JSON arrays.

use super::{columns, pick, rows};
use crate::plan::{names, Format, Plan, Wanted};
use crate::Result;
use rusqlite::Connection;

/// Reads Listenarr's wanted audiobooks into `plan`.
pub fn read(conn: &Connection, plan: &mut Plan) -> Result<()> {
    let c = columns(conn, "Audiobooks")?;
    let has_path = c.contains("filepath");
    let sql = format!(
        "SELECT a.Id AS id, {}, {}, {}, {}, {} FROM Audiobooks a
         WHERE a.Monitored = 1 AND NOT EXISTS (SELECT 1 FROM AudiobookFiles f WHERE f.AudiobookId = a.Id){}
         ORDER BY a.Id",
        pick(&c, "a", "Title", "title"),
        pick(&c, "a", "Authors", "authors"),
        pick(&c, "a", "Narrators", "narrators"),
        pick(&c, "a", "Asin", "asin"),
        pick(&c, "a", "Language", "language"),
        if has_path { " AND (a.FilePath IS NULL OR trim(a.FilePath) = '')" } else { "" },
    );
    for r in rows(conn, &sql)? {
        let title = r.s("title");
        if title.is_empty() {
            plan.skip(plan.id(r.s("id")), "Listenarr has this audiobook without a title.");
            continue;
        }
        plan.wanted.push(Wanted {
            source_id: plan.id(r.s("id")),
            format: Format::Audiobook,
            title,
            authors: names(&r.s("authors")),
            narrators: names(&r.s("narrators")),
            asin: r.s("asin"),
            // an audiobook's ISBN isn't how Folio finds its edition; the ASIN is
            isbn: String::new(),
            language: r.s("language"),
        });
    }
    Ok(())
}
