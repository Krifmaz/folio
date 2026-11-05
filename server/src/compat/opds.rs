//! The OPDS 1.2 catalogue: e-readers and reading apps (KOReader, Moon+ Reader, Thorium, Panels, Librera) browse
//! the library and download books from it. Written from the OPDS 1.2 specification (opds-spec.org) and Atom
//! (RFC 4287); people sign in with their Folio username and password (HTTP Basic), or an app token.
//!
//! ```text
//! GET /opds                          the start: new books, audiobooks, ebooks, authors, series, search
//! GET /opds/new|audiobooks|ebooks    books, newest first, 50 a page (?page=, from 1)
//! GET /opds/authors, /opds/authors/{id}
//! GET /opds/series, /opds/series/{id} (in series order)
//! GET /opds/search?q=                books whose title, author, narrator or series has every word
//! GET /opds/search.xml               the OpenSearch description of the above
//! GET /opds/books/{id}/files/{file}  one of the book's files (ranges allowed)
//! GET /opds/books/{id}/cover         the cover picture
//! ```

use super::model::Item;
use super::shelf::{self, Who};
use super::xml::{esc, Xml};
use crate::library::text;
use crate::AppState;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use kit_server_core::{err, Result};
use serde::Deserialize;

/// Books on one page of a feed.
pub const PAGE: usize = 50;

/// The media type of a navigation feed.
const NAV: &str = "application/atom+xml;profile=opds-catalog;kind=navigation";
/// The media type of an acquisition feed (books you can download).
const ACQ: &str = "application/atom+xml;profile=opds-catalog;kind=acquisition";

/// The catalogue's routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/opds", get(start))
        .route("/opds/", get(start))
        .route("/opds/new", get(new_books))
        .route("/opds/audiobooks", get(audiobooks))
        .route("/opds/ebooks", get(ebooks))
        .route("/opds/authors", get(authors))
        .route("/opds/authors/{id}", get(author))
        .route("/opds/series", get(series_list))
        .route("/opds/series/{id}", get(series))
        .route("/opds/search", get(search))
        .route("/opds/search.xml", get(opensearch))
        .route("/opds/books/{id}/files/{file}", get(file))
        .route("/opds/books/{id}/files/{file}/{name}", get(file_named))
        .route("/opds/books/{id}/cover", get(cover))
}

/// `?page=` of a feed, from 1.
#[derive(Debug, Deserialize, Default)]
pub struct Paging {
    /// the page, from 1
    #[serde(default)]
    pub page: Option<usize>,
    /// the words searched for
    #[serde(default)]
    pub q: String,
}

/// A feed as an answer, with its media type.
fn feed(kind: &'static str, body: String) -> Response {
    ([(header::CONTENT_TYPE, HeaderValue::from_static(kind))], body).into_response()
}

/// The start of a feed: the XML header, the feed's id, title, update time and its self and start links.
fn head(x: &mut Xml, id: &str, title: &str, path: &str, kind: &str) {
    x.raw("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.raw("<feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:dc=\"http://purl.org/dc/terms/\" ");
    x.raw("xmlns:opds=\"http://opds-spec.org/2010/catalog\" xmlns:opensearch=\"http://a9.com/-/spec/opensearch/1.1/\">\n");
    x.el("id", &format!("urn:folio:{id}"));
    x.el("title", title);
    x.el("updated", &rfc3339(super::now()));
    x.raw("<author><name>Folio</name></author>\n");
    x.link("self", path, kind, "");
    x.link("start", "/opds", NAV, "");
    x.link("search", "/opds/search.xml", "application/opensearchdescription+xml", "");
}

/// A time in seconds since 1970 written as RFC 3339 (`2026-11-05T01:47:19Z`).
pub fn rfc3339(secs: f64) -> String {
    let secs = secs.max(0.0) as i64;
    let (days, rest) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The calendar day `days` after 1970-01-01 (Howard Hinnant's days-to-civil).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A navigation entry: a link to another feed.
fn nav_entry(x: &mut Xml, id: &str, title: &str, summary: &str, href: &str, kind: &str) {
    x.raw("<entry>\n");
    x.el("id", &format!("urn:folio:{id}"));
    x.el("title", title);
    x.el("updated", &rfc3339(super::now()));
    if !summary.is_empty() {
        x.el("content", summary);
    }
    x.link("subsection", href, kind, "");
    x.raw("</entry>\n");
}

/// One book as an acquisition entry: its facts, cover and a download link per file.
fn book_entry(x: &mut Xml, it: &Item) {
    let m = &it.meta;
    x.raw("<entry>\n");
    x.el("id", &format!("urn:folio:book:{}", it.id));
    x.el("title", &it.full_title());
    x.el("updated", &rfc3339(it.updated_at));
    for a in &m.authors {
        x.raw(&format!("<author><name>{}</name></author>\n", esc(&a.name)));
    }
    for n in &m.narrators {
        x.raw(&format!("<contributor><name>{}</name></contributor>\n", esc(n)));
    }
    if !m.language.is_empty() {
        x.el("dc:language", &m.language);
    }
    if !m.publisher.is_empty() {
        x.el("dc:publisher", &m.publisher);
    }
    if !m.release_date.is_empty() {
        x.el("dc:issued", &m.release_date);
    }
    let ident = if !m.isbn13.is_empty() { format!("urn:isbn:{}", m.isbn13) } else { format!("urn:asin:{}", m.asin) };
    if ident.len() > 9 {
        x.el("dc:identifier", &ident);
    }
    for g in &m.genres {
        x.raw(&format!("<category term=\"{0}\" label=\"{0}\"/>\n", esc(g)));
    }
    let mut about = Vec::new();
    if let Some(s) = m.series.first() {
        about.push(if s.position.is_empty() { s.name.clone() } else { format!("{} #{}", s.name, s.position) });
    }
    if !m.narrators.is_empty() {
        about.push(format!("Narrated by {}", m.narrators.join(", ")));
    }
    let blurb = super::plain(&m.description);
    if !blurb.is_empty() {
        about.push(blurb);
    }
    if !about.is_empty() {
        x.el("summary", &about.join(". "));
    }
    if let Some((_, mime)) = &it.cover {
        let c = format!("/opds/books/{}/cover", it.id);
        x.link("http://opds-spec.org/image", &c, mime, "");
        x.link("http://opds-spec.org/image/thumbnail", &c, mime, "");
    }
    for f in &it.files {
        let href = format!("/opds/books/{}/files/{}/{}", it.id, f.id, urlencode(&f.name));
        x.link("http://opds-spec.org/acquisition", &href, &f.mime, &f.name);
    }
    x.raw("</entry>\n");
}

/// A name made safe for one part of an address.
pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A feed of `items` (already in order), one page of them, with next and previous links.
fn books_feed(id: &str, title: &str, path: &str, items: &[Item], page: Option<usize>) -> Response {
    let page = page.unwrap_or(1).max(1);
    let pages = items.len().div_ceil(PAGE).max(1);
    let mut x = Xml::default();
    let sep = if path.contains('?') { '&' } else { '?' };
    let here = if page > 1 { format!("{path}{sep}page={page}") } else { path.to_string() };
    head(&mut x, id, title, &here, ACQ);
    x.el("opensearch:totalResults", &items.len().to_string());
    x.el("opensearch:itemsPerPage", &PAGE.to_string());
    x.el("opensearch:startIndex", &((page - 1) * PAGE + 1).to_string());
    if page < pages {
        x.link("next", &format!("{path}{sep}page={}", page + 1), ACQ, "");
    }
    if page > 1 {
        x.link("previous", &format!("{path}{sep}page={}", page - 1), ACQ, "");
    }
    for it in items.iter().skip((page - 1) * PAGE).take(PAGE) {
        book_entry(&mut x, it);
    }
    x.raw("</feed>\n");
    feed(ACQ, x.done())
}

/// The newest first.
fn newest(mut items: Vec<Item>) -> Vec<Item> {
    items.sort_by(|a, b| b.added_at.total_cmp(&a.added_at).then(b.id.cmp(&a.id)));
    items
}

/// `GET /opds`: the start.
async fn start(State(_st): State<AppState>, _who: Who) -> Response {
    let mut x = Xml::default();
    head(&mut x, "root", "Folio", "/opds", NAV);
    nav_entry(&mut x, "new", "New books", "The newest books first.", "/opds/new", ACQ);
    nav_entry(&mut x, "audiobooks", "Audiobooks", "Every audiobook, newest first.", "/opds/audiobooks", ACQ);
    nav_entry(&mut x, "ebooks", "Ebooks", "Every ebook, newest first.", "/opds/ebooks", ACQ);
    nav_entry(&mut x, "authors", "Authors", "Books by author.", "/opds/authors", NAV);
    nav_entry(&mut x, "series", "Series", "Books by series, in order.", "/opds/series", NAV);
    x.raw("</feed>\n");
    feed(NAV, x.done())
}

/// `GET /opds/new`.
async fn new_books(State(st): State<AppState>, who: Who, Query(p): Query<Paging>) -> Result<Response> {
    let items = newest(shelf::items(&st, &who)?);
    Ok(books_feed("new", "New books", "/opds/new", &items, p.page))
}

/// `GET /opds/audiobooks`.
async fn audiobooks(State(st): State<AppState>, who: Who, Query(p): Query<Paging>) -> Result<Response> {
    let items = newest(shelf::items(&st, &who)?.into_iter().filter(Item::is_audio).collect());
    Ok(books_feed("audiobooks", "Audiobooks", "/opds/audiobooks", &items, p.page))
}

/// `GET /opds/ebooks`.
async fn ebooks(State(st): State<AppState>, who: Who, Query(p): Query<Paging>) -> Result<Response> {
    let items = newest(shelf::items(&st, &who)?.into_iter().filter(|i| !i.is_audio()).collect());
    Ok(books_feed("ebooks", "Ebooks", "/opds/ebooks", &items, p.page))
}

/// The people or series named in `items`, by their key, with their name and how many books: (key, name, count),
/// in name order.
fn groups(items: &[Item], names: impl Fn(&Item) -> Vec<String>) -> Vec<(String, String, usize)> {
    let mut out: std::collections::BTreeMap<String, (String, usize)> = Default::default();
    for it in items {
        for n in names(it) {
            let key = text::person_key(&n);
            if key.is_empty() {
                continue;
            }
            out.entry(key).or_insert_with(|| (n.clone(), 0)).1 += 1;
        }
    }
    let mut v: Vec<_> = out.into_iter().map(|(k, (n, c))| (k, n, c)).collect();
    v.sort_by_key(|g| text::sort_name(&g.1).to_lowercase());
    v
}

/// The authors' names of an item.
fn author_names(it: &Item) -> Vec<String> {
    it.meta.authors.iter().map(|a| a.name.clone()).collect()
}

/// The series' names of an item.
fn series_names(it: &Item) -> Vec<String> {
    it.meta.series.iter().map(|s| s.name.clone()).collect()
}

/// A navigation feed of groups, each a link to `<base>/<key>`.
fn groups_feed(id: &str, title: &str, base: &str, groups: &[(String, String, usize)]) -> Response {
    let mut x = Xml::default();
    head(&mut x, id, title, base, NAV);
    for (key, name, count) in groups {
        let books = if *count == 1 { "1 book".to_string() } else { format!("{count} books") };
        nav_entry(&mut x, &format!("{id}:{key}"), name, &books, &format!("{base}/{}", urlencode(key)), ACQ);
    }
    x.raw("</feed>\n");
    feed(NAV, x.done())
}

/// `GET /opds/authors`.
async fn authors(State(st): State<AppState>, who: Who) -> Result<Response> {
    let items = shelf::items(&st, &who)?;
    Ok(groups_feed("authors", "Authors", "/opds/authors", &groups(&items, author_names)))
}

/// `GET /opds/authors/{key}`: the author's books, newest first.
async fn author(State(st): State<AppState>, who: Who, Path(key): Path<String>, Query(p): Query<Paging>) -> Result<Response> {
    let items = shelf::items(&st, &who)?;
    let name = groups(&items, author_names).into_iter().find(|g| g.0 == key).map(|g| g.1);
    let Some(name) = name else { return Err(err(404, "There is no author like that in the library.")) };
    let mine = newest(items.into_iter().filter(|i| author_names(i).iter().any(|n| text::person_key(n) == key)).collect());
    Ok(books_feed(&format!("authors:{key}"), &name, &format!("/opds/authors/{}", urlencode(&key)), &mine, p.page))
}

/// `GET /opds/series`.
async fn series_list(State(st): State<AppState>, who: Who) -> Result<Response> {
    let items = shelf::items(&st, &who)?;
    Ok(groups_feed("series", "Series", "/opds/series", &groups(&items, series_names)))
}

/// The place of `it` in the series with key `key`, as a number to sort by (unknown places last).
fn place(it: &Item, key: &str) -> f64 {
    it.meta
        .series
        .iter()
        .find(|s| text::person_key(&s.name) == key)
        .and_then(|s| s.position.split(['-', ' ']).next().and_then(|p| p.parse::<f64>().ok()))
        .unwrap_or(f64::MAX)
}

/// `GET /opds/series/{key}`: the series' books in order.
async fn series(State(st): State<AppState>, who: Who, Path(key): Path<String>, Query(p): Query<Paging>) -> Result<Response> {
    let items = shelf::items(&st, &who)?;
    let name = groups(&items, series_names).into_iter().find(|g| g.0 == key).map(|g| g.1);
    let Some(name) = name else { return Err(err(404, "There is no series like that in the library.")) };
    let mut mine: Vec<Item> = items.into_iter().filter(|i| series_names(i).iter().any(|n| text::person_key(n) == key)).collect();
    mine.sort_by(|a, b| place(a, &key).total_cmp(&place(b, &key)).then(a.id.cmp(&b.id)));
    Ok(books_feed(&format!("series:{key}"), &name, &format!("/opds/series/{}", urlencode(&key)), &mine, p.page))
}

/// Whether `it` has every word of `q` in its title, subtitle, authors, narrators, series or ISBN/ASIN.
pub fn matches(it: &Item, q: &str) -> bool {
    let m = &it.meta;
    let hay = text::person_key(&format!(
        "{} {} {} {} {} {} {}",
        m.title,
        m.subtitle,
        it.authors(),
        m.narrators.join(" "),
        m.series.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" "),
        m.isbn13,
        m.asin
    ));
    let words: Vec<String> = q.split_whitespace().map(text::person_key).filter(|w| !w.is_empty()).collect();
    !words.is_empty() && words.iter().all(|w| hay.contains(w.as_str()))
}

/// `GET /opds/search?q=`.
async fn search(State(st): State<AppState>, who: Who, Query(p): Query<Paging>) -> Result<Response> {
    let items = newest(shelf::items(&st, &who)?.into_iter().filter(|i| matches(i, &p.q)).collect());
    let path = format!("/opds/search?q={}", urlencode(&p.q));
    Ok(books_feed("search", &format!("Search: {}", p.q), &path, &items, p.page))
}

/// `GET /opds/search.xml`: how to search (OpenSearch 1.1).
async fn opensearch() -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<OpenSearchDescription xmlns=\"http://a9.com/-/spec/opensearch/1.1/\">\n\
         <ShortName>Folio</ShortName>\n<Description>Search the library by title, author, narrator or series.</Description>\n\
         <InputEncoding>UTF-8</InputEncoding>\n<OutputEncoding>UTF-8</OutputEncoding>\n\
         <Url type=\"{ACQ}\" template=\"/opds/search?q={{searchTerms}}\"/>\n</OpenSearchDescription>\n"
    );
    ([(header::CONTENT_TYPE, HeaderValue::from_static("application/opensearchdescription+xml"))], body).into_response()
}

/// `GET /opds/books/{id}/files/{file}[/name]`: one file of a book.
async fn file(State(st): State<AppState>, who: Who, Path((id, fid)): Path<(i64, String)>, req: Request) -> Result<Response> {
    let it = shelf::item(&st, &who, id)?;
    let fid: i64 = fid.parse().map_err(|_| err(404, "That file isn't in the library."))?;
    let f = it.files.iter().find(|f| f.id == fid).ok_or_else(|| err(404, "That file isn't in the library."))?;
    super::send_file(&f.path, &f.mime, Some(&f.name), req).await
}

/// `GET /opds/books/{id}/files/{file}/{name}`: the same, with the file's name at the end for readers that name
/// the download after the address.
async fn file_named(st: State<AppState>, who: Who, Path((id, fid, _)): Path<(i64, String, String)>, req: Request) -> Result<Response> {
    file(st, who, Path((id, fid)), req).await
}

/// `GET /opds/books/{id}/cover`.
async fn cover(State(st): State<AppState>, who: Who, Path(id): Path<i64>, req: Request) -> Result<Response> {
    let it = shelf::item(&st, &who, id)?;
    match &it.cover {
        Some((path, mime)) => super::send_file(path, mime, None, req).await,
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_rfc3339() {
        assert_eq!(rfc3339(0.0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_843_239.0), "2026-11-05T01:47:19Z");
        assert_eq!(rfc3339(951_782_400.0), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn names_are_encoded_for_addresses() {
        assert_eq!(urlencode("Hail Mary (2021).m4b"), "Hail%20Mary%20%282021%29.m4b");
        assert_eq!(urlencode("é"), "%C3%A9");
    }

    #[test]
    fn search_needs_every_word() {
        let mut it = Item::default();
        it.meta.title = "Project Hail Mary".into();
        it.meta.narrators = vec!["Ray Porter".into()];
        assert!(matches(&it, "hail porter"));
        assert!(!matches(&it, "hail weir"));
        assert!(!matches(&it, "  "));
    }
}
