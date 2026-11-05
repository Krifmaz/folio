//! Readarr's lookups (`BookLookupController`, `AuthorLookupController`, `SearchController`): books and authors
//! found through [`super::bookinfo`], to add them to the library.
//!
//! ```text
//! GET /api/v1/book/lookup?term=     books for a term, with all their editions
//! GET /api/v1/author/lookup?term=   the authors of the books for a term
//! GET /api/v1/search?term=          authors and books for a term, each book with its found edition
//! ```
//!
//! A term can name what it wants: `author:1654`, `work:7000`, `edition:7000001`, `isbn:…` or `asin:…`. Covers
//! of what the library has point to Folio's copies; the others go through the cover proxy.

use super::bookinfo::{lock, ratings_json, Book, BookInfo, Edition, Found, Shared};
use super::json::{date, date_or_min, object, Query};
use super::naming::{self, Naming};
use super::{authors, Answer, Fail, Key};
use crate::AppState;
use axum::extract::{RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Row, RowExt};
use serde_json::{json, Value};

/// The lookup routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/book/lookup", get(book_lookup))
        .route("/api/v1/author/lookup", get(author_lookup))
        .route("/api/v1/search", get(search))
}

/// The search term, as ASP.NET binds it: an empty one is none.
fn term(q: Option<&str>) -> Option<String> {
    Query::of(q).get("term").filter(|t| !t.is_empty()).map(str::to_string)
}

/// A value, or nothing when it is 0 (Readarr leaves out ids that are 0).
fn id_or_none(id: i64) -> Value {
    if id == 0 {
        Value::Null
    } else {
        json!(id)
    }
}

/// The monitored edition of a book (`SingleOrDefault`): 500 like Readarr when there are several.
fn selected(b: &Book) -> Answer<Option<Shared<Edition>>> {
    let mut m = b.monitored_editions();
    if m.len() > 1 {
        return Err(Fail::exception(500, "System.InvalidOperationException", "Sequence contains more than one matching element"));
    }
    Ok(m.pop())
}

/// An edition as `EditionResource.ToResource` makes it.
fn edition_json(e: &Edition) -> Value {
    object([
        ("id", id_or_none(e.id)),
        ("bookId", json!(e.book_id)),
        ("foreignEditionId", json!(e.foreign_edition_id)),
        ("titleSlug", json!(e.title_slug)),
        ("isbn13", e.isbn13.clone()),
        ("asin", e.asin.clone()),
        ("title", json!(e.title)),
        ("language", e.language.clone()),
        ("overview", e.overview.clone()),
        ("format", e.format.clone()),
        ("isEbook", json!(e.is_ebook)),
        ("disambiguation", e.disambiguation.clone()),
        ("publisher", e.publisher.clone()),
        ("pageCount", json!(e.page_count)),
        ("releaseDate", json!(e.release_date)),
        ("images", Value::Array(lock(&e.images).clone())),
        ("links", Value::Array(e.links.clone())),
        ("ratings", ratings_json(e.votes, e.value)),
        ("monitored", json!(e.monitored)),
        ("manualAdd", json!(e.manual_add)),
        ("grabbed", json!(false)),
    ])
}

/// A book as `BookResource.ToResource` makes it, from its monitored edition `sel`.
fn book_json(b: &Book, sel: Option<&Shared<Edition>>) -> Value {
    let e = sel.map(|s| lock(s).clone());
    let title = e.as_ref().map(|e| e.title.clone()).or_else(|| b.title.clone());
    let sort_name = b.author.get("SortNameLastFirst").and_then(Value::as_str).unwrap_or_default();
    let author_title = format!("{sort_name} {}", title.clone().unwrap_or_default());
    let mut links = b.series_links.clone();
    links.sort_by_key(|l| l.series_position);
    let series_title = links
        .iter()
        .map(|l| match l.position.as_deref().filter(|p| !p.trim().is_empty()) {
            Some(p) => format!("{} #{p}", l.title.clone().unwrap_or_default()),
            None => l.title.clone().unwrap_or_default(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut all_links = b.links.clone();
    if let Some(e) = &e {
        all_links.extend(e.links.iter().cloned());
    }
    object([
        ("id", id_or_none(b.id)),
        ("authorId", json!(b.author.i("Id"))),
        ("foreignBookId", json!(b.foreign_book_id)),
        ("foreignEditionId", json!(e.as_ref().map(|e| e.foreign_edition_id.clone()))),
        ("titleSlug", json!(b.title_slug)),
        ("monitored", json!(b.monitored)),
        ("anyEditionOk", json!(b.any_edition_ok)),
        ("releaseDate", json!(b.release_date)),
        ("pageCount", json!(e.as_ref().map(|e| e.page_count).unwrap_or(0))),
        ("genres", b.genres.clone()),
        ("title", json!(title)),
        ("authorTitle", json!(author_title)),
        ("seriesTitle", json!(series_title)),
        ("disambiguation", e.as_ref().map(|e| e.disambiguation.clone()).unwrap_or(Value::Null)),
        ("images", Value::Array(e.as_ref().map(|e| lock(&e.images).clone()).unwrap_or_default())),
        ("links", Value::Array(all_links)),
        ("ratings", e.as_ref().map(|e| ratings_json(e.votes, e.value)).unwrap_or_else(|| ratings_json(0, 0.0))),
        ("added", date_or_min(Some(&b.added))),
        ("lastSearchTime", date(Some(&b.last_search_time))),
        ("grabbed", json!(false)),
    ])
}

/// An author as `AuthorResource.ToResource` makes it (covers as stored).
fn author_json(a: &Row) -> Value {
    let mut v = authors::resource(a);
    if a.i("Id") == 0 {
        if let Some(o) = v.as_object_mut() {
            o.remove("id");
        }
    }
    v
}

/// The first cover of a kind's `remoteUrl`, if any.
fn remote(images: &[Value], kind: &str) -> Value {
    images.iter().find(|i| i["coverType"] == json!(kind)).and_then(|i| i.get("remoteUrl").cloned()).unwrap_or(Value::Null)
}

/// Converts a book's covers in place (they are its monitored edition's, shared with the metadata Readarr keeps)
/// and gives them.
fn convert_book_covers(info: &BookInfo, b: &Book, sel: Option<&Shared<Edition>>) -> Vec<Value> {
    let Some(sel) = sel else { return Vec::new() };
    let images = lock(sel).images.clone();
    let mut list = lock(&images);
    info.convert_covers(&mut list, "book", b.id);
    list.clone()
}

/// An author as the lookups send it: covers converted, with `remotePoster` and its folder.
fn found_author(info: &BookInfo, a: &Row, n: &Naming) -> Value {
    let mut v = author_json(a);
    let mut images = match v.get("images") {
        Some(Value::Array(i)) => i.clone(),
        _ => Vec::new(),
    };
    info.convert_covers(&mut images, "author", a.i("Id"));
    let poster = remote(&images, "poster");
    v["images"] = Value::Array(images);
    if !poster.is_null() {
        v["remotePoster"] = poster;
    }
    v["folder"] = json!(naming::author_folder(a, n));
    v
}

/// `GET /api/v1/book/lookup`.
async fn book_lookup(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let info = &st.readarr_lookup;
    let books = info.search_for_new_book(&st.readarr, term(q.as_deref()).as_deref(), true).await?;
    let mut out = Vec::new();
    for book in &books {
        let b = lock(book).clone();
        let sel = selected(&b)?;
        let images = convert_book_covers(info, &b, sel.as_ref());
        let mut v = book_json(&b, sel.as_ref());
        let cover = remote(&images, "cover");
        if !cover.is_null() {
            v["remoteCover"] = cover;
        }
        out.push(v);
    }
    Ok(Json(Value::Array(out)))
}

/// `GET /api/v1/author/lookup`.
async fn author_lookup(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let info = &st.readarr_lookup;
    let found = info.search_for_new_author(&st.readarr, term(q.as_deref()).as_deref()).await?;
    let n = naming::config(&st.readarr)?;
    Ok(Json(Value::Array(found.iter().map(|a| found_author(info, a, &n)).collect())))
}

/// `GET /api/v1/search`.
async fn search(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let info = &st.readarr_lookup;
    let found = info.search_for_new_entity(&st.readarr, term(q.as_deref()).as_deref()).await?;
    let n = naming::config(&st.readarr)?;
    let mut out = Vec::new();
    for (i, f) in found.iter().enumerate() {
        let id = json!(i + 1);
        match f {
            Found::Author(a) => {
                out.push(json!({ "id": id, "foreignId": a.s("ForeignAuthorId"), "author": found_author(info, a, &n) }));
            }
            Found::Book(book) => {
                let b = lock(book).clone();
                let sel = selected(&b)?;
                let monitored = b.monitored_editions();
                let overview = match monitored.as_slice() {
                    [one] => lock(one).overview.clone(),
                    [] => return Err(Fail::exception(500, "System.InvalidOperationException", "Sequence contains no matching element")),
                    _ => {
                        return Err(Fail::exception(
                            500,
                            "System.InvalidOperationException",
                            "Sequence contains more than one matching element",
                        ))
                    }
                };
                let images = convert_book_covers(info, &b, sel.as_ref());
                let mut v = book_json(&b, sel.as_ref());
                if !overview.is_null() {
                    v["overview"] = overview;
                }
                let mut author = author_json(&b.author);
                author["folder"] = json!(naming::author_folder(&b.author, &n));
                v["author"] = author;
                v["editions"] = Value::Array(b.editions.iter().map(|e| edition_json(&lock(e))).collect());
                let cover = remote(&images, "cover");
                if !cover.is_null() {
                    v["remoteCover"] = cover;
                }
                out.push(json!({ "id": id, "foreignId": b.foreign_book_id, "book": v }));
            }
        }
    }
    Ok(Json(Value::Array(out)))
}
