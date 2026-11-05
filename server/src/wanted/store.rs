//! Preferences (one profile per format) and the wanted list, in the database.
//!
//! A **profile** says what a good copy of a book is: the file formats in order of preference (M4B before MP3;
//! EPUB before AZW3 before PDF), the languages, and for audiobooks whether only unabridged ones will do. Every
//! wanted edition names its profile. The searching, verification and import steps read the same profile, so a
//! preference is said once.

use crate::metadata::{EditionMeta, Format};
use kit_server_core::db::now;
use kit_server_core::{err, p, Db, Migrations, Result, Row, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The wanted list's tables, step by step (a released step is never changed). The followed flags live on the
/// library's authors and series (`folio-library` step 2: `follow` is "", `audiobook`, `ebook` or `both`).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-wanted",
    steps: &["CREATE TABLE profiles (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            format TEXT NOT NULL,
            name TEXT NOT NULL,
            file_formats TEXT NOT NULL,
            languages TEXT NOT NULL,
            unabridged_only INTEGER NOT NULL DEFAULT 0,
            created_at REAL NOT NULL
        );
        INSERT INTO profiles (format, name, file_formats, languages, unabridged_only, created_at)
            VALUES ('audiobook', 'Audiobooks', '[\"m4b\",\"mp3\"]', '[\"en\"]', 1, 0),
                   ('ebook', 'Ebooks', '[\"epub\",\"azw3\",\"pdf\"]', '[\"en\"]', 0, 0);
        CREATE TABLE wanted (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            edition_id INTEGER NOT NULL UNIQUE,
            profile_id INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'wanted',
            user_id INTEGER NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'manual',
            note TEXT NOT NULL DEFAULT '',
            added_at REAL NOT NULL,
            updated_at REAL NOT NULL,
            searched_at REAL
        );
        CREATE INDEX wanted_status ON wanted (status);
        CREATE TABLE follow_seen (
            source TEXT NOT NULL,
            key TEXT NOT NULL,
            outcome TEXT NOT NULL,
            seen_at REAL NOT NULL,
            PRIMARY KEY (source, key)
        );"],
};

/// Makes the wanted list's tables (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The wanted list's tables can't be made: {}", e.detail))
}

/// Where a wanted edition is on its way into the library.
pub const STATUSES: &[&str] = &["wanted", "searching", "downloading", "downloaded", "review", "imported", "failed"];

/// What a good copy of a book in one format is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// the profile's id
    #[serde(default)]
    pub id: i64,
    /// the format it is for
    pub format: Format,
    /// its name
    pub name: String,
    /// the file formats that will do, best first (`m4b`, `mp3`; `epub`, `azw3`, `pdf`)
    pub file_formats: Vec<String>,
    /// the languages that will do, two letters; empty: any
    pub languages: Vec<String>,
    /// audiobooks: only unabridged ones will do
    pub unabridged_only: bool,
}

/// The file formats Folio knows for each format, the order a new profile starts with.
pub fn known_file_formats(f: Format) -> &'static [&'static str] {
    match f {
        Format::Audiobook => &["m4b", "mp3", "m4a", "flac", "ogg", "opus"],
        Format::Ebook => &["epub", "azw3", "mobi", "pdf"],
    }
}

impl Profile {
    /// The profile of a database row.
    pub fn from_row(r: &Row) -> Profile {
        let list = |k: &str| serde_json::from_str::<Vec<String>>(&r.s(k)).unwrap_or_default();
        Profile {
            id: r.i("id"),
            format: Format::parse(&r.s("format")).unwrap_or_default(),
            name: r.s("name"),
            file_formats: list("file_formats"),
            languages: list("languages"),
            unabridged_only: r.b("unabridged_only"),
        }
    }

    /// Why the edition `m` won't do for this profile, in plain words; `None` when it will. A language the
    /// source does not say is let through here; the files are checked again when they arrive.
    pub fn refuses(&self, m: &EditionMeta) -> Option<String> {
        let what = match m.format {
            Format::Audiobook => "audiobook",
            Format::Ebook => "ebook",
        };
        if m.format != self.format {
            return Some(format!("This is an {what}, and the {} preferences are for {}s.", self.name, self.format.as_str()));
        }
        if !self.languages.is_empty() && !m.language.is_empty() && !self.languages.iter().any(|l| l == &m.language) {
            return Some(format!(
                "This {what} is in {}, and the {} preferences want {}.",
                language_name(&m.language),
                self.name,
                self.languages.iter().map(|l| language_name(l)).collect::<Vec<_>>().join(" or ")
            ));
        }
        if self.format == Format::Audiobook && self.unabridged_only && m.abridged == Some(true) {
            return Some(format!("This audiobook is abridged, and the {} preferences want unabridged ones only.", self.name));
        }
        None
    }

    /// Where the file format `ext` (`m4b`, `.EPUB`) stands in this profile: 0 is the best; `None` when it won't
    /// do.
    pub fn file_rank(&self, ext: &str) -> Option<usize> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        self.file_formats.iter().position(|f| *f == ext)
    }

    /// The profile checked: a name, known file formats (at least one, no repeats) and two-letter languages.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(err(400, "Give the preferences a name."));
        }
        if self.file_formats.is_empty() {
            return Err(err(400, "Pick at least one file format."));
        }
        let known = known_file_formats(self.format);
        for (i, f) in self.file_formats.iter().enumerate() {
            if !known.contains(&f.as_str()) {
                return Err(err(
                    400,
                    format!("\"{f}\" isn't a file format Folio knows for {}s. These are: {}.", self.format.as_str(), known.join(", ")),
                ));
            }
            if self.file_formats[..i].contains(f) {
                return Err(err(400, format!("{f} is listed twice.")));
            }
        }
        for l in &self.languages {
            if l.len() != 2 || !l.bytes().all(|b| b.is_ascii_lowercase()) {
                return Err(err(400, format!("\"{l}\" is not a two-letter language code like en or de.")));
            }
        }
        Ok(())
    }
}

/// A language's English name for messages (`de` → `German`); the code itself when not in the list.
pub fn language_name(code: &str) -> String {
    let n = match code {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        "nl" => "Dutch",
        "pt" => "Portuguese",
        "sv" => "Swedish",
        "da" => "Danish",
        "no" => "Norwegian",
        "fi" => "Finnish",
        "pl" => "Polish",
        "ru" => "Russian",
        "ja" => "Japanese",
        "zh" => "Chinese",
        "ko" => "Korean",
        other => return other.to_string(),
    };
    n.to_string()
}

/// Every profile.
pub fn profiles(db: &SqliteDb) -> Result<Vec<Profile>> {
    Ok(db.query("SELECT * FROM profiles ORDER BY format, id", p![])?.iter().map(Profile::from_row).collect())
}

/// The profile `id` (404 when there is none).
pub fn profile(db: &SqliteDb, id: i64) -> Result<Profile> {
    db.one("SELECT * FROM profiles WHERE id=?", p![id])?
        .map(|r| Profile::from_row(&r))
        .ok_or_else(|| err(404, "Those preferences don't exist any more."))
}

/// The first profile for `format` (the one made at the start).
pub fn default_profile(db: &SqliteDb, format: Format) -> Result<Profile> {
    db.one("SELECT * FROM profiles WHERE format=? ORDER BY id LIMIT 1", p![format.as_str()])?
        .map(|r| Profile::from_row(&r))
        .ok_or_else(|| err(500, format!("There are no {} preferences.", format.as_str())))
}

/// Saves the profile `id` with `p`'s name, file formats, languages and unabridged rule (its format stays).
pub fn update_profile(db: &SqliteDb, id: i64, mut pr: Profile) -> Result<Profile> {
    let old = profile(db, id)?;
    pr.format = old.format;
    pr.file_formats = pr.file_formats.iter().map(|f| f.trim().trim_start_matches('.').to_ascii_lowercase()).collect();
    pr.languages = pr.languages.iter().map(|l| crate::library::text::language(l)).filter(|l| !l.is_empty()).collect();
    pr.languages.dedup();
    pr.validate()?;
    db.execute(
        "UPDATE profiles SET name=?, file_formats=?, languages=?, unabridged_only=? WHERE id=?",
        p![pr.name.trim(), json!(pr.file_formats).to_string(), json!(pr.languages).to_string(), pr.unabridged_only, id],
    )?;
    profile(db, id)
}

/// Wants the edition `edition_id` with the profile `profile_id` (the format's first when `None`), as the
/// account `user_id`, from `source` (`manual`, `follow`). Refused (400, the reason) when the profile won't take
/// it; already wanted: the same row back.
pub fn want(db: &SqliteDb, edition_id: i64, profile_id: Option<i64>, user_id: i64, source: &str) -> Result<Value> {
    let (_, m) = crate::library::store::edition(db, edition_id)?;
    let pr = match profile_id {
        Some(id) => profile(db, id)?,
        None => default_profile(db, m.format)?,
    };
    if let Some(why) = pr.refuses(&m) {
        return Err(err(400, why));
    }
    if let Some(r) = db.one("SELECT id FROM wanted WHERE edition_id=?", p![edition_id])? {
        return wanted_one(db, r.i("id"));
    }
    let t = now();
    let id = db.execute(
        "INSERT INTO wanted (edition_id, profile_id, status, user_id, source, added_at, updated_at) VALUES (?,?,?,?,?,?,?)",
        p![edition_id, pr.id, "wanted", user_id, source, t, t],
    )?;
    wanted_one(db, id)
}

/// The SQL of a wanted row with its edition and profile.
const WANTED_SQL: &str = "SELECT w.*, e.format, e.meta, e.work_id, p.name AS profile_name FROM wanted w
    JOIN editions e ON e.id = w.edition_id JOIN profiles p ON p.id = w.profile_id";

/// A wanted row as clients get it: the wanted entry and the edition's main facts.
fn wanted_json(r: &Row) -> Value {
    let m: EditionMeta = serde_json::from_str(&r.s("meta")).unwrap_or_default();
    json!({
        "id": r.i("id"),
        "status": r.s("status"),
        "source": r.s("source"),
        "note": r.s("note"),
        "added_at": r.f("added_at"),
        "searched_at": r.get("searched_at").cloned().unwrap_or(Value::Null),
        "profile_id": r.i("profile_id"),
        "profile": r.s("profile_name"),
        "user_id": r.i("user_id"),
        "edition": {
            "id": r.i("edition_id"),
            "work_id": r.i("work_id"),
            "format": r.s("format"),
            "title": m.title,
            "authors": m.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
            "narrators": m.narrators,
            "asin": m.asin,
            "isbn13": m.isbn13,
            "language": m.language,
            "runtime_min": m.runtime_min,
            "release_date": m.release_date,
            "cover_url": m.cover_url,
            "series": m.series,
        },
    })
}

/// The wanted entry `id` (404 when there is none).
pub fn wanted_one(db: &SqliteDb, id: i64) -> Result<Value> {
    db.one(&format!("{WANTED_SQL} WHERE w.id=?"), p![id])?
        .map(|r| wanted_json(&r))
        .ok_or_else(|| err(404, "That book isn't on the wanted list any more."))
}

/// The wanted list, newest first, of one format and one status when given.
pub fn wanted(db: &SqliteDb, format: Option<Format>, status: &str) -> Result<Vec<Value>> {
    Ok(db
        .query(
            &format!("{WANTED_SQL} WHERE (?1 = '' OR e.format = ?1) AND (?2 = '' OR w.status = ?2) ORDER BY w.added_at DESC, w.id DESC"),
            p![format.map(Format::as_str).unwrap_or(""), status],
        )?
        .iter()
        .map(wanted_json)
        .collect())
}

/// Moves the wanted entry `id` to `status`, with a note (why it failed, what arrived).
pub fn set_status(db: &SqliteDb, id: i64, status: &str, note: &str) -> Result<()> {
    if !STATUSES.contains(&status) {
        return Err(err(400, format!("\"{status}\" is not a wanted status.")));
    }
    db.execute("UPDATE wanted SET status=?, note=?, updated_at=? WHERE id=?", p![status, note, now(), id])?;
    Ok(())
}

/// Gives the wanted entry `id` another profile of the same format.
pub fn set_profile(db: &SqliteDb, id: i64, profile_id: i64) -> Result<Value> {
    let r = db
        .one("SELECT edition_id FROM wanted WHERE id=?", p![id])?
        .ok_or_else(|| err(404, "That book isn't on the wanted list any more."))?;
    let (_, m) = crate::library::store::edition(db, r.i("edition_id"))?;
    let pr = profile(db, profile_id)?;
    if let Some(why) = pr.refuses(&m) {
        return Err(err(400, why));
    }
    db.execute("UPDATE wanted SET profile_id=?, updated_at=? WHERE id=?", p![profile_id, now(), id])?;
    wanted_one(db, id)
}

/// Takes the edition `id` off the wanted list (404 when it isn't on it).
pub fn unwant(db: &SqliteDb, id: i64) -> Result<()> {
    if db.one("SELECT id FROM wanted WHERE id=?", p![id])?.is_none() {
        return Err(err(404, "That book isn't on the wanted list any more."));
    }
    db.execute("DELETE FROM wanted WHERE id=?", p![id])?;
    Ok(())
}

/// Whether something a followed author or series offered (`source`, `key`) was already looked at.
pub fn seen(db: &SqliteDb, source: &str, key: &str) -> Result<bool> {
    Ok(db.one("SELECT 1 AS x FROM follow_seen WHERE source=? AND key=?", p![source, key])?.is_some())
}

/// Records what came of something a followed author or series offered.
pub fn mark_seen(db: &SqliteDb, source: &str, key: &str, outcome: &str) -> Result<()> {
    db.execute(
        "INSERT INTO follow_seen (source, key, outcome, seen_at) VALUES (?,?,?,?) ON CONFLICT (source, key) DO UPDATE SET outcome=excluded.outcome, seen_at=excluded.seen_at",
        p![source, key, outcome, now()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Person;

    fn db() -> SqliteDb {
        let db = SqliteDb::memory();
        crate::library::store::migrate(&db).unwrap();
        migrate(&db).unwrap();
        db
    }

    fn book(abridged: bool, language: &str) -> EditionMeta {
        EditionMeta {
            format: Format::Audiobook,
            asin: if abridged { "B0ABRIDGE1".into() } else { "B08G9PRS1K".into() },
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            language: language.into(),
            abridged: Some(abridged),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_profiles_say_m4b_before_mp3_and_epub_before_azw3_before_pdf() {
        let db = db();
        let a = default_profile(&db, Format::Audiobook).unwrap();
        assert_eq!((a.file_formats.clone(), a.unabridged_only), (vec!["m4b".to_string(), "mp3".to_string()], true));
        assert_eq!((a.file_rank("M4B"), a.file_rank(".mp3"), a.file_rank("flac")), (Some(0), Some(1), None));
        let e = default_profile(&db, Format::Ebook).unwrap();
        assert_eq!(e.file_formats, vec!["epub", "azw3", "pdf"]);
    }

    #[test]
    fn profiles_refuse_abridged_and_other_languages_in_plain_words() {
        let db = db();
        let a = default_profile(&db, Format::Audiobook).unwrap();
        assert_eq!(a.refuses(&book(false, "en")), None);
        assert_eq!(a.refuses(&book(false, "")), None, "unknown languages are checked on the files");
        assert_eq!(
            a.refuses(&book(true, "en")).unwrap(),
            "This audiobook is abridged, and the Audiobooks preferences want unabridged ones only."
        );
        assert_eq!(a.refuses(&book(false, "de")).unwrap(), "This audiobook is in German, and the Audiobooks preferences want English.");
        let e = default_profile(&db, Format::Ebook).unwrap();
        assert!(e.refuses(&book(false, "en")).unwrap().contains("are for ebooks"));
    }

    #[test]
    fn wanting_checks_the_profile_and_keeps_one_entry_per_edition() {
        let db = db();
        let ok = crate::library::store::save(&db, &book(false, "en")).unwrap();
        let bad = crate::library::store::save(&db, &book(true, "en")).unwrap();
        let w = want(&db, ok.edition_id, None, 1, "manual").unwrap();
        assert_eq!(
            (w["status"].as_str(), w["profile"].as_str(), w["edition"]["asin"].as_str()),
            (Some("wanted"), Some("Audiobooks"), Some("B08G9PRS1K"))
        );
        assert_eq!(want(&db, ok.edition_id, None, 1, "manual").unwrap()["id"], w["id"]);
        assert_eq!(want(&db, bad.edition_id, None, 1, "manual").unwrap_err().status.as_u16(), 400);
        assert_eq!(wanted(&db, Some(Format::Audiobook), "").unwrap().len(), 1);
        assert!(wanted(&db, Some(Format::Ebook), "").unwrap().is_empty());
        let id = w["id"].as_i64().unwrap();
        set_status(&db, id, "searching", "").unwrap();
        assert_eq!(wanted(&db, None, "searching").unwrap().len(), 1);
        assert_eq!(set_status(&db, id, "lost", "").unwrap_err().status.as_u16(), 400);
        unwant(&db, id).unwrap();
        assert_eq!(unwant(&db, id).unwrap_err().status.as_u16(), 404);
    }

    #[test]
    fn profiles_are_edited_and_checked() {
        let db = db();
        let a = default_profile(&db, Format::Audiobook).unwrap();
        let mut b = a.clone();
        b.file_formats = vec!["MP3".into(), ".m4b".into()];
        b.languages = vec!["English".into(), "de".into()];
        b.unabridged_only = false;
        let saved = update_profile(&db, a.id, b.clone()).unwrap();
        assert_eq!(
            (saved.file_formats, saved.languages, saved.unabridged_only),
            (vec!["mp3".to_string(), "m4b".to_string()], vec!["en".to_string(), "de".to_string()], false)
        );
        b.file_formats = vec!["epub".into()];
        assert_eq!(
            update_profile(&db, a.id, b.clone()).unwrap_err().detail,
            "\"epub\" isn't a file format Folio knows for audiobooks. These are: m4b, mp3, m4a, flac, ogg, opus."
        );
        b.file_formats = vec![];
        assert_eq!(update_profile(&db, a.id, b).unwrap_err().detail, "Pick at least one file format.");
    }
}
