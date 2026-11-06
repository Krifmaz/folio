//! Readarr's metadata profiles (`MetadataProfileService.FilterBooks`): which of an author's books and editions
//! the library takes when the author is refreshed. A profile can leave out books with too few ratings, without a
//! release date, that are parts or sets of others, that are secondary in their series or whose titles have
//! ignored terms, and editions in other languages, without an ISBN or ASIN, or with ignored terms. Books and
//! editions the library already has by hand or with files always stay.
//!
//! Languages are compared the way calibre does (`CanonicalizeLanguage`), with calibre's tables in `languages/`.

use super::bookinfo::{lock, AuthorInfo, Book, Edition, Shared};
use super::json::now_param;
use anyhow::{anyhow, bail, Result};
use fancy_regex::Regex as FancyRegex;
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

/// The `MinPopularity` of the "None" profile, which takes no books at all.
pub const NONE_PROFILE_MIN_POPULARITY: f64 = 1e10;

/// Two-letter language codes to three-letter ones (calibre's `2to3.json`).
static TWO_TO_THREE: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("languages/2to3.json")).expect("a valid language table"));

/// The known three-letter codes (calibre's `by3.json`, by its keys).
static BY_THREE: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("languages/by3.json")).expect("a valid language table"));

/// Language names to three-letter codes (calibre's `name_map.json`).
static NAME_MAP: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| serde_json::from_str(include_str!("languages/name_map.json")).expect("a valid language table"));

/// ISO 639-2 bibliographic codes to the terminology ones (`BtoTmap`).
const B_TO_T: &[(&str, &str)] = &[
    ("alb", "sqi"),
    ("arm", "hye"),
    ("baq", "eus"),
    ("bur", "mya"),
    ("chi", "zho"),
    ("cze", "ces"),
    ("dut", "nld"),
    ("fre", "fra"),
    ("geo", "kat"),
    ("ger", "deu"),
    ("gre", "ell"),
    ("ice", "isl"),
    ("mac", "mkd"),
    ("mao", "mri"),
    ("may", "msa"),
    ("per", "fas"),
    ("rum", "ron"),
    ("slo", "slk"),
    ("tib", "bod"),
    ("wel", "cym"),
];

/// "1 of 3", "1/3" or "1-3" in a title (`PartOrSetRegex`; .NET names each alternative's first number `from`).
static PART_OR_SET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\d+) of (?:\d+)|(\d+)\s?/\s?(?:\d+)|(\d+)\s?-\s?(?:\d+)").expect("a valid pattern"));

/// A term written as a Perl regex (`/pattern/modifiers`, `PerlRegexFactory`).
static PERL_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/(?<pattern>.*)/(?<modifiers>[a-z]*)").expect("a valid pattern"));

/// A metadata profile (`MetadataProfile`).
#[derive(Debug, Clone)]
pub struct Profile {
    /// the fewest ratings × average rating a book needs (unless it comes out later)
    pub min_popularity: f64,
    /// whether books without a release date are left out
    pub skip_missing_date: bool,
    /// whether editions without an ISBN-13 and ASIN are left out
    pub skip_missing_isbn: bool,
    /// whether books that are parts or sets of others are left out
    pub skip_parts_and_sets: bool,
    /// whether books that are only secondary in their series are left out
    pub skip_series_secondary: bool,
    /// the languages editions may be in, separated by commas ("eng, null"); blank allows all
    pub allowed_languages: String,
    /// the pages a book needs (when its editions say how many they have)
    pub min_pages: i64,
    /// terms whose books and editions are left out
    pub ignored: Vec<String>,
}

impl Profile {
    /// The profile `id`, or Readarr's `ModelNotFoundException`.
    pub fn get(db: &SqliteDb, id: i64) -> Result<Profile> {
        let r = db
            .one("SELECT * FROM MetadataProfiles WHERE Id = ?", vec![json!(id)])?
            .ok_or_else(|| anyhow!("NzbDrone.Core.Datastore.ModelNotFoundException: MetadataProfile with ID {id} does not exist"))?;
        let ignored = match super::json::doc(r.get("Ignored")) {
            Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            _ => Vec::new(),
        };
        Ok(Profile {
            min_popularity: r.get("MinPopularity").and_then(Value::as_f64).unwrap_or(0.0),
            skip_missing_date: r.i("SkipMissingDate") != 0,
            skip_missing_isbn: r.i("SkipMissingIsbn") != 0,
            skip_parts_and_sets: r.i("SkipPartsAndSets") != 0,
            skip_series_secondary: r.i("SkipSeriesSecondary") != 0,
            allowed_languages: r.s("AllowedLanguages"),
            min_pages: r.i("MinPages"),
            ignored,
        })
    }
}

/// A language as calibre names it (`CanonicalizeLanguage`): its three-letter code, from a two- or three-letter
/// code, a code with a region ("en-GB") or a name ("English"); nothing when it isn't one.
pub fn canonicalize_language(raw: &str) -> Option<String> {
    let raw = raw.to_lowercase();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let raw = raw.replace('_', "-");
    let raw = raw.split('-').next().unwrap_or_default().trim().to_string();
    if raw.is_empty() {
        return None;
    }
    match raw.chars().count() {
        2 => {
            if let Some(l) = TWO_TO_THREE.get(&raw) {
                return Some(l.clone());
            }
        }
        3 => {
            if BY_THREE.contains_key(&raw) {
                return Some(raw);
            }
            if let Some((_, t)) = B_TO_T.iter().find(|(b, _)| *b == raw) {
                return Some((*t).to_string());
            }
        }
        _ => {}
    }
    NAME_MAP.get(&raw).cloned()
}

/// Whether `term` is found in `value` (`TermMatcherService.IsMatch`): a `/regex/` term as that regex, any other
/// term anywhere in it, ignoring case.
fn term_matches(term: &str, value: &str) -> Result<bool> {
    let Some(c) = PERL_REGEX.captures(term) else {
        return Ok(value.to_lowercase().contains(&term.to_lowercase()));
    };
    // .NET's options as inline flags; `n` (only named groups capture) changes nothing for a match.
    let mut flags = String::new();
    for m in c["modifiers"].chars() {
        match m {
            'i' | 'm' | 's' | 'x' => flags.push(m),
            'n' => {}
            _ => bail!("System.ArgumentException: Unknown or unsupported perl regex modifier: {m}"),
        }
    }
    let pattern = if flags.is_empty() { c["pattern"].to_string() } else { format!("(?{flags}){}", &c["pattern"]) };
    let re = FancyRegex::new(&pattern).map_err(|e| anyhow!("System.ArgumentException: {e}"))?;
    re.is_match(value).map_err(|e| anyhow!("System.Text.RegularExpressions.RegexMatchTimeoutException: {e}"))
}

/// Whether a title has one of a profile's ignored terms (`MatchesTerms`): each entry may list several, separated
/// by commas.
fn has_ignored_terms(value: &str, profile: &Profile) -> Result<bool> {
    for terms in &profile.ignored {
        if terms.trim().is_empty() || value.trim().is_empty() {
            continue;
        }
        for t in terms.split(',').filter(|t| !t.is_empty()) {
            if term_matches(t, value)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Now in the form the metadata's release dates have (`2026-11-05T18:38:30`), to compare them.
fn now_iso() -> String {
    now_param().replacen(' ', "T", 1).chars().take(19).collect()
}

/// Whether a release date (`2030-01-01T00:00:00Z`) is later than now.
fn is_future(date: Option<&str>) -> bool {
    date.is_some_and(|d| d.chars().take(19).collect::<String>() > now_iso())
}

/// The current year.
fn this_year() -> i64 {
    now_iso()[..4].parse().unwrap_or(0)
}

/// A book's place in its series as the filter sees it.
struct Link {
    /// the place as written
    position: Option<String>,
    /// whether it is a main book of the series
    primary: bool,
}

/// Whether .NET's `double.TryParse` reads the text as a number.
fn parses_as_double(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && (t.replace(',', "").parse::<f64>().is_ok() || matches!(t.to_lowercase().as_str(), "nan" | "infinity" | "-infinity"))
}

/// Whether a book is a part or a set of others (`IsPartOrSet`): its series places are all ranges ("1-3"), its
/// title (or an edition's) is "A / B" of titles that are there already, or it has "1 of 3", "1/3" or "1-3" that
/// isn't a span of years.
fn is_part_or_set(book: &Book, links: Option<&Vec<Link>>, titles: &HashSet<Option<String>>) -> Result<bool> {
    if let Some(links) = links {
        if links.iter().any(|l| l.position.as_deref().is_some_and(|p| !p.trim().is_empty()))
            && !links.iter().any(|l| l.position.as_deref().is_some_and(parses_as_double))
        {
            return Ok(true);
        }
    }
    let title = book
        .title
        .clone()
        .ok_or_else(|| anyhow!("System.NullReferenceException: Object reference not set to an instance of an object."))?;
    let mut book_titles = vec![title.clone()];
    book_titles.extend(book.editions.iter().map(|e| lock(e).title.clone()));
    for t in &book_titles {
        let split: Vec<String> = t.split('/').map(|x| x.trim().to_string()).collect();
        if split.len() > 1 && split.iter().all(|x| titles.contains(&Some(x.clone()))) {
            return Ok(true);
        }
    }
    if let Some(c) = PART_OR_SET.captures(&title) {
        let from = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)).map(|m| m.as_str()).unwrap_or_default();
        let from: i32 =
            from.parse().map_err(|_| anyhow!("System.OverflowException: Value was either too large or too small for an Int32."))?;
        return Ok(from <= 1800 || i64::from(from) > this_year());
    }
    Ok(false)
}

/// Removes the items that `allowed` refuses, except those the library has by hand or with files (`local`, by their
/// id at the metadata server), keeping the order (`FilterByPredicate`).
fn filter<T>(items: &mut Vec<T>, id: impl Fn(&T) -> String, local: &HashSet<String>, allowed: impl Fn(&T) -> Result<bool>) -> Result<()> {
    let mut keep = Vec::with_capacity(items.len());
    for item in items.iter() {
        keep.push(allowed(item)? || local.contains(&id(item)));
    }
    let mut i = 0;
    items.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    Ok(())
}

/// What the library has of an author: its books' editions by book, and the books and editions with files.
struct Local {
    /// the ids at the metadata server of books added by hand or with files
    books: HashSet<String>,
    /// the library's editions of each book, by the book's id at the metadata server
    editions: HashMap<String, Vec<Row>>,
    /// the ids at the metadata server of editions with files
    edition_files: HashSet<String>,
}

/// What the library has of the author with this id at the metadata server.
fn local(db: &SqliteDb, foreign_author_id: &str) -> Result<Local> {
    let author = db.one(
        "SELECT a.Id, a.AuthorMetadataId FROM Authors a JOIN AuthorMetadata m ON m.Id = a.AuthorMetadataId WHERE m.ForeignAuthorId = ?",
        vec![json!(foreign_author_id)],
    )?;
    let mut out = Local { books: HashSet::new(), editions: HashMap::new(), edition_files: HashSet::new() };
    let Some(author) = author else { return Ok(out) };
    for b in db.query(
        "SELECT Id, ForeignBookId, AddOptions FROM Books WHERE AuthorMetadataId = ? ORDER BY Id",
        vec![json!(author.i("AuthorMetadataId"))],
    )? {
        if super::books::add_options(b.get("AddOptions"))["addType"] == json!("manual") {
            out.books.insert(b.s("ForeignBookId"));
        }
        let editions = db.query("SELECT * FROM Editions WHERE BookId = ? ORDER BY Id", vec![json!(b.i("Id"))])?;
        out.editions.insert(b.s("ForeignBookId"), editions);
    }
    let files = db.query(
        "SELECT e.ForeignEditionId, b.ForeignBookId FROM BookFiles f JOIN Editions e ON e.Id = f.EditionId
         JOIN Books b ON b.Id = e.BookId JOIN Authors a ON a.AuthorMetadataId = b.AuthorMetadataId WHERE a.Id = ? ORDER BY f.Id",
        vec![json!(author.i("Id"))],
    )?;
    for f in files {
        out.books.insert(f.s("ForeignBookId"));
        out.edition_files.insert(f.s("ForeignEditionId"));
    }
    Ok(out)
}

/// The editions of a book the profile takes (`FilterEditions`).
fn filter_editions(editions: &mut Vec<Shared<Edition>>, local_editions: &[Row], local: &Local, profile: &Profile) -> Result<()> {
    let allowed: HashSet<Option<String>> = if profile.allowed_languages.trim().is_empty() {
        HashSet::new()
    } else {
        profile.allowed_languages.trim_matches(',').split(',').map(canonicalize_language).collect()
    };
    let mut keep: HashSet<String> = local_editions.iter().filter(|e| e.i("ManualAdd") != 0).map(|e| e.s("ForeignEditionId")).collect();
    keep.extend(local.edition_files.iter().cloned());
    let id = |e: &Shared<Edition>| lock(e).foreign_edition_id.clone();
    filter(editions, id, &keep, |e| {
        let lang = lock(e).language.as_str().and_then(canonicalize_language);
        Ok(allowed.is_empty() || allowed.contains(&lang))
    })?;
    filter(editions, id, &keep, |e| {
        let e = lock(e);
        let filled = |v: &Value| v.as_str().is_some_and(|s| !s.trim().is_empty());
        Ok(!profile.skip_missing_isbn || filled(&e.isbn13) || filled(&e.asin))
    })?;
    filter(editions, id, &keep, |e| Ok(!has_ignored_terms(&lock(e).title, profile)?))?;
    Ok(())
}

/// The books of an author the profile `profile_id` takes (`FilterBooks`), in the metadata's order. Each book's
/// editions are cut down to the ones the profile takes, in place (Readarr changes the kept author too).
pub fn filter_books(db: &SqliteDb, info: &AuthorInfo, profile_id: i64) -> Result<Vec<Shared<Book>>> {
    let mut series_links: Vec<(Shared<Book>, Vec<Link>)> = Vec::new();
    for s in &info.series {
        let items = lock(&s.items).clone();
        let items = items.ok_or_else(|| anyhow!("System.NullReferenceException: Object reference not set to an instance of an object."))?;
        for item in &items {
            let link = Link { position: item.position.clone(), primary: item.primary };
            match series_links.iter_mut().find(|(b, _)| Arc::ptr_eq(b, &item.book)) {
                Some((_, l)) => l.push(link),
                None => series_links.push((item.book.clone(), vec![link])),
            }
        }
    }
    let links_of = |b: &Shared<Book>| series_links.iter().find(|(k, _)| Arc::ptr_eq(k, b)).map(|(_, l)| l);
    let local = local(db, &info.meta.s("ForeignAuthorId"))?;
    let profile = Profile::get(db, profile_id)?;
    let mut books: Vec<Shared<Book>> = Vec::new();
    for b in &info.books {
        if !books.iter().any(|k| Arc::ptr_eq(k, b)) {
            books.push(b.clone());
        }
    }
    let titles: HashSet<Option<String>> = info.books.iter().map(|b| lock(b).title.clone()).collect();
    let id = |b: &Shared<Book>| lock(b).foreign_book_id.clone();
    let keep = &local.books;
    filter(&mut books, id, keep, |b| {
        let b = lock(b);
        if profile.min_popularity == NONE_PROFILE_MIN_POPULARITY {
            return Ok(false);
        }
        Ok(b.value * b.votes as f64 >= profile.min_popularity || is_future(b.release_date.as_deref()))
    })?;
    filter(&mut books, id, keep, |b| Ok(!profile.skip_missing_date || lock(b).release_date.is_some()))?;
    filter(&mut books, id, keep, |b| Ok(!profile.skip_parts_and_sets || !is_part_or_set(&lock(b), links_of(b), &titles)?))?;
    filter(&mut books, id, keep, |b| Ok(!profile.skip_series_secondary || links_of(b).is_none_or(|l| l.iter().any(|x| x.primary))))?;
    filter(&mut books, id, keep, |b| {
        let title = lock(b).title.clone().unwrap_or_default();
        Ok(!has_ignored_terms(&title, &profile)?)
    })?;
    for b in &books {
        let mut b = lock(b);
        let local_editions = local.editions.get(&b.foreign_book_id).cloned().unwrap_or_default();
        filter_editions(&mut b.editions, &local_editions, &local, &profile)?;
    }
    filter(&mut books, id, keep, |b| {
        let b = lock(b);
        let pages: Vec<i64> = b.editions.iter().map(|e| lock(e).page_count).collect();
        Ok(pages.iter().any(|p| *p > profile.min_pages) || pages.iter().all(|p| *p == 0))
    })?;
    filter(&mut books, id, keep, |b| Ok(!lock(b).editions.is_empty()))?;
    Ok(books)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_read_like_calibre() {
        assert_eq!(canonicalize_language("eng").as_deref(), Some("eng"));
        assert_eq!(canonicalize_language("en-GB").as_deref(), Some("eng"));
        assert_eq!(canonicalize_language(" English ").as_deref(), Some("eng"));
        assert_eq!(canonicalize_language("fre").as_deref(), Some("fra"));
        assert_eq!(canonicalize_language("null"), None);
        assert_eq!(canonicalize_language(" "), None);
    }

    #[test]
    fn terms_match_like_readarr() {
        assert!(term_matches("box", "The Boxed Set").unwrap());
        assert!(term_matches("/^the/i", "The Boxed Set").unwrap());
        assert!(!term_matches("/^the/", "The Boxed Set").unwrap());
        assert!(term_matches("/x/q", "x").is_err());
    }

    #[test]
    fn parts_and_sets_are_found_like_readarr() {
        let titles = HashSet::from([Some("A".to_string()), Some("B".to_string())]);
        let book = |t: &str| Book { title: Some(t.into()), ..Book::default() };
        assert!(is_part_or_set(&book("A / B"), None, &titles).unwrap());
        assert!(is_part_or_set(&book("Stories 1-3"), None, &titles).unwrap());
        assert!(!is_part_or_set(&book("Poems 1914-1918"), None, &titles).unwrap());
        assert!(!is_part_or_set(&book("Mort"), None, &titles).unwrap());
        let range = vec![Link { position: Some("1-3".into()), primary: false }];
        assert!(is_part_or_set(&book("Mort"), Some(&range), &titles).unwrap());
        let numbered = vec![Link { position: Some("2".into()), primary: true }];
        assert!(!is_part_or_set(&book("Mort"), Some(&numbered), &titles).unwrap());
    }
}
