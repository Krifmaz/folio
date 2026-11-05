//! The plan: everything read from one source, as Folio will bring it in. Showing it unchanged is the dry run.
//!
//! Books, wanted entries, progress and bookmarks point at each other by **source id**: the source's key and its
//! own id for the book (`abs:<library item id>`, `calibre:<book id>`). Progress names the person by their user name
//! in the source, so the server can pair it with a Folio account.

use crate::Source;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// An audiobook or an ebook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// something to listen to
    Audiobook,
    /// something to read
    Ebook,
}

/// A book's place in a series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesPlace {
    /// the series' name
    pub name: String,
    /// its place as the source writes it (`"1"`, `"2.5"`); empty when unknown
    pub position: String,
}

/// A book the source has files for: one audiobook or one ebook (a Calibre book with an EPUB and a PDF is one
/// ebook with two files).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Book {
    /// the source's key and its id for the book (`abs:…`, `calibre:42`)
    pub source_id: String,
    /// audiobook or ebook
    pub format: Format,
    /// the title, without the subtitle
    pub title: String,
    /// the subtitle; empty when there is none
    pub subtitle: String,
    /// the authors, first author first
    pub authors: Vec<String>,
    /// the narrators (audiobooks)
    pub narrators: Vec<String>,
    /// the series it is in, with its place
    pub series: Vec<SeriesPlace>,
    /// the Audible ASIN as the source has it; empty when unknown
    pub asin: String,
    /// the ISBN as the source has it (10 or 13 digits); empty when unknown
    pub isbn: String,
    /// the language as the source writes it (`"English"`, `"eng"`); empty when unknown
    pub language: String,
    /// the year it came out
    pub year: Option<i32>,
    /// how long it plays, in seconds (audiobooks)
    pub duration_secs: Option<f64>,
    /// whether it is abridged, when the source says
    pub abridged: Option<bool>,
    /// the book's folder, when it has one of its own
    pub folder: Option<PathBuf>,
    /// its files, in playing or preference order, as paths the source saw them at
    pub files: Vec<PathBuf>,
    /// the cover image, when the source keeps one
    pub cover: Option<PathBuf>,
}

/// A book the source wanted but didn't have yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wanted {
    /// the source's key and its id for the book (`readarr:12`)
    pub source_id: String,
    /// audiobook or ebook
    pub format: Format,
    /// the title
    pub title: String,
    /// the authors, first author first
    pub authors: Vec<String>,
    /// the narrators, when the source knows them
    pub narrators: Vec<String>,
    /// the Audible ASIN of the exact edition; empty when unknown
    pub asin: String,
    /// the ISBN of the exact edition; empty when unknown
    pub isbn: String,
    /// the language as the source writes it; empty when unknown
    pub language: String,
}

/// How far a person got in a book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// the person's user name in the source
    pub user: String,
    /// the book's source id
    pub book: String,
    /// audiobook or ebook
    pub format: Format,
    /// where they are in an audiobook, in milliseconds
    pub position_ms: Option<i64>,
    /// how much of it they have had, 0 to 1
    pub fraction: Option<f64>,
    /// where they are in an ebook as an EPUB CFI; empty when unknown
    pub cfi: String,
    /// whether they finished it
    pub finished: bool,
    /// when it last changed, in seconds since 1970
    pub updated_at: Option<f64>,
}

/// A place a person marked in an audiobook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    /// the person's user name in the source
    pub user: String,
    /// the book's source id
    pub book: String,
    /// the place, in milliseconds
    pub position_ms: i64,
    /// the name they gave it
    pub title: String,
}

/// Something read but left out of the plan, with the reason in plain words.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    /// the source's id for it, or its title when it has none
    pub item: String,
    /// why, in a plain sentence
    pub reason: String,
}

/// How much is in a plan, for the dry run's headline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// audiobooks with files
    pub audiobooks: usize,
    /// ebooks with files
    pub ebooks: usize,
    /// wanted audiobooks
    pub wanted_audiobooks: usize,
    /// wanted ebooks
    pub wanted_ebooks: usize,
    /// progress entries
    pub progress: usize,
    /// bookmarks
    pub bookmarks: usize,
    /// people with progress or bookmarks
    pub users: usize,
    /// things left out
    pub skipped: usize,
}

/// Everything read from one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// where it was read from
    pub source: Source,
    /// the database file read (never changed)
    pub path: PathBuf,
    /// books with files
    pub books: Vec<Book>,
    /// books wanted but not had
    pub wanted: Vec<Wanted>,
    /// how far people got
    pub progress: Vec<Progress>,
    /// places people marked
    pub bookmarks: Vec<Bookmark>,
    /// what was left out and why
    pub skipped: Vec<Skipped>,
}

impl Plan {
    /// An empty plan for `source` read from `path`.
    pub fn new(source: Source, path: &Path) -> Plan {
        Plan { source, path: path.to_path_buf(), books: vec![], wanted: vec![], progress: vec![], bookmarks: vec![], skipped: vec![] }
    }

    /// The source id of the source's own id `id` (`calibre:42`).
    pub fn id(&self, id: impl std::fmt::Display) -> String {
        format!("{}:{id}", self.source.key())
    }

    /// Notes that `item` was left out because of `reason`.
    pub fn skip(&mut self, item: impl Into<String>, reason: impl Into<String>) {
        self.skipped.push(Skipped { item: item.into(), reason: reason.into() });
    }

    /// How much is in the plan.
    pub fn summary(&self) -> Summary {
        let mut users: Vec<&str> =
            self.progress.iter().map(|p| p.user.as_str()).chain(self.bookmarks.iter().map(|b| b.user.as_str())).collect();
        users.sort_unstable();
        users.dedup();
        Summary {
            audiobooks: self.books.iter().filter(|b| b.format == Format::Audiobook).count(),
            ebooks: self.books.iter().filter(|b| b.format == Format::Ebook).count(),
            wanted_audiobooks: self.wanted.iter().filter(|w| w.format == Format::Audiobook).count(),
            wanted_ebooks: self.wanted.iter().filter(|w| w.format == Format::Ebook).count(),
            progress: self.progress.len(),
            bookmarks: self.bookmarks.len(),
            users: users.len(),
            skipped: self.skipped.len(),
        }
    }
}

/// Splits a list of names the way the sources write them: a JSON array (`["A","B"]`, or of objects with a `name`),
/// or text separated by commas, `&` or ` and `; blanks dropped.
pub fn names(raw: &str) -> Vec<String> {
    let t = raw.trim();
    if t.starts_with('[') {
        if let Ok(v) = serde_json::from_str::<Vec<serde_json::Value>>(t) {
            return v
                .iter()
                .filter_map(|x| x.as_str().map(str::to_string).or_else(|| x["name"].as_str().map(str::to_string)))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    if t.starts_with('"') {
        if let Ok(s) = serde_json::from_str::<String>(t) {
            return names(&s);
        }
    }
    t.replace(" & ", ",").replace(" and ", ",").split([',', ';']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The year at the start of a date the sources write (`"2020"`, `"2020-11-17 00:00:00+00:00"`); `None` for
/// Calibre's "unknown" date (year 101) and anything else.
pub fn year(date: &str) -> Option<i32> {
    let y: i32 = date.trim().get(..4)?.parse().ok()?;
    (1000..3000).contains(&y).then_some(y)
}

/// Seconds since 1970 from a date and time the sources write (`"2024-03-01 10:20:30.123 +00:00"`,
/// `"2024-03-01T10:20:30Z"`, `"2024-03-01 10:20:30.123456"`, taken as UTC) or from a number of milliseconds;
/// `None` when it is neither.
pub fn epoch(raw: &str) -> Option<f64> {
    let t = raw.trim();
    if let Ok(n) = t.parse::<f64>() {
        // milliseconds (ABS, Readarr) or seconds
        return Some(if n > 1e11 { n / 1000.0 } else { n });
    }
    let num = |a: usize, b: usize| t.get(a..b).and_then(|s| s.parse::<i64>().ok());
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = if t.len() >= 19 { (num(11, 13)?, num(14, 16)?, num(17, 19)?) } else { (0, 0, 0) };
    let rest = t.get(19..).unwrap_or("");
    let frac: f64 = rest
        .strip_prefix('.')
        .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>())
        .map_or(0.0, |digits| format!("0.{digits}").parse().unwrap_or(0.0));
    // a "+01:00" or "-05:00" offset after the time
    let offset = rest.rfind(['+', '-']).and_then(|i| {
        let o = rest[i + 1..].replace(':', "");
        let (oh, om) = (o.get(0..2)?.parse::<i64>().ok()?, o.get(2..4).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0));
        Some(if &rest[i..=i] == "-" { -(oh * 3600 + om * 60) } else { oh * 3600 + om * 60 })
    });
    // days from 1970-01-01 to the date (Howard Hinnant's days_from_civil)
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + h * 3600 + mi * 60 + s - offset.unwrap_or(0)) as f64 + frac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_become_seconds_since_1970() {
        assert_eq!(epoch("1970-01-01 00:00:00"), Some(0.0));
        assert_eq!(epoch("2024-03-01T10:20:30Z"), Some(1709288430.0));
        assert_eq!(epoch("2024-03-01 10:20:30.500 +00:00"), Some(1709288430.5));
        assert_eq!(epoch("2024-03-01 11:20:30 +01:00"), Some(1709288430.0));
        assert_eq!(epoch("2024-03-01 05:20:30-05:00"), Some(1709288430.0));
        assert_eq!(epoch("1709288430000"), Some(1709288430.0));
        assert_eq!(epoch("2024-03-01"), Some(1709251200.0));
        assert_eq!(epoch("soon"), None);
    }

    #[test]
    fn names_are_split_from_json_and_text() {
        assert_eq!(names(r#"["Ray Porter", " "]"#), vec!["Ray Porter"]);
        assert_eq!(names(r#"[{"name":"Andy Weir"}]"#), vec!["Andy Weir"]);
        assert_eq!(names(r#""Ray Porter""#), vec!["Ray Porter"]);
        assert_eq!(names("Terry Pratchett & Neil Gaiman"), vec!["Terry Pratchett", "Neil Gaiman"]);
        assert_eq!(names("A, B and C"), vec!["A", "B", "C"]);
        assert!(names("").is_empty());
    }

    #[test]
    fn years_come_from_the_start_of_a_date() {
        assert_eq!(year("2021-05-04 00:00:00+00:00"), Some(2021));
        assert_eq!(year("2021"), Some(2021));
        assert_eq!(year("0101-01-01 00:00:00+00:00"), None);
        assert_eq!(year(""), None);
    }
}
