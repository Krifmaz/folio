//! Reads what another app knows about your books into one [`Plan`] Folio can show as a dry run and then bring in.
//!
//! | Source | Point it at | What it gives |
//! | --- | --- | --- |
//! | [Audiobookshelf](Source::Audiobookshelf) | `absdatabase.sqlite` (in ABS's `/config`) | audiobooks and ebooks with their files, listening and reading progress, bookmarks |
//! | [Calibre](Source::Calibre) | the library folder, or its `metadata.db` | ebooks (and any audiobooks) with their files |
//! | [Calibre-Web](Source::CalibreWeb) | `app.db` | read and reading marks and Kobo reading positions for the Calibre books |
//! | [Readarr](Source::Readarr) | `readarr.db` | the wanted list: monitored books without a file |
//! | [LazyLibrarian](Source::LazyLibrarian) | `lazylibrarian.db` | the wanted list: books and audiobooks marked Wanted |
//! | [Listenarr](Source::Listenarr) | `listenarr.db` | the wanted list: monitored audiobooks without a file |
//!
//! **Nothing is ever changed in the source.** A database is copied (with its write-ahead log) into a scratch
//! folder the caller gives, read there, and the copy is removed ([`snapshot`]). The source's files are only
//! listed, never opened for writing.
//!
//! The plan is a faithful reading: titles, people, ASINs and ISBNs as the source has them. Matching them to
//! Folio's editions (looking them up, finding what is already there) is the server's part of the migration.

pub mod plan;
pub mod snapshot;
pub mod sources;

pub use plan::{Book, Bookmark, Format, Plan, Progress, SeriesPlace, Skipped, Summary, Wanted};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

/// The apps Folio can bring books, wanted lists and progress in from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Audiobookshelf's database (`absdatabase.sqlite`, ABS 2.3 and later)
    Audiobookshelf,
    /// a Calibre library (`metadata.db` and the book folders beside it)
    Calibre,
    /// Calibre-Web's own database (`app.db`): who read what in the Calibre library
    #[serde(rename = "calibre-web")]
    CalibreWeb,
    /// Readarr's database (`readarr.db`)
    Readarr,
    /// LazyLibrarian's database (`lazylibrarian.db`)
    LazyLibrarian,
    /// Listenarr's database (`listenarr.db`)
    Listenarr,
}

impl Source {
    /// Every source, in the order a person would look for theirs.
    pub const ALL: [Source; 6] =
        [Source::Audiobookshelf, Source::Calibre, Source::CalibreWeb, Source::Readarr, Source::LazyLibrarian, Source::Listenarr];

    /// The source's name as its users know it (`"Calibre-Web"`).
    pub fn name(self) -> &'static str {
        match self {
            Source::Audiobookshelf => "Audiobookshelf",
            Source::Calibre => "Calibre",
            Source::CalibreWeb => "Calibre-Web",
            Source::Readarr => "Readarr",
            Source::LazyLibrarian => "LazyLibrarian",
            Source::Listenarr => "Listenarr",
        }
    }

    /// The short word used in ids and the API (`"abs"`, `"calibre-web"`): a book read from Calibre is
    /// `calibre:<id>`, which is also how Calibre-Web's progress points at it.
    pub fn key(self) -> &'static str {
        match self {
            Source::Audiobookshelf => "abs",
            Source::Calibre => "calibre",
            Source::CalibreWeb => "calibre-web",
            Source::Readarr => "readarr",
            Source::LazyLibrarian => "lazylibrarian",
            Source::Listenarr => "listenarr",
        }
    }

    /// The source for a key or name as typed (`"abs"`, `"Audiobookshelf"`, `"calibreweb"`); `None` for another word.
    pub fn parse(s: &str) -> Option<Source> {
        let f: String = s.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        Source::ALL.into_iter().find(|src| {
            let key: String = src.key().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            f == key || f == src.name().to_lowercase().replace('-', "")
        })
    }

    /// The database file to read when `path` is a folder: the file the source keeps its data in
    /// (`metadata.db` in a Calibre library).
    pub fn file_name(self) -> &'static str {
        match self {
            Source::Audiobookshelf => "absdatabase.sqlite",
            Source::Calibre => "metadata.db",
            Source::CalibreWeb => "app.db",
            Source::Readarr => "readarr.db",
            Source::LazyLibrarian => "lazylibrarian.db",
            Source::Listenarr => "listenarr.db",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a source couldn't be read, in a plain sentence that says what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl Error {
    /// An error with this sentence.
    pub fn new(s: impl Into<String>) -> Error {
        Error(s.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A result whose error is a plain sentence.
pub type Result<T> = std::result::Result<T, Error>;

/// Reads `source` at `path` (its database file, or the folder that holds it) into a plan, using `scratch` (a
/// writable folder, such as one inside Folio's data folder) for the copy it reads. Nothing at `path` is changed.
pub fn read(source: Source, path: &Path, scratch: &Path) -> Result<Plan> {
    let file = snapshot::database_file(source, path)?;
    let snap = snapshot::Snapshot::take(&file, scratch)?;
    let conn = snap.conn();
    sources::expect(source, conn)?;
    let mut plan = Plan::new(source, &file);
    match source {
        Source::Audiobookshelf => sources::abs::read(conn, &mut plan)?,
        Source::Calibre => sources::calibre::read(conn, file.parent().unwrap_or(Path::new("")), &mut plan)?,
        Source::CalibreWeb => sources::calibre_web::read(conn, &mut plan)?,
        Source::Readarr => sources::readarr::read(conn, &mut plan)?,
        Source::LazyLibrarian => sources::lazylibrarian::read(conn, &mut plan)?,
        Source::Listenarr => sources::listenarr::read(conn, &mut plan)?,
    }
    Ok(plan)
}

/// Which source the database at `path` (a file) belongs to, by the tables in it; `None` when it is none of them.
pub fn detect(path: &Path, scratch: &Path) -> Result<Option<Source>> {
    let snap = snapshot::Snapshot::take(path, scratch)?;
    Ok(Source::ALL.into_iter().find(|s| sources::expect(*s, snap.conn()).is_ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_found_by_key_or_name() {
        assert_eq!(Source::parse("abs"), Some(Source::Audiobookshelf));
        assert_eq!(Source::parse("Audiobookshelf"), Some(Source::Audiobookshelf));
        assert_eq!(Source::parse("calibre-web"), Some(Source::CalibreWeb));
        assert_eq!(Source::parse("CalibreWeb"), Some(Source::CalibreWeb));
        assert_eq!(Source::parse("lazylibrarian"), Some(Source::LazyLibrarian));
        assert_eq!(Source::parse("plex"), None);
        for s in Source::ALL {
            assert_eq!(Source::parse(s.key()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap().as_str().map(Source::parse), Some(Some(s)));
        }
    }
}
