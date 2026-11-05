//! Readarr's metadata client (`BookInfoProxy` and `GoodreadsSearchProxy`): it searches Goodreads for books and
//! fetches authors, works and editions from the metadata server, then fills in what the library already has.
//!
//! - **Search** goes to Goodreads' `book/auto_complete` (`READARR_GOODREADS_URL`, default
//!   `https://www.goodreads.com`), as in Readarr. A search can also name what it wants: `author:1654`,
//!   `work:7000`, `edition:7000001`, `isbn:…` and `asin:…`.
//! - **Authors, works and editions** come from the metadata server: Readarr's `MetadataSource` setting, or, when
//!   it is empty, the public server of rreading-glasses ([`DEFAULT_METADATA_SOURCE`]), which answers like
//!   Readarr's own server did before it went away (docs/DECISIONS.md).
//!
//! Like Readarr, an author's answer is kept for 30 minutes and Goodreads' for 5 days, and the author made from it
//! is kept for 10 minutes (while it is used at least once a minute; at most 10 authors). Readarr hands out that
//! kept author's books and covers themselves, not copies, so what one request changes in them (which edition is
//! monitored, covers moved to Readarr's own addresses) is still changed in the next. The port keeps them shared
//! the same way ([`Shared`]), so its answers match Readarr's in that too.

use super::json::{double, extension, object};
use super::text::{clean_author_name, clean_spaces, to_last_first};
use super::{authors, books, Answer, Fail};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The metadata server used when Readarr's `MetadataSource` setting is empty: rreading-glasses' public one.
pub const DEFAULT_METADATA_SOURCE: &str = "https://api.bookinfo.pro";

/// Goodreads, where searches go unless `READARR_GOODREADS_URL` says otherwise.
pub const DEFAULT_GOODREADS: &str = "https://www.goodreads.com";

/// The browser Readarr says it is to Goodreads.
const GOODREADS_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/83.0.4103.116 Safari/537.36";

/// Something that several models hold and change together, as .NET objects are.
pub type Shared<T> = Arc<Mutex<T>>;

/// A new [`Shared`] value.
pub fn shared<T>(t: T) -> Shared<T> {
    Arc::new(Mutex::new(t))
}

/// The value inside a [`Shared`] (also after a panic elsewhere left it locked).
pub fn lock<T>(s: &Shared<T>) -> MutexGuard<'_, T> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// A book in a series (`SeriesBookLink`), as the metadata gives it.
#[derive(Debug, Clone)]
pub struct SeriesLink {
    /// the series' id at the metadata server
    pub foreign_series_id: String,
    /// the series' name
    pub title: Option<String>,
    /// the book's place in the series as written ("1", "2.5", "1-3")
    pub position: Option<String>,
    /// the book's place as a number, for sorting
    pub series_position: i64,
    /// whether the book is one of the series' main books
    pub primary: bool,
}

/// An edition (`Edition`) as the metadata gives it, with the library's id and monitoring once matched.
#[derive(Debug, Clone)]
pub struct Edition {
    /// the library's id, 0 when the library doesn't have it
    pub id: i64,
    /// the library's book id, 0 when the library doesn't have it
    pub book_id: i64,
    /// its id at the metadata server
    pub foreign_edition_id: String,
    /// its slug (the id)
    pub title_slug: String,
    /// its ISBN-13
    pub isbn13: Value,
    /// its Amazon id
    pub asin: Value,
    /// its title
    pub title: String,
    /// its language
    pub language: Value,
    /// its description
    pub overview: Value,
    /// its format ("Paperback", "Kindle Edition")
    pub format: Value,
    /// whether it is an e-book
    pub is_ebook: bool,
    /// what sets it apart from other editions
    pub disambiguation: Value,
    /// its publisher
    pub publisher: Value,
    /// its number of pages, 0 when unknown
    pub page_count: i64,
    /// when it came out (`2013-08-27T00:00:00Z`)
    pub release_date: Option<String>,
    /// how many people rated it
    pub votes: i64,
    /// its average rating
    pub value: f64,
    /// its covers (`[{url, coverType, extension, remoteUrl}]`), shared with the books that show them
    pub images: Shared<Vec<Value>>,
    /// its links (`[{url, name}]`)
    pub links: Vec<Value>,
    /// whether it is the edition the library wants
    pub monitored: bool,
    /// whether someone added it by hand
    pub manual_add: bool,
}

impl Edition {
    /// Its popularity (`Ratings.Popularity`): average rating × number of ratings.
    pub fn popularity(&self) -> f64 {
        self.value * self.votes as f64
    }
}

/// A book (a "work": `Book`) as the metadata gives it, with the library's fields once matched.
#[derive(Debug, Clone)]
pub struct Book {
    /// the library's id, 0 when the library doesn't have it
    pub id: i64,
    /// the library's id of its author's metadata
    pub author_metadata_id: i64,
    /// its id at the metadata server
    pub foreign_book_id: String,
    /// its slug (the id)
    pub title_slug: String,
    /// its title
    pub title: Option<String>,
    /// when it first came out
    pub release_date: Option<String>,
    /// its links (`[{url, name}]`)
    pub links: Vec<Value>,
    /// its genres (a list, or nothing)
    pub genres: Value,
    /// the ids of related books (a list, or nothing)
    pub related_books: Value,
    /// how many people rated its editions
    pub votes: i64,
    /// the average rating of its editions
    pub value: f64,
    /// its clean title, for matching
    pub clean_title: String,
    /// whether the library wants it
    pub monitored: bool,
    /// whether any edition will do
    pub any_edition_ok: bool,
    /// when the library last refreshed it (as stored)
    pub last_info_sync: Value,
    /// when the library last searched for it (as stored)
    pub last_search_time: Value,
    /// when it was added to the library (as stored; nothing for a book the library doesn't have)
    pub added: Value,
    /// how it was added (as stored)
    pub add_options: Value,
    /// its author's metadata, once known (`AuthorMetadata` columns)
    pub author_metadata: Option<Row>,
    /// its author as the library has it, or a new one (the author columns of [`authors::SELECT`])
    pub author: Row,
    /// its editions
    pub editions: Vec<Shared<Edition>>,
    /// the series it is in
    pub series_links: Vec<SeriesLink>,
}

impl Default for Book {
    /// A new book with nothing in it (`new Book()`), with a new author (`new Author()`).
    fn default() -> Book {
        Book {
            id: 0,
            author_metadata_id: 0,
            foreign_book_id: String::new(),
            title_slug: String::new(),
            title: None,
            release_date: None,
            links: Vec::new(),
            genres: json!([]),
            related_books: json!([]),
            votes: 0,
            value: 0.0,
            clean_title: String::new(),
            monitored: false,
            any_edition_ok: false,
            last_info_sync: Value::Null,
            last_search_time: Value::Null,
            added: Value::Null,
            add_options: Value::Null,
            author_metadata: None,
            author: new_author(&Row::new()),
            editions: Vec::new(),
            series_links: Vec::new(),
        }
    }
}

impl Book {
    /// Its monitored editions (Readarr expects exactly one).
    pub fn monitored_editions(&self) -> Vec<Shared<Edition>> {
        self.editions.iter().filter(|e| lock(e).monitored).cloned().collect()
    }
}

/// An author as the metadata server gives it (`Author` from `MapAuthor`): its metadata and its books.
#[derive(Debug)]
pub struct AuthorInfo {
    /// its metadata (`AuthorMetadata` columns)
    pub meta: Row,
    /// its clean name, for matching
    pub clean_name: String,
    /// its books, shared with every request that uses this author while it is kept
    pub books: Vec<Shared<Book>>,
}

/// What a search found (`SearchForNewEntity`): an author or a book.
pub enum Found {
    /// an author (the author columns of [`authors::SELECT`])
    Author(Row),
    /// a book
    Book(Shared<Book>),
}

/// Why fetching metadata went wrong, by the .NET exception Readarr throws, so that each step catches what
/// Readarr's catches.
#[derive(Debug)]
enum Failure {
    /// `AuthorNotFoundException`
    AuthorNotFound,
    /// `BookNotFoundException`
    BookNotFound,
    /// `EditionNotFoundException`
    EditionNotFound,
    /// `BookInfoException` (503 with its message)
    BookInfo(String),
    /// `HttpException`: an error status, with its name (`NotFound`)
    Http(String),
    /// any other exception: its .NET type and message
    Other(&'static str, String),
}

/// .NET's `NullReferenceException`.
fn null_reference() -> Failure {
    Failure::Other("System.NullReferenceException", "Object reference not set to an instance of an object.".into())
}

/// .NET's `InvalidOperationException` for `Single` over several matches.
fn more_than_one() -> Failure {
    Failure::Other("System.InvalidOperationException", "Sequence contains more than one matching element".into())
}

impl From<kit_server_core::Error> for Failure {
    fn from(e: kit_server_core::Error) -> Failure {
        Failure::Other("System.Data.SQLite.SQLiteException", e.detail)
    }
}

/// An answer from the metadata server or Goodreads.
struct Resp {
    /// the status
    status: u16,
    /// the `Location` header, if any
    location: Option<String>,
    /// the `Retry-After` header, if any
    retry_after: Option<String>,
    /// the body
    body: String,
}

/// An author kept by [`BookInfo`], with when it was made and last used.
struct Kept {
    /// the author
    author: Arc<AuthorInfo>,
    /// when it was made (kept 10 minutes at most)
    made: Instant,
    /// when it was last used (kept while used at least once a minute)
    used: Instant,
}

/// What [`BookInfo`] keeps between requests.
#[derive(Default)]
struct Memory {
    /// answers by address: status, body and until when they are kept
    answers: HashMap<String, (u16, String, Instant)>,
    /// authors by their id at the metadata server
    authors: HashMap<String, Kept>,
    /// cover addresses by the hash Readarr's cover proxy names them with, and until when
    covers: HashMap<String, (String, Instant)>,
}

/// The metadata client, shared by every request (it keeps answers and authors between them).
#[derive(Clone)]
pub struct BookInfo {
    /// the HTTP client that follows redirects
    follow: reqwest::Client,
    /// the HTTP client that doesn't (Readarr reads the redirects of `work/` and `book/`)
    manual: reqwest::Client,
    /// Goodreads' address
    goodreads: String,
    /// what is kept between requests
    kept: Arc<Mutex<Memory>>,
}

/// .NET's name of an HTTP status (`HttpStatusCode`), as Readarr writes it in messages.
fn status_name(status: u16) -> String {
    match status {
        400 => "BadRequest",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "NotFound",
        405 => "MethodNotAllowed",
        408 => "RequestTimeout",
        409 => "Conflict",
        410 => "Gone",
        415 => "UnsupportedMediaType",
        422 => "UnprocessableEntity",
        429 => "TooManyRequests",
        500 => "InternalServerError",
        501 => "NotImplemented",
        502 => "BadGateway",
        503 => "ServiceUnavailable",
        504 => "GatewayTimeout",
        _ => return status.to_string(),
    }
    .to_string()
}

/// Whether a status is a redirect Readarr follows (`HasHttpRedirect`).
fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// A time from the metadata server as Readarr sends it again (in UTC, ending with `Z`).
fn utc(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty()).map(|s| {
        let has_zone = s.ends_with('Z') || s.get(19..).is_some_and(|z| z.contains(['+', '-']));
        if has_zone || s.len() <= 10 {
            if s.len() == 10 {
                format!("{s}T00:00:00Z")
            } else {
                s
            }
        } else {
            format!("{s}Z")
        }
    })
}

/// A rating as .NET's `decimal` holds a `double`: rounded to 15 significant digits.
fn decimal(x: f64) -> f64 {
    format!("{x:.14e}").parse().unwrap_or(x)
}

/// Ratings as Readarr sends them: `{votes, value, popularity}`.
pub fn ratings_json(votes: i64, value: f64) -> Value {
    json!({ "votes": votes, "value": double(value), "popularity": double(value * votes as f64) })
}

// The metadata server's answers (`BookInfoResource`), read the way System.Text.Json reads them: the names exactly
// as written, a missing field as its default, a wrong type as an error.

/// An author (`AuthorResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct AuthorRes {
    /// its id
    foreign_id: i64,
    /// its name
    name: Option<String>,
    /// its description
    description: Option<String>,
    /// its picture
    image_url: Option<String>,
    /// its page
    url: Option<String>,
    /// how many people rated it
    rating_count: i64,
    /// its average rating
    average_rating: f64,
    /// its books
    works: Option<Vec<WorkRes>>,
    /// its series
    series: Option<Vec<SeriesRes>>,
}

/// A book (`WorkResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct WorkRes {
    /// its id
    foreign_id: i64,
    /// its title
    title: Option<String>,
    /// its page
    url: Option<String>,
    /// when it first came out
    release_date: Option<String>,
    /// its genres
    genres: Option<Vec<String>>,
    /// the ids of related books
    related_works: Option<Vec<i64>>,
    /// its editions
    books: Option<Vec<EditionRes>>,
    /// its series
    series: Option<Vec<SeriesRes>>,
    /// its authors
    authors: Option<Vec<AuthorRes>>,
}

/// An edition (`BookResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct EditionRes {
    /// its id
    foreign_id: i64,
    /// its Amazon id
    asin: Option<String>,
    /// its description
    description: Option<String>,
    /// its ISBN-13
    isbn13: Option<String>,
    /// its title
    title: Option<String>,
    /// its language
    language: Option<String>,
    /// its format
    format: Option<String>,
    /// what sets it apart
    edition_information: Option<String>,
    /// its publisher
    publisher: Option<String>,
    /// its cover
    image_url: Option<String>,
    /// whether it is an e-book
    is_ebook: bool,
    /// its number of pages
    num_pages: Option<i64>,
    /// how many people rated it
    rating_count: i64,
    /// its average rating
    average_rating: f64,
    /// its page
    url: Option<String>,
    /// when it came out
    release_date: Option<String>,
    /// who wrote it, the main author first
    contributors: Option<Vec<ContributorRes>>,
}

/// Someone who worked on an edition (`ContributorResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct ContributorRes {
    /// the author's id
    foreign_id: i64,
}

/// A series (`SeriesResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct SeriesRes {
    /// its id
    foreign_id: i64,
    /// its name
    title: Option<String>,
    /// the books in it
    link_items: Option<Vec<LinkRes>>,
}

/// A book in a series (`SeriesWorkLinkResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct LinkRes {
    /// the book's id
    foreign_work_id: i64,
    /// its place as written
    position_in_series: Option<String>,
    /// its place as a number
    series_position: i64,
    /// whether it is a main book of the series
    primary: bool,
}

/// The answer to a bulk request (`BulkBookResource`).
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct BulkRes {
    /// the books
    works: Option<Vec<WorkRes>>,
    /// their series
    series: Option<Vec<SeriesRes>>,
    /// their authors
    authors: Option<Vec<AuthorRes>>,
}

/// Reads an answer like System.Text.Json (`JsonException` when it doesn't fit).
fn parse<T: for<'a> Deserialize<'a>>(body: &str) -> Result<T, Failure> {
    serde_json::from_str(body).map_err(|e| Failure::Other("System.Text.Json.JsonException", e.to_string()))
}

/// A Goodreads search result (`SearchJsonResource`): the parts Readarr uses.
struct Hit {
    /// the edition's id
    book_id: i64,
    /// the book's id
    work_id: i64,
    /// the author's id, when there is an author
    author_id: Option<i64>,
}

/// A whole number from Goodreads, read like Newtonsoft (a number, or text that is one).
fn int(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0)
}

/// An author's metadata (`MapAuthorMetadata`), in the `AuthorMetadata` columns.
fn map_author_metadata(r: &AuthorRes) -> Result<Row, Failure> {
    let name = clean_spaces(r.name.as_deref().ok_or_else(null_reference)?);
    let last_first = to_last_first(&name);
    let mut images = Vec::new();
    if let Some(url) = r.image_url.as_deref().filter(|u| !u.trim().is_empty()) {
        images.push(json!({ "url": url, "coverType": "poster", "extension": extension(url) }));
    }
    let mut links = Vec::new();
    if let Some(url) = r.url.as_deref().filter(|u| !u.trim().is_empty()) {
        links.push(json!({ "url": url, "name": "Goodreads" }));
    }
    let mut m = Row::new();
    m.insert("ForeignAuthorId".into(), json!(r.foreign_id.to_string()));
    m.insert("TitleSlug".into(), json!(r.foreign_id.to_string()));
    m.insert("Name".into(), json!(name));
    m.insert("Overview".into(), json!(r.description));
    m.insert("Disambiguation".into(), Value::Null);
    m.insert("Status".into(), json!(0));
    m.insert("Images".into(), json!(Value::Array(images).to_string()));
    m.insert("Links".into(), json!(Value::Array(links).to_string()));
    m.insert("Genres".into(), json!("[]"));
    m.insert("Ratings".into(), json!(json!({ "votes": r.rating_count, "value": double(decimal(r.average_rating)) }).to_string()));
    m.insert("Aliases".into(), json!("[]"));
    m.insert("SortName".into(), json!(name.to_lowercase()));
    m.insert("SortNameLastFirst".into(), json!(last_first.to_lowercase()));
    m.insert("NameLastFirst".into(), json!(last_first));
    Ok(m)
}

/// A new author for some metadata (`new Author { CleanName, Metadata }`), in the columns of [`authors::SELECT`].
pub fn new_author(meta: &Row) -> Row {
    let mut a = meta.clone();
    let name = meta.get("Name").and_then(Value::as_str).unwrap_or_default();
    for (k, v) in [
        ("Id", json!(0)),
        ("CleanName", json!(clean_author_name(name))),
        ("Path", Value::Null),
        ("Monitored", json!(0)),
        ("LastInfoSync", Value::Null),
        ("QualityProfileId", json!(0)),
        ("Tags", json!("[]")),
        ("Added", Value::Null),
        ("AddOptions", Value::Null),
        ("MetadataProfileId", json!(0)),
        ("AuthorMetadataId", json!(0)),
        ("MonitorNewItems", json!(0)),
    ] {
        a.insert(k.into(), v);
    }
    for k in ["Images", "Links", "Genres", "Aliases"] {
        a.entry(k.to_string()).or_insert(json!("[]"));
    }
    a.entry("Ratings".to_string()).or_insert(json!(r#"{"votes":0,"value":0}"#));
    a.entry("Status".to_string()).or_insert(json!(0));
    a
}

/// An edition (`MapEdition`).
fn map_edition(r: &EditionRes) -> Result<Edition, Failure> {
    let mut images = Vec::new();
    if let Some(url) = r.image_url.as_deref().filter(|u| !u.trim().is_empty()) {
        images.push(json!({ "url": url, "coverType": "cover", "extension": extension(url) }));
    }
    Ok(Edition {
        id: 0,
        book_id: 0,
        foreign_edition_id: r.foreign_id.to_string(),
        title_slug: r.foreign_id.to_string(),
        isbn13: json!(r.isbn13),
        asin: json!(r.asin),
        title: clean_spaces(r.title.as_deref().ok_or_else(null_reference)?),
        language: json!(r.language),
        overview: json!(r.description),
        format: json!(r.format),
        is_ebook: r.is_ebook,
        disambiguation: json!(r.edition_information),
        publisher: json!(r.publisher),
        page_count: r.num_pages.unwrap_or(0),
        release_date: utc(r.release_date.clone()),
        votes: r.rating_count,
        value: decimal(r.average_rating),
        images: shared(images),
        links: vec![object([("url", json!(r.url)), ("name", json!("Goodreads Book"))])],
        monitored: false,
        manual_add: false,
    })
}

/// Whether a time is on the 1st or in January, which usually means only the year is known.
fn year_only(date: &str) -> bool {
    date.get(5..7) == Some("01") || date.get(8..10) == Some("01")
}

/// A book with its editions (`MapBook`): the most popular edition is monitored, and a book without a release
/// date takes its earliest edition's (preferring dates that aren't just a year).
fn map_book(r: &WorkRes) -> Result<Book, Failure> {
    let mut book = Book {
        foreign_book_id: r.foreign_id.to_string(),
        title: r.title.clone(),
        title_slug: r.foreign_id.to_string(),
        clean_title: clean_author_name(r.title.as_deref().unwrap_or_default()),
        release_date: utc(r.release_date.clone()),
        genres: json!(r.genres),
        related_books: json!(r.related_works),
        ..Book::default()
    };
    book.links.push(object([("url", json!(r.url)), ("name", json!("Goodreads Editions"))]));
    let editions: Vec<Edition> = match &r.books {
        Some(list) => list.iter().map(map_edition).collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let mut editions = editions;
    let mut best: Option<usize> = None;
    for (i, e) in editions.iter().enumerate() {
        if best.is_none_or(|b| e.popularity() > editions[b].popularity()) {
            best = Some(i);
        }
    }
    if let Some(b) = best {
        editions[b].monitored = true;
        if book.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
            book.title = Some(editions[b].title.clone());
        }
    }
    if book.release_date.is_none() {
        let not_year_only = editions.iter().filter_map(|e| e.release_date.clone()).filter(|d| !year_only(d)).min();
        book.release_date = not_year_only.or_else(|| editions.iter().filter_map(|e| e.release_date.clone()).min());
    }
    book.any_edition_ok = true;
    let votes: i64 = editions.iter().map(|e| e.votes).sum();
    if votes > 0 {
        book.votes = votes;
        book.value = editions.iter().map(|e| e.votes as f64 * e.value).sum::<f64>() / votes as f64;
    }
    book.editions = editions.into_iter().map(shared).collect();
    Ok(book)
}

/// The author of a book (`GetAuthorId`): the first contributor of its most popular edition that has any, 0 when
/// none has.
fn get_author_id(r: &WorkRes) -> Result<i64, Failure> {
    let books = r
        .books
        .as_ref()
        .ok_or_else(|| Failure::Other("System.ArgumentNullException", "Value cannot be null. (Parameter 'source')".into()))?;
    let mut order: Vec<&EditionRes> = books.iter().collect();
    order.sort_by(|a, b| (b.rating_count as f64 * b.average_rating).total_cmp(&(a.rating_count as f64 * a.average_rating)));
    for e in order {
        let c = e.contributors.as_ref().ok_or_else(null_reference)?;
        if let Some(first) = c.first() {
            return Ok(first.foreign_id);
        }
    }
    Ok(0)
}

/// Links the books to their series (`MapSeriesLinks`): only series with books in them, and only books that are
/// here.
fn map_series_links(series: &[SeriesRes], books: &[Shared<Book>]) {
    let ids: Vec<String> = books.iter().map(|b| lock(b).foreign_book_id.clone()).collect();
    for b in books {
        lock(b).series_links = Vec::new();
    }
    for s in series.iter().filter(|s| s.link_items.as_ref().is_some_and(|l| !l.is_empty())) {
        for l in s.link_items.iter().flatten().filter(|l| l.foreign_work_id != 0) {
            let Some(i) = ids.iter().position(|id| *id == l.foreign_work_id.to_string()) else { continue };
            lock(&books[i]).series_links.push(SeriesLink {
                foreign_series_id: s.foreign_id.to_string(),
                title: s.title.clone(),
                position: l.position_in_series.clone(),
                series_position: l.series_position,
                primary: l.primary,
            });
        }
    }
}

/// An author with its books (`MapAuthor`): only the books it is the main author of.
fn map_author(r: AuthorRes) -> Result<AuthorInfo, Failure> {
    let meta = map_author_metadata(&r)?;
    let mut books = Vec::new();
    for w in r.works.iter().flatten() {
        if w.foreign_id > 0 && get_author_id(w)? == r.foreign_id {
            let mut b = map_book(w)?;
            b.author_metadata = Some(meta.clone());
            books.push(shared(b));
        }
    }
    map_series_links(r.series.as_deref().unwrap_or_default(), &books);
    let clean_name = clean_author_name(&meta.s("Name"));
    Ok(AuthorInfo { meta, clean_name, books })
}

/// The editions of a library book (`GetEditionsByBook`), by their id at the metadata server.
fn library_editions(db: &SqliteDb, book_id: i64) -> kit_server_core::Result<HashMap<String, Row>> {
    Ok(db
        .query("SELECT * FROM Editions WHERE BookId = ? ORDER BY Id", vec![json!(book_id)])?
        .into_iter()
        .map(|e| (e.s("ForeignEditionId"), e))
        .collect())
}

/// Fills in what the library has (`AddDbIds`): the book's own fields when the library has it, which of its
/// editions is monitored, and its author (the library's, or a new one from `authors`).
fn add_db_ids(db: &SqliteDb, author_id: &str, book: &Shared<Book>, metas: &HashMap<String, Row>) -> Result<(), Failure> {
    let mut b = lock(book);
    if let Some(row) = books::rows(db, "b.ForeignBookId = ?", vec![json!(b.foreign_book_id)])?.into_iter().next() {
        b.id = row.i("Id");
        b.author_metadata_id = row.i("AuthorMetadataId");
        b.monitored = row.i("Monitored") != 0;
        b.any_edition_ok = row.i("AnyEditionOk") != 0;
        b.last_info_sync = row.get("LastInfoSync").cloned().unwrap_or(Value::Null);
        b.last_search_time = row.get("LastSearchTime").cloned().unwrap_or(Value::Null);
        b.added = row.get("Added").cloned().unwrap_or(Value::Null);
        b.add_options = row.get("AddOptions").cloned().unwrap_or(Value::Null);
        let library = library_editions(db, b.id)?;
        for e in &b.editions {
            let mut e = lock(e);
            e.monitored = false;
            if let Some(le) = library.get(&e.foreign_edition_id) {
                e.id = le.i("Id");
                e.book_id = le.i("BookId");
                e.monitored = le.i("Monitored") != 0;
                e.manual_add = le.i("ManualAdd") != 0;
            }
        }
        if !b.editions.is_empty() && !b.editions.iter().any(|e| lock(e).monitored) {
            let popularity: Vec<f64> = b.editions.iter().map(|e| lock(e).popularity()).collect();
            let mut best = 0;
            for (i, p) in popularity.iter().enumerate() {
                if *p > popularity[best] {
                    best = i;
                }
            }
            lock(&b.editions[best]).monitored = true;
        }
    }
    let author = match authors::rows(db, "m.ForeignAuthorId = ?", vec![json!(author_id)])?.into_iter().next() {
        Some(a) => a,
        None => {
            let meta = metas.get(author_id).ok_or_else(|| {
                Failure::BookInfo(format!(
                    "Expected author metadata for id [{author_id}] in book data [{}][{}]",
                    b.foreign_book_id,
                    b.title.clone().unwrap_or_else(|| "[NULL]".into())
                ))
            })?;
            new_author(meta)
        }
    };
    b.author_metadata_id = author.i("AuthorMetadataId");
    b.author_metadata = Some(author.clone());
    b.author = author;
    Ok(())
}

impl BookInfo {
    /// The client, with Goodreads at `goodreads` (nothing or blank: [`DEFAULT_GOODREADS`]).
    pub fn new(goodreads: Option<String>) -> BookInfo {
        let build = |redirect: reqwest::redirect::Policy| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(100))
                .user_agent(format!("Readarr/{} (Folio {})", super::READARR_VERSION, crate::VERSION))
                .redirect(redirect)
                .build()
                .unwrap_or_default()
        };
        BookInfo {
            follow: build(reqwest::redirect::Policy::limited(10)),
            manual: build(reqwest::redirect::Policy::none()),
            goodreads: goodreads
                .filter(|g| !g.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_GOODREADS.into())
                .trim_end_matches('/')
                .to_string(),
            kept: Arc::new(Mutex::new(Memory::default())),
        }
    }

    /// What is kept between requests.
    fn kept(&self) -> MutexGuard<'_, Memory> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The address of a metadata server route: Readarr's `MetadataSource` setting, or the default server.
    fn route(db: &SqliteDb, route: &str) -> Result<String, Failure> {
        let source = super::disk::config(db, "MetadataSource")?.filter(|s| !s.trim().is_empty());
        let base = match source {
            Some(s) => s.strip_suffix('/').map(str::to_string).unwrap_or(s),
            None => DEFAULT_METADATA_SOURCE.to_string(),
        };
        Ok(format!("{base}/{route}"))
    }

    /// Sends a request and reads the answer (a failed connection is .NET's `WebException`).
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Resp, Failure> {
        let res = req.send().await.map_err(|e| Failure::Other("System.Net.WebException", e.to_string()))?;
        let header = |n: &str| res.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
        let (status, location, retry_after) = (res.status().as_u16(), header("location"), header("retry-after"));
        let body = res.text().await.map_err(|e| Failure::Other("System.Net.WebException", e.to_string()))?;
        Ok(Resp { status, location, retry_after, body })
    }

    /// A GET, from what is kept when `use_cache` and it is still fresh; answers that aren't errors are kept for
    /// `ttl` (`CachedHttpResponseService`).
    async fn cached_get(&self, url: &str, use_cache: bool, ttl: Duration, agent: Option<&str>) -> Result<Resp, Failure> {
        if use_cache {
            if let Some((status, body, until)) = self.kept().answers.get(url).cloned() {
                if until > Instant::now() {
                    return Ok(Resp { status, location: None, retry_after: None, body });
                }
            }
        }
        let mut req = self.follow.get(url);
        if let Some(a) = agent {
            req = req.header("user-agent", a);
        }
        let resp = self.send(req).await?;
        if resp.status < 400 {
            self.kept().answers.insert(url.to_string(), (resp.status, resp.body.clone(), Instant::now() + ttl));
        }
        Ok(resp)
    }

    /// Waits as long as a 429 answer asks (`Retry-After`, else 5 seconds).
    async fn wait_until_retry(resp: &Resp) {
        let secs = resp.retry_after.as_deref().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(5);
        tokio::time::sleep(Duration::from_secs(secs)).await;
    }

    /// A GET that doesn't follow redirects, asked again after a 429.
    async fn get_manual(&self, url: &str) -> Result<Resp, Failure> {
        loop {
            let resp = self.send(self.manual.get(url)).await?;
            if resp.status == 429 {
                Self::wait_until_retry(&resp).await;
                continue;
            }
            return Ok(resp);
        }
    }

    /// Goodreads' search results for `query` (`GoodreadsSearchProxy.Search`); nothing when Goodreads answers
    /// `null`.
    async fn goodreads_search(&self, query: &str) -> Result<Option<Vec<Hit>>, Failure> {
        let q: String = url_encode(query);
        let url = format!("{}/book/auto_complete?format=json&q={q}", self.goodreads);
        let resp = self.cached_get(&url, true, Duration::from_secs(5 * 24 * 3600), Some(GOODREADS_AGENT)).await?;
        if resp.status >= 400 {
            return Err(Failure::Http(status_name(resp.status)));
        }
        let v: Value =
            serde_json::from_str(&resp.body).map_err(|e| Failure::Other("Newtonsoft.Json.JsonReaderException", e.to_string()))?;
        let Value::Array(list) = v else {
            return if v.is_null() {
                Ok(None)
            } else {
                Err(Failure::Other("Newtonsoft.Json.JsonSerializationException", "not a list".into()))
            };
        };
        Ok(Some(
            list.iter()
                .map(|h| Hit {
                    book_id: int(&h["bookId"]),
                    work_id: int(&h["workId"]),
                    author_id: h.get("author").filter(|a| a.is_object()).map(|a| int(&a["id"])),
                })
                .collect(),
        ))
    }

    /// An author from the metadata server, made again only when it isn't kept (`PollAuthor`).
    async fn poll_author(&self, db: &SqliteDb, id: &str) -> Result<Arc<AuthorInfo>, Failure> {
        let now = Instant::now();
        {
            let mut kept = self.kept();
            if let Some(k) = kept.authors.get_mut(id) {
                if now.duration_since(k.made) < Duration::from_secs(600) && now.duration_since(k.used) < Duration::from_secs(60) {
                    k.used = now;
                    return Ok(k.author.clone());
                }
            }
        }
        let author = Arc::new(self.poll_author_uncached(db, id).await?);
        let mut kept = self.kept();
        kept.authors
            .retain(|_, k| now.duration_since(k.made) < Duration::from_secs(600) && now.duration_since(k.used) < Duration::from_secs(60));
        if kept.authors.len() >= 10 {
            if let Some(oldest) = kept.authors.iter().min_by_key(|(_, k)| k.used).map(|(k, _)| k.clone()) {
                kept.authors.remove(&oldest);
            }
        }
        kept.authors.insert(id.to_string(), Kept { author: author.clone(), made: now, used: now });
        Ok(author)
    }

    /// An author from the metadata server (`PollAuthorUncached`): asked again (without what is kept) while the
    /// server is still putting its books together.
    async fn poll_author_uncached(&self, db: &SqliteDb, id: &str) -> Result<AuthorInfo, Failure> {
        let url = Self::route(db, &format!("author/{id}"))?;
        let mut use_cache = true;
        for _ in 0..60 {
            let resp = self.cached_get(&url, use_cache, Duration::from_secs(30 * 60), None).await?;
            if resp.status >= 400 {
                match resp.status {
                    429 => {
                        Self::wait_until_retry(&resp).await;
                        continue;
                    }
                    404 => return Err(Failure::AuthorNotFound),
                    400 => return Err(Failure::Other("NzbDrone.Core.Exceptions.BadRequestException", id.to_string())),
                    _ => return Err(Failure::BookInfo("Unexpected error fetching author data".into())),
                }
            }
            let res: AuthorRes = parse(&resp.body)?;
            if res.works.is_some() {
                return map_author(res);
            }
            use_cache = false;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(Failure::BookInfo(format!("Failed to get works for {id}")))
    }

    /// A book from the metadata server (`PollBook`): its author's id, the book and its authors' metadata. The
    /// server may send the book's author instead (a redirect), whose book it then is.
    async fn poll_book(&self, db: &SqliteDb, id: &str) -> Result<(String, Shared<Book>, Vec<Row>), Failure> {
        let url = Self::route(db, &format!("work/{id}"))?;
        let mut res: Option<WorkRes> = None;
        for _ in 0..60 {
            let resp = self.get_manual(&url).await?;
            if resp.status == 404 {
                return Err(Failure::BookNotFound);
            }
            if is_redirect(resp.status) {
                let (new_id, kind) = redirect_target(resp.location.as_deref())?;
                if kind != "author" {
                    return Err(Failure::Other("System.NotImplementedException", format!("Unexpected response from {url}")));
                }
                let author = self.poll_author(db, &new_id).await?;
                let found: Vec<&Shared<Book>> = author.books.iter().filter(|b| lock(b).foreign_book_id == id).collect();
                if found.len() > 1 {
                    return Err(more_than_one());
                }
                let book = found.first().copied().cloned().ok_or(Failure::BookNotFound)?;
                return Ok((author.meta.s("ForeignAuthorId"), book, vec![author.meta.clone()]));
            }
            if resp.status >= 400 {
                return Err(if resp.status == 400 {
                    Failure::Other("NzbDrone.Core.Exceptions.BadRequestException", id.to_string())
                } else {
                    Failure::BookInfo("Unexpected response fetching book data".into())
                });
            }
            let r: WorkRes = parse(&resp.body)?;
            if r.books.is_some() {
                res = Some(r);
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        let r = res
            .filter(|r| r.authors.as_ref().is_some_and(|a| !a.is_empty()))
            .ok_or_else(|| Failure::BookInfo(format!("Failed to get books for {id}")))?;
        let book = shared(map_book(&r)?);
        let author_id = get_author_id(&r)?.to_string();
        let metas = r.authors.iter().flatten().map(map_author_metadata).collect::<Result<Vec<_>, _>>()?;
        map_series_links(r.series.as_deref().unwrap_or_default(), std::slice::from_ref(&book));
        Ok((author_id, book, metas))
    }

    /// The book of an edition (`GetEditionInfo`), with all its editions or (`all` false) only that one,
    /// monitored. The server answers with a redirect to the edition's book or author.
    async fn edition_info(&self, db: &SqliteDb, id: i64, all: bool) -> Result<Shared<Book>, Failure> {
        let url = Self::route(db, &format!("book/{id}"))?;
        let resp = self.get_manual(&url).await?;
        if resp.status == 404 {
            return Err(Failure::EditionNotFound);
        }
        if !is_redirect(resp.status) {
            return Err(Failure::BookInfo(format!("Unexpected response from {url}")));
        }
        let (new_id, kind) = redirect_target(resp.location.as_deref())?;
        let edition_id = id.to_string();
        let has_edition = |b: &Shared<Book>| lock(b).editions.iter().any(|e| lock(e).foreign_edition_id == edition_id);
        let (book, metas) = match kind.as_str() {
            "author" => {
                let author = self.poll_author(db, &new_id).await?;
                (author.books.iter().find(|b| has_edition(b)).cloned(), vec![author.meta.clone()])
            }
            "work" => {
                let (_, book, metas) = self.poll_book(db, &new_id).await?;
                (Some(book), metas)
            }
            _ => return Err(Failure::Other("System.NotImplementedException", format!("Unexpected response from {url}"))),
        };
        let book = book.filter(|b| has_edition(b)).ok_or(Failure::EditionNotFound)?;
        let book = if all {
            book
        } else {
            let src = lock(&book);
            let meta = src.author_metadata.clone().ok_or_else(null_reference)?;
            let matching: Vec<Shared<Edition>> =
                src.editions.iter().filter(|e| lock(e).foreign_edition_id == edition_id).cloned().collect();
            if matching.len() > 1 {
                return Err(more_than_one());
            }
            for e in &matching {
                lock(e).monitored = true;
            }
            shared(Book {
                foreign_book_id: src.foreign_book_id.clone(),
                title_slug: src.title_slug.clone(),
                title: src.title.clone(),
                release_date: src.release_date.clone(),
                links: src.links.clone(),
                genres: src.genres.clone(),
                related_books: src.related_books.clone(),
                votes: src.votes,
                value: src.value,
                clean_title: src.clean_title.clone(),
                author_metadata: Some(meta),
                series_links: src.series_links.clone(),
                editions: matching,
                ..Book::default()
            })
        };
        let author_id = lock(&book).author_metadata.as_ref().map(|m| m.s("ForeignAuthorId")).ok_or_else(null_reference)?;
        let metas: HashMap<String, Row> = metas.into_iter().map(|m| (m.s("ForeignAuthorId"), m)).collect();
        add_db_ids(db, &author_id, &book, &metas)?;
        Ok(book)
    }

    /// Several editions' books at once (`MapSearchResult`), in the order of `ids`.
    async fn bulk(&self, db: &SqliteDb, ids: &[i64]) -> Result<Vec<Shared<Book>>, Failure> {
        let url = Self::route(db, "book/bulk")?;
        let resp = loop {
            let resp = self.send(self.follow.post(&url).json(&ids)).await?;
            if resp.status == 429 {
                Self::wait_until_retry(&resp).await;
                continue;
            }
            break resp;
        };
        if resp.status >= 400 {
            return Err(Failure::Http(status_name(resp.status)));
        }
        let Some(r): Option<BulkRes> = parse(&resp.body)? else { return Ok(Vec::new()) };
        let null = || Failure::Other("System.ArgumentNullException", "Value cannot be null. (Parameter 'source')".into());
        let metas: HashMap<String, Row> = r
            .authors
            .as_ref()
            .ok_or_else(null)?
            .iter()
            .map(|a| map_author_metadata(a).map(|m| (m.s("ForeignAuthorId"), m)))
            .collect::<Result<_, _>>()?;
        let series = r.series.as_ref().ok_or_else(null)?;
        let mut found = Vec::new();
        for w in r.works.as_ref().ok_or_else(null)? {
            let book = shared(map_book(w)?);
            let mut order: Vec<&EditionRes> = w.books.iter().flatten().collect();
            order.sort_by(|a, b| (b.average_rating * b.rating_count as f64).total_cmp(&(a.average_rating * a.rating_count as f64)));
            let none = || Failure::Other("System.InvalidOperationException", "Sequence contains no elements".into());
            let top = order.first().ok_or_else(none)?;
            let author_id = top.contributors.as_ref().ok_or_else(null_reference)?.first().ok_or_else(none)?.foreign_id.to_string();
            add_db_ids(db, &author_id, &book, &metas)?;
            found.push(book);
        }
        map_series_links(series, &found);
        let ids: Vec<String> = ids.iter().map(i64::to_string).collect();
        let mut keyed = Vec::new();
        for b in found {
            let first = lock(&b).editions.first().map(|e| lock(e).foreign_edition_id.clone());
            let first = first.ok_or_else(|| Failure::Other("System.InvalidOperationException", "Sequence contains no elements".into()))?;
            let at = ids.iter().position(|i| *i == first).map_or(-1, |p| p as i64);
            keyed.push((at, b));
        }
        keyed.sort_by_key(|(at, _)| *at);
        Ok(keyed.into_iter().map(|(_, b)| b).collect())
    }

    /// An author's books (`SearchByGoodreadsAuthorId`); none when there is no such author.
    async fn by_author_id(&self, db: &SqliteDb, id: i64) -> Result<Vec<Shared<Book>>, Failure> {
        let id = id.to_string();
        let found = async {
            let author = self.poll_author(db, &id).await?;
            let metas = HashMap::from([(id.clone(), author.meta.clone())]);
            for b in &author.books {
                add_db_ids(db, &id, b, &metas)?;
            }
            Ok::<_, Failure>(author.books.clone())
        };
        match found.await {
            Err(Failure::AuthorNotFound | Failure::BookInfo(_)) => Ok(Vec::new()),
            r => r,
        }
    }

    /// A book (`SearchByGoodreadsWorkId`); none when there is no such book.
    async fn by_work_id(&self, db: &SqliteDb, id: i64) -> Result<Vec<Shared<Book>>, Failure> {
        let found = async {
            let (author_id, book, metas) = self.poll_book(db, &id.to_string()).await?;
            let metas: HashMap<String, Row> = metas.into_iter().map(|m| (m.s("ForeignAuthorId"), m)).collect();
            add_db_ids(db, &author_id, &book, &metas)?;
            Ok::<_, Failure>(vec![book])
        };
        match found.await {
            Err(Failure::BookNotFound | Failure::BookInfo(_)) => Ok(Vec::new()),
            r => r,
        }
    }

    /// An edition's book (`SearchByGoodreadsBookId`); none when there is no such edition.
    async fn by_book_id(&self, db: &SqliteDb, id: i64, all: bool) -> Result<Vec<Shared<Book>>, Failure> {
        match self.edition_info(db, id, all).await {
            Ok(b) => Ok(vec![b]),
            Err(Failure::AuthorNotFound | Failure::BookNotFound | Failure::EditionNotFound | Failure::BookInfo(_)) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// A Goodreads search, then the books it found from the metadata server (`Search`). With `all`, by their
    /// authors (all editions); otherwise by their editions (the one found, monitored).
    async fn search(&self, db: &SqliteDb, query: &str, all: bool) -> Result<Vec<Shared<Book>>, Failure> {
        let hits = match self.goodreads_search(query).await {
            Ok(h) => h,
            Err(_) => return Ok(Vec::new()),
        };
        let hits =
            hits.ok_or_else(|| Failure::Other("System.ArgumentNullException", "Value cannot be null. (Parameter 'source')".into()))?;
        if all {
            let mut by_author: Vec<(i64, Vec<String>)> = Vec::new();
            for h in &hits {
                let a = h.author_id.ok_or_else(null_reference)?;
                match by_author.iter_mut().find(|(id, _)| *id == a) {
                    Some((_, ids)) => ids.push(h.work_id.to_string()),
                    None => by_author.push((a, vec![h.work_id.to_string()])),
                }
            }
            let mut found = Vec::new();
            for (author, ids) in &by_author {
                for b in self.by_author_id(db, *author).await? {
                    if ids.contains(&lock(&b).foreign_book_id) {
                        found.push(b);
                    }
                }
            }
            let have: Vec<String> = found.iter().map(|b| lock(b).foreign_book_id.clone()).collect();
            let mut missing: Vec<i64> = Vec::new();
            for h in &hits {
                if !have.contains(&h.work_id.to_string()) && !missing.contains(&h.work_id) {
                    missing.push(h.work_id);
                }
            }
            for id in missing {
                found.extend(self.by_work_id(db, id).await?);
            }
            Ok(found)
        } else {
            let ids: Vec<i64> = hits.iter().map(|h| h.book_id).collect();
            match ids.len() {
                0 => Ok(Vec::new()),
                1 => self.by_book_id(db, ids[0], false).await,
                _ => match self.bulk(db, &ids).await {
                    Ok(b) => Ok(b),
                    Err(Failure::Http(code)) => Err(Failure::BookInfo(format!(
                        "Search for '{query}' failed. Unable to communicate with ReadarrAPI, returning status code: {code}."
                    ))),
                    Err(_) => Ok(Vec::new()),
                },
            }
        }
    }

    /// The books for a search term (`SearchForNewBook`), with all their editions (`all`) or only the one found.
    /// No term is Readarr's `NullReferenceException`; a failure is its `GoodreadsException` (503).
    pub async fn search_for_new_book(&self, db: &SqliteDb, title: Option<&str>, all: bool) -> Answer<Vec<Shared<Book>>> {
        let Some(title) = title else {
            return Err(Fail::exception(500, "System.NullReferenceException", "Object reference not set to an instance of an object."));
        };
        let found = async {
            let lower = title.to_lowercase();
            let mut q = lower.trim().to_string();
            let split: Vec<&str> = lower.split(':').collect();
            if split.len() == 2 && ["author", "work", "edition", "isbn", "asin"].contains(&split[0]) {
                let prefix = split[0];
                let slug = split[1].trim();
                if slug.is_empty() || slug.chars().any(char::is_whitespace) {
                    return Ok(Vec::new());
                }
                if matches!(prefix, "author" | "work" | "edition") {
                    let Ok(id) = slug.parse::<i32>() else { return Ok(Vec::new()) };
                    let id = i64::from(id);
                    return match prefix {
                        "author" => self.by_author_id(db, id).await,
                        "work" => self.by_work_id(db, id).await,
                        _ => self.by_book_id(db, id, all).await,
                    };
                }
                q = slug.to_string();
            }
            self.search(db, &q, all).await
        };
        let kind = "NzbDrone.Core.MetadataSource.Goodreads.GoodreadsException";
        match found.await {
            Ok(b) => Ok(b),
            Err(Failure::BookInfo(m)) => Err(Fail::exception(503, "NzbDrone.Core.MetadataSource.BookInfo.BookInfoException", m)),
            Err(Failure::Http(_)) => {
                Err(Fail::exception(503, kind, format!("Search for '{title}' failed. Unable to communicate with Goodreads.")))
            }
            Err(e) => {
                let why = match &e {
                    Failure::Other(kind, message) => format!("{kind}: {message}"),
                    other => format!("{other:?}"),
                };
                tracing::warn!("Readarr lookup for {title:?} failed: {why}");
                Err(Fail::exception(503, kind, format!("Search for '{title}' failed. Invalid response received from Goodreads.")))
            }
        }
    }

    /// The authors for a search term (`SearchForNewAuthor`): the authors of the books found, each once.
    pub async fn search_for_new_author(&self, db: &SqliteDb, title: Option<&str>) -> Answer<Vec<Row>> {
        let mut out: Vec<Row> = Vec::new();
        for b in self.search_for_new_book(db, title, true).await? {
            let a = lock(&b).author.clone();
            if !out.iter().any(|o| o.s("ForeignAuthorId") == a.s("ForeignAuthorId")) {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// The authors and books for a search term (`SearchForNewEntity`): each book's author (once) before it.
    pub async fn search_for_new_entity(&self, db: &SqliteDb, title: Option<&str>) -> Answer<Vec<Found>> {
        let mut out = Vec::new();
        let mut seen: Vec<Row> = Vec::new();
        for b in self.search_for_new_book(db, title, false).await? {
            let a = lock(&b).author.clone();
            if !seen.contains(&a) {
                seen.push(a.clone());
                out.push(Found::Author(a));
            }
            out.push(Found::Book(b));
        }
        Ok(out)
    }

    /// Covers as Readarr sends them (`ConvertToLocalUrls`), changed in place: for an item the library has
    /// (`id` not 0) the address of its own copy, otherwise an address of the cover proxy; the address before
    /// becomes `remoteUrl`. `entity` is `author` or `book`.
    pub fn convert_covers(&self, covers: &mut Vec<Value>, entity: &str, id: i64) {
        if id != 0 {
            *covers = super::json::local_covers(std::mem::take(covers), entity, id);
            return;
        }
        for c in covers.iter_mut() {
            let remote = c.get("url").cloned().unwrap_or(Value::Null);
            c["remoteUrl"] = remote.clone();
            c["url"] = match remote.as_str().filter(|u| !u.trim().is_empty()) {
                Some(u) => json!(self.register_cover(u)),
                None => Value::Null,
            };
            if let Some(o) = c.as_object_mut() {
                o.retain(|_, v| !v.is_null());
            }
        }
    }

    /// Registers a cover with the cover proxy for a day (`MediaCoverProxy.RegisterUrl`): its proxy address.
    fn register_cover(&self, url: &str) -> String {
        let hash = hex::encode(Sha256::digest(url.as_bytes()));
        let mut kept = self.kept();
        let now = Instant::now();
        kept.covers.retain(|_, (_, until)| *until > now);
        kept.covers.insert(hash.clone(), (url.to_string(), now + Duration::from_secs(24 * 3600)));
        let file = url.rsplit('/').next().unwrap_or_default();
        format!("/MediaCoverProxy/{hash}/{file}")
    }

    /// The cover a proxy hash stands for, while it is registered.
    pub fn proxied_cover(&self, hash: &str) -> Option<String> {
        self.kept().covers.get(hash).filter(|(_, until)| *until > Instant::now()).map(|(u, _)| u.clone())
    }
}

/// The id and kind (`author`, `work`) a redirect points to: the last two parts of its address.
fn redirect_target(location: Option<&str>) -> Result<(String, String), Failure> {
    let location = location.ok_or_else(null_reference)?;
    let mut parts = location.rsplit('/');
    let id = parts.next().unwrap_or_default().to_string();
    let kind = parts.next().ok_or_else(|| {
        Failure::Other(
            "System.ArgumentOutOfRangeException",
            "Index was out of range. Must be non-negative and less than the size of the collection. (Parameter 'index')".into(),
        )
    })?;
    Ok((id, kind.to_string()))
}

/// A query value as .NET escapes it in an address (`Uri.EscapeDataString`).
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_maps_like_readarr() {
        let w: WorkRes = serde_json::from_str(
            r#"{"ForeignId": 7000, "Title": "Mort", "Url": "u", "ReleaseDate": null, "Books": [
              {"ForeignId": 1, "Title": "Mort", "RatingCount": 200, "AverageRating": 4.25, "ReleaseDate": "2013-04-01T00:00:00Z",
               "ImageUrl": "https://x/1.png", "Contributors": [{"ForeignId": 9}]},
              {"ForeignId": 2, "Title": "Mort  (2)", "RatingCount": 100, "AverageRating": 4.0, "ReleaseDate": "1987-11-12T00:00:00Z"}]}"#,
        )
        .unwrap();
        let b = map_book(&w).unwrap();
        assert_eq!(b.release_date.as_deref(), Some("1987-11-12T00:00:00Z"));
        assert!(lock(&b.editions[0]).monitored && !lock(&b.editions[1]).monitored);
        assert_eq!(lock(&b.editions[1]).title, "Mort (2)");
        assert_eq!(lock(&lock(&b.editions[0]).images)[0]["extension"], json!(".png"));
        assert_eq!(get_author_id(&w).unwrap(), 9);
        assert_eq!(ratings_json(800000, decimal(4.1))["popularity"], json!(3279999.9999999995));
        assert_eq!(utc(Some("2013-04-01T00:00:00".into())).as_deref(), Some("2013-04-01T00:00:00Z"));
        assert_eq!(status_name(404), "NotFound");
        assert_eq!(url_encode("small gods"), "small%20gods");
        assert_eq!(redirect_target(Some("/author/1654")).unwrap(), ("1654".into(), "author".into()));

        let info = BookInfo::new(None);
        let mut covers = vec![json!({"url": "https://images.example/7000001.png", "coverType": "cover", "extension": ".png"})];
        info.convert_covers(&mut covers, "book", 0);
        assert_eq!(
            covers[0]["url"],
            json!("/MediaCoverProxy/acab39b1e6c4f875a18e9f9b6432f51bffe75c87d1ca7bfaabd190ec9a5120fb/7000001.png")
        );
        assert_eq!(
            info.proxied_cover("acab39b1e6c4f875a18e9f9b6432f51bffe75c87d1ca7bfaabd190ec9a5120fb").as_deref(),
            Some("https://images.example/7000001.png")
        );
    }
}
