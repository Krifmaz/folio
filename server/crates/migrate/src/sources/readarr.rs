//! Readarr (`readarr.db`): its wanted list, the monitored books of monitored authors that have no file yet.
//!
//! The exact edition is the book's monitored edition (its ISBN-13 and ASIN). Whether it is an audiobook comes from
//! the edition's format (`Audiobook`, `Audible Audio`, `MP3 CD` …), else from the author's quality profile: one that
//! allows only audio qualities (MP3, FLAC, M4B) means an audiobook instance. Anything else is an ebook.

use super::{columns, pick, rows};
use crate::plan::{Format, Plan, Wanted};
use crate::Result;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;

/// Readarr's audio qualities start at this id (MP3 10, FLAC 11, M4B 12, unknown audio 13).
const FIRST_AUDIO_QUALITY: i64 = 10;

/// Reads Readarr's wanted (missing) books into `plan`.
pub fn read(conn: &Connection, plan: &mut Plan) -> Result<()> {
    let ed = columns(conn, "Editions")?;
    let au = columns(conn, "Authors")?;
    let audio_profiles = audio_only_profiles(conn)?;
    let sql = format!(
        "SELECT b.Id AS id, b.Title AS title, am.Name AS author, e.Title AS edition_title, {}, {}, {}, {}, {}, {}
         FROM Books b
         LEFT JOIN AuthorMetadata am ON am.Id = b.AuthorMetadataId
         LEFT JOIN Authors a ON a.AuthorMetadataId = b.AuthorMetadataId
         LEFT JOIN Editions e ON e.Id = (SELECT e2.Id FROM Editions e2 WHERE e2.BookId = b.Id ORDER BY e2.Monitored DESC, e2.Id LIMIT 1)
         WHERE b.Monitored = 1 AND (a.Id IS NULL OR a.Monitored = 1)
           AND NOT EXISTS (SELECT 1 FROM BookFiles f JOIN Editions fe ON fe.Id = f.EditionId WHERE fe.BookId = b.Id)
         ORDER BY b.Id",
        pick(&ed, "e", "Isbn13", "isbn"),
        pick(&ed, "e", "Asin", "asin"),
        pick(&ed, "e", "Language", "language"),
        pick(&ed, "e", "Format", "format"),
        pick(&ed, "e", "IsEbook", "is_ebook"),
        pick(&au, "a", "QualityProfileId", "profile"),
    );
    for r in rows(conn, &sql)? {
        let title = Some(r.s("title")).filter(|t| !t.is_empty()).unwrap_or_else(|| r.s("edition_title"));
        if title.is_empty() {
            plan.skip(plan.id(r.s("id")), "Readarr has this book without a title.");
            continue;
        }
        let kind = r.s("format").to_ascii_lowercase();
        let audio = r.b("is_ebook") != Some(true)
            && (["audio", "audible", "mp3"].iter().any(|w| kind.contains(w))
                || r.i("profile").is_some_and(|p| audio_profiles.get(&p).copied().unwrap_or(false)));
        plan.wanted.push(Wanted {
            source_id: plan.id(r.s("id")),
            format: if audio { Format::Audiobook } else { Format::Ebook },
            title,
            authors: Some(r.s("author")).filter(|a| !a.is_empty()).into_iter().collect(),
            narrators: vec![],
            asin: r.s("asin"),
            isbn: r.s("isbn"),
            language: r.s("language"),
        });
    }
    Ok(())
}

/// Which quality profiles allow only audio: profile id → yes or no.
fn audio_only_profiles(conn: &Connection) -> Result<HashMap<i64, bool>> {
    if columns(conn, "QualityProfiles")?.is_empty() {
        return Ok(HashMap::new());
    }
    let mut out = HashMap::new();
    for r in rows(conn, "SELECT Id, Items FROM QualityProfiles")? {
        let mut allowed = vec![];
        if let Ok(items) = serde_json::from_str::<Value>(&r.s("Items")) {
            collect(&items, false, &mut allowed);
        }
        out.insert(r.i("Id").unwrap_or(0), !allowed.is_empty() && allowed.iter().all(|q| *q >= FIRST_AUDIO_QUALITY));
    }
    Ok(out)
}

/// Gathers the allowed quality ids of a profile's items (groups hold items of their own).
fn collect(v: &Value, group_allowed: bool, out: &mut Vec<i64>) {
    for item in v.as_array().into_iter().flatten() {
        let allowed = group_allowed || item["allowed"].as_bool() == Some(true);
        let quality = item["quality"].as_i64().or_else(|| item["quality"]["id"].as_i64());
        match quality {
            Some(q) if allowed => out.push(q),
            Some(_) => {}
            None => collect(&item["items"], allowed, out),
        }
    }
}
