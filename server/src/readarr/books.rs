//! Readarr's books (`Readarr.Api.V1/Books`): a book with its monitored edition, series and statistics, as
//! `GET /api/v1/book` and `GET /api/v1/book/{id}` send them, and a book's overview.
//!
//! ```text
//! GET /api/v1/book?authorId=&bookIds=&titleSlug=&includeAllAuthorBooks=   books (all of them without a filter)
//! GET /api/v1/book/{id}                                                    one book, with its author
//! GET /api/v1/book/{id}/overview                                           {id, overview} of its monitored edition
//! PUT /api/v1/book/{id}                                                    change a book and its editions (202)
//! PUT /api/v1/book/monitor   {bookIds, monitored}                          monitor books (202 with them)
//! PUT /api/v1/book/editor    {bookIds, monitored}                          change several books (202 with them)
//! ```

use super::json::{date, date_or_min, doc, list, local_covers, marks, object, percent, ratings, Query};
use super::{authors, Answer, Fail, Key};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The book routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/book", get(all).put(update_unrouted).post(super::add::add_book))
        .route("/api/v1/book/{id}", get(one).put(update).delete(remove))
        .route("/api/v1/book/{id}/overview", get(overview))
        .route("/api/v1/book/monitor", put(monitor))
        .route("/api/v1/book/editor", put(editor).delete(remove_many))
}

/// A book's stored add options, or the default ones a book without them gets (`AddBookOptions`).
pub fn add_options(v: Option<&Value>) -> Value {
    match doc(v) {
        Value::Null => json!({ "addType": "automatic", "searchForNewBook": false }),
        o => o,
    }
}

/// The books matching `filter` (SQL after `WHERE` on `b`, or empty), in the order they were added.
pub fn rows(db: &SqliteDb, filter: &str, params: Vec<Value>) -> kit_server_core::Result<Vec<Row>> {
    let sql = if filter.is_empty() {
        "SELECT b.* FROM Books b ORDER BY b.Id".to_string()
    } else {
        format!("SELECT b.* FROM Books b WHERE {filter} ORDER BY b.Id")
    };
    db.query(&sql, params)
}

/// The book `id`, or Readarr's `ModelNotFoundException`.
pub fn get_or_404(db: &SqliteDb, id: i64) -> Answer<Row> {
    rows(db, "b.Id = ?", vec![json!(id)])?.into_iter().next().ok_or_else(|| Fail::model_not_found("Book", id))
}

/// The books of the author `author_id` (by its metadata), in the order they were added.
fn of_author(db: &SqliteDb, author_id: i64) -> kit_server_core::Result<Vec<Row>> {
    rows(db, "b.AuthorMetadataId IN (SELECT AuthorMetadataId FROM Authors WHERE Id = ?)", vec![json!(author_id)])
}

/// What a book's resource needs besides the book: its monitored edition, series links and author.
struct Context {
    /// each book's monitored editions, by book id
    editions: HashMap<i64, Vec<Row>>,
    /// each book's series links with the series' title, by book id
    series: HashMap<i64, Vec<Row>>,
    /// the authors (with metadata), by metadata id
    authors: HashMap<i64, Row>,
}

impl Context {
    /// The context of `books`.
    fn of(db: &SqliteDb, books: &[Row]) -> kit_server_core::Result<Context> {
        let ids: Vec<Value> = books.iter().map(|b| json!(b.i("Id"))).collect();
        let mut editions: HashMap<i64, Vec<Row>> = HashMap::new();
        let mut series: HashMap<i64, Vec<Row>> = HashMap::new();
        if !ids.is_empty() {
            let sql = format!("SELECT * FROM Editions WHERE Monitored = 1 AND BookId IN ({}) ORDER BY Id", marks(ids.len()));
            for e in db.query(&sql, ids.clone())? {
                editions.entry(e.i("BookId")).or_default().push(e);
            }
            let sql = format!(
                "SELECT l.*, s.Title AS SeriesTitle FROM SeriesBookLink l LEFT JOIN Series s ON s.Id = l.SeriesId
                 WHERE l.BookId IN ({}) ORDER BY l.Id",
                marks(ids.len())
            );
            for l in db.query(&sql, ids)? {
                series.entry(l.i("BookId")).or_default().push(l);
            }
        }
        let authors = authors::rows(db, "", vec![])?.into_iter().map(|a| (a.i("AuthorMetadataId"), a)).collect();
        Ok(Context { editions, series, authors })
    }
}

/// A book as `BookResource.ToResource` makes it, with its covers as Folio's copies (`local`) or as stored. 500
/// like Readarr when the book has more than one monitored edition.
fn resource(b: &Row, cx: &Context, local: bool) -> Answer<Value> {
    let id = b.i("Id");
    let monitored = cx.editions.get(&id).map(Vec::as_slice).unwrap_or_default();
    if monitored.len() > 1 {
        return Err(Fail::exception(500, "System.InvalidOperationException", "Sequence contains more than one matching element"));
    }
    let edition = monitored.first();
    let author = cx.authors.get(&b.i("AuthorMetadataId"));
    let title = edition.map(|e| e.s("Title")).unwrap_or_else(|| b.s("Title"));
    let author_title = format!("{} {title}", author.map(|a| a.s("SortNameLastFirst")).unwrap_or_default());
    let mut links = cx.series.get(&id).cloned().unwrap_or_default();
    links.sort_by_key(|l| l.i("SeriesPosition"));
    let series_title = links
        .iter()
        .map(|l| {
            let pos = l.s("Position");
            if pos.trim().is_empty() {
                l.s("SeriesTitle")
            } else {
                format!("{} #{pos}", l.s("SeriesTitle"))
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut all_links = match list(b.get("Links")) {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    if let Some(e) = edition {
        if let Value::Array(a) = list(e.get("Links")) {
            all_links.extend(a);
        }
    }
    let images = edition.map(|e| super::json::images(e.get("Images"))).unwrap_or_default();
    Ok(object([
        ("title", json!(title)),
        ("authorTitle", json!(author_title)),
        ("seriesTitle", json!(series_title)),
        ("disambiguation", edition.and_then(|e| e.get("Disambiguation").cloned()).unwrap_or(Value::Null)),
        ("authorId", json!(author.map(|a| a.i("Id")).unwrap_or(0))),
        ("foreignBookId", b.get("ForeignBookId").cloned().unwrap_or(Value::Null)),
        ("foreignEditionId", edition.and_then(|e| e.get("ForeignEditionId").cloned()).unwrap_or(Value::Null)),
        ("titleSlug", b.get("TitleSlug").cloned().unwrap_or(Value::Null)),
        ("monitored", json!(b.i("Monitored") != 0)),
        ("anyEditionOk", json!(b.i("AnyEditionOk") != 0)),
        ("ratings", edition.map(|e| ratings(e.get("Ratings"))).unwrap_or_else(|| ratings(None))),
        ("releaseDate", date(b.get("ReleaseDate"))),
        ("pageCount", json!(edition.map(|e| e.i("PageCount")).unwrap_or(0))),
        ("genres", list(b.get("Genres"))),
        ("images", Value::Array(if local { local_covers(images, "book", id) } else { images })),
        ("links", Value::Array(all_links)),
        ("added", date_or_min(b.get("Added"))),
        ("lastSearchTime", date(b.get("LastSearchTime"))),
        ("grabbed", json!(false)),
        ("id", json!(id)),
    ]))
}

/// A book's statistics as the API sends them (`BookStatisticsResource`).
fn stats_json(s: &authors::Stats) -> Value {
    json!({
        "bookFileCount": s.book_file_count,
        "bookCount": s.book_count,
        "totalBookCount": s.total_book_count,
        "sizeOnDisk": s.size_on_disk,
        "percentOfBooks": percent(s.book_file_count, s.book_count),
    })
}

/// Books as the list routes send them (`MapToResource(books, false)`): with statistics, without their author.
pub fn many(db: &SqliteDb, books: &[Row]) -> Answer<Vec<Value>> {
    let cx = Context::of(db, books)?;
    let stats: HashMap<i64, authors::Stats> = authors::book_stats(db, None)?.into_iter().map(|s| (s.book_id, s)).collect();
    books
        .iter()
        .map(|b| {
            let mut v = resource(b, &cx, true)?;
            if let Some(s) = stats.get(&b.i("Id")) {
                v["statistics"] = stats_json(s);
            }
            Ok(v)
        })
        .collect()
}

/// `GET /api/v1/book`.
async fn all(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let db = &st.readarr;
    let q = Query::of(q.as_deref());
    let ids = q.ints("bookIds");
    let slug = q.get("titleSlug").filter(|s| !s.trim().is_empty());
    let books = if let Some(author) = q.int("authorId") {
        authors::get_or_404(db, author)?;
        of_author(db, author)?
    } else if let Some(slug) = slug {
        match rows(db, "b.TitleSlug = ?", vec![json!(slug)])?.into_iter().next() {
            None => Vec::new(),
            Some(b) if q.bool("includeAllAuthorBooks") == Some(true) => {
                let author = db.one("SELECT Id FROM Authors WHERE AuthorMetadataId = ?", vec![json!(b.i("AuthorMetadataId"))])?;
                of_author(db, author.map(|a| a.i("Id")).unwrap_or(0))?
            }
            Some(b) => vec![b],
        }
    } else if !ids.is_empty() {
        let found = rows(db, &format!("b.Id IN ({})", marks(ids.len())), ids.iter().map(|i| json!(i)).collect())?;
        if found.len() != ids.len() {
            return Err(Fail::exception(
                500,
                "System.ApplicationException",
                format!("Expected query to return {} rows but returned {}", ids.len(), found.len()),
            ));
        }
        found
    } else {
        rows(db, "", vec![])?
    };
    Ok(Json(Value::Array(many(db, &books)?)))
}

/// `GET /api/v1/book/{id}`: the book with its author (`MapToResource(book, true)`).
async fn one(State(st): State<AppState>, _: Key, Path(id): Path<String>) -> Answer {
    let id: i64 = id.parse().map_err(|_| Fail::not_found())?;
    let Some(b) = rows(&st.readarr, "b.Id = ?", vec![json!(id)])?.into_iter().next() else { return Err(Fail::not_found()) };
    Ok(Json(with_author(&st.readarr, &b)?))
}

/// A book as `GET /api/v1/book/{id}` sends it: with its author and statistics.
pub(super) fn with_author(db: &SqliteDb, b: &Row) -> Answer<Value> {
    let id = b.i("Id");
    let cx = Context::of(db, std::slice::from_ref(b))?;
    let mut v = resource(b, &cx, true)?;
    let author = cx.authors.get(&b.i("AuthorMetadataId"));
    if let Some(a) = author {
        v["author"] = authors::resource(a);
        let stats = authors::book_stats(db, Some(a.i("Id")))?;
        if let Some(s) = stats.iter().find(|s| s.book_id == id) {
            v["statistics"] = stats_json(s);
        }
    }
    Ok(v)
}

/// The books with these ids, or Readarr's `Expected query to return …` when one is missing.
fn get_many(db: &SqliteDb, ids: &[i64]) -> Answer<Vec<Row>> {
    let found = if ids.is_empty() {
        Vec::new()
    } else {
        rows(db, &format!("b.Id IN ({})", marks(ids.len())), ids.iter().map(|i| json!(i)).collect())?
    };
    authors::get_many(found, ids.len())
}

/// `PUT /api/v1/book` (the id from the body).
async fn update_unrouted(st: State<AppState>, key: Key, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    put_book(st, key, 0, bytes).await
}

/// `PUT /api/v1/book/{id}`.
async fn update(st: State<AppState>, key: Key, Path(id): Path<i64>, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    put_book(st, key, id, bytes).await
}

/// Changes a book like `BookController.UpdateBook`: its foreign id, add options, monitoring and "any edition",
/// and every edition sent (all their fields); 500 like Readarr when no editions are sent. 202 with the book.
async fn put_book(State(st): State<AppState>, _: Key, route_id: i64, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    let id = match r.get("id").and_then(Value::as_i64).unwrap_or(0) {
        0 => route_id,
        id => id,
    };
    let mut f = super::validation::Failures::default();
    f.valid_id("Id", id);
    f.check()?;
    let stored = get_or_404(db, id)?;
    let Some(editions) = r.get("editions").and_then(Value::as_array) else {
        return Err(Fail::exception(500, "System.ArgumentNullException", "Value cannot be null. (Parameter 'source')"));
    };
    let flag = |k: &str| json!(r.get(k).and_then(Value::as_bool).unwrap_or(false));
    db.execute(
        "UPDATE Books SET ForeignBookId = ?, AddOptions = ?, Monitored = ?, AnyEditionOk = ?, Ratings = ? WHERE Id = ?",
        vec![
            r.get("foreignBookId").cloned().unwrap_or(Value::Null),
            // Readarr writes the whole book back: add options (the default ones when none are sent) and its
            // ratings with their popularity.
            json!(add_options(r.get("addOptions").filter(|v| !v.is_null()).map(|v| json!(v.to_string())).as_ref()).to_string()),
            flag("monitored"),
            flag("anyEditionOk"),
            json!(ratings(stored.get("Ratings")).to_string()),
            json!(id),
        ],
    )
    .map_err(conflict)?;
    for e in editions {
        super::editions::store(db, e).map_err(conflict)?;
    }
    let b = get_or_404(db, id)?;
    Ok((StatusCode::ACCEPTED, Json(with_author(db, &b)?)))
}

/// A write SQLite refused for a constraint (a field that can't be empty): Readarr's 409.
fn conflict(e: kit_server_core::Error) -> kit_server_core::Error {
    if e.detail.contains("constraint") {
        kit_server_core::err(409, e.detail)
    } else {
        e
    }
}

/// `PUT /api/v1/book/monitor` (`SetBooksMonitored`): 202 with the books as the list routes send them.
async fn monitor(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    let ids = authors::ids(&r, "bookIds");
    let monitored = r.get("monitored").and_then(Value::as_bool).unwrap_or(false);
    if !ids.is_empty() {
        db.execute(&format!("UPDATE Books SET Monitored = ? WHERE Id IN ({})", marks(ids.len())), {
            let mut p = vec![json!(monitored)];
            p.extend(ids.iter().map(|i| json!(i)));
            p
        })?;
    }
    let books = get_many(db, &ids)?;
    Ok((StatusCode::ACCEPTED, Json(Value::Array(many(db, &books)?))))
}

/// `PUT /api/v1/book/editor` (`BookEditorController.SaveAll`): monitoring of several books; 202 with them as
/// `ToResource` makes them (covers as stored, no statistics).
async fn editor(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    let ids = authors::ids(&r, "bookIds");
    let books = get_many(db, &ids)?;
    let monitored = r.get("monitored").and_then(Value::as_bool);
    for b in &books {
        // `UpdateMany` writes the whole book back, so its add options and ratings come out as Readarr writes
        // them (the default add options when it has none, ratings with their popularity).
        db.execute(
            "UPDATE Books SET Monitored = ?, AddOptions = ?, Ratings = ? WHERE Id = ?",
            vec![
                json!(monitored.unwrap_or(b.i("Monitored") != 0)),
                json!(add_options(b.get("AddOptions")).to_string()),
                json!(ratings(b.get("Ratings")).to_string()),
                json!(b.i("Id")),
            ],
        )?;
    }
    let books = get_many(db, &ids)?;
    let cx = Context::of(db, &books)?;
    let out = books.iter().map(|b| resource(b, &cx, false)).collect::<Answer<Vec<_>>>()?;
    Ok((StatusCode::ACCEPTED, Json(Value::Array(out))))
}

/// `GET /api/v1/book/{id}/overview`: the overview of the book's one monitored edition; 500 like Readarr when
/// there isn't exactly one.
async fn overview(State(st): State<AppState>, _: Key, Path(id): Path<i64>) -> Answer {
    let eds = st.readarr.query("SELECT Overview FROM Editions WHERE BookId = ? AND Monitored = 1", vec![json!(id)])?;
    match eds.as_slice() {
        [e] => Ok(Json(json!({ "id": id, "overview": e.s("Overview") }))),
        [] => Err(Fail::exception(500, "System.InvalidOperationException", "Sequence contains no matching element")),
        _ => Err(Fail::exception(500, "System.InvalidOperationException", "Sequence contains more than one matching element")),
    }
}

/// `DELETE /api/v1/book/{id}?deleteFiles=&addImportListExclusion=`: 200 with no body.
async fn remove(State(st): State<AppState>, _: Key, Path(id): Path<String>, RawQuery(q): RawQuery) -> Answer<StatusCode> {
    let id: i64 = id.parse().map_err(|_| Fail::not_found())?;
    let q = Query::of(q.as_deref());
    let (files, exclude) = (q.bool("deleteFiles").unwrap_or(false), q.bool("addImportListExclusion").unwrap_or(false));
    super::delete::book(&st.readarr, &st.readarr_disk, id, files, exclude)?;
    Ok(StatusCode::OK)
}

/// `DELETE /api/v1/book/editor` (`BookEditorController.DeleteBook`): deletes each book asked for, until one is
/// missing; 200 with no body.
async fn remove_many(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer<StatusCode> {
    let r = super::body(&bytes)?;
    let flag = |k: &str| r.get(k).and_then(Value::as_bool).unwrap_or(false);
    for id in authors::ids(&r, "bookIds") {
        super::delete::book(&st.readarr, &st.readarr_disk, id, flag("deleteFiles"), flag("addImportListExclusion"))?;
    }
    Ok(StatusCode::OK)
}
