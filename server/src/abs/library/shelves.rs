//! The home page shelves of a book library (`LibraryItem.getPersonalizedShelves` and the `libraryFilters`
//! loaders it calls), for `GET /api/libraries/:id/personalized` (A0013): Continue Listening and Reading,
//! Continue Series, Recently Added, Recent Series, Discover, Listen and Read Again, and Newest Authors.

use super::filters::{self, permission_conditions, PageItem};
use super::util::{natural_cmp, title_prefix_at_end, Sorting};
use crate::abs::models::{book, date_text, from_row, library_item, now_ms, Author, Library, LibraryItem, Series, User};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Map, Value};

/// Sixty days, the age limit of the "recent" shelves.
const RECENT_MS: i64 = 60 * 24 * 60 * 60 * 1000;

/// One shelf as the apps get it.
fn shelf(id: &str, label: &str, key: &str, kind: &str, entities: Vec<Value>, total: i64) -> Value {
    json!({"id": id, "label": label, "labelStringKey": key, "type": kind, "entities": entities, "total": total})
}

/// The page query of a progress or "recent" shelf.
fn page_query(group: &str, value: Option<&str>, sort_by: &str, include: &[String], limit: i64, home_page: bool) -> filters::Query {
    filters::Query {
        group: Some(group.into()),
        value: value.map(String::from),
        sort_by: Some(sort_by.into()),
        sort_desc: true,
        collapse_series: false,
        include: include.to_vec(),
        limit,
        offset: 0,
        home_page,
    }
}

/// Splits items into the ones with audio and the ebook-only ones (the Listen/Read split of the original).
fn audio_and_ebook(items: Vec<Value>) -> (Vec<Value>, Vec<Value>) {
    let has = |v: &Value, k: &str| v["media"].get(k).is_some_and(book::truthy);
    let audio = items.iter().filter(|v| has(v, "numTracks")).cloned().collect();
    let ebook = items.into_iter().filter(|v| has(v, "ebookFormat") && !has(v, "numTracks")).collect();
    (audio, ebook)
}

/// Every shelf of a book library for one user, in the original's order, leaving out empty ones. `limit` is the
/// number of entities per shelf (Recent Series always shows up to 5).
pub fn book_shelves(db: &dyn Db, lib: &Library, u: &User, include: &[String], limit: i64, sorting: &Sorting) -> Result<Vec<Value>> {
    let library_id = lib.id.clone().unwrap_or_default();
    let settings = lib.settings.clone().unwrap_or(Value::Null);
    let minified = |p: &PageItem| Value::Object(library_item::minified(&p.item, &p.book, sorting));
    let mut shelves = Vec::new();

    let (items, count) =
        filters::book_items(db, &library_id, u, &page_query("progress", Some("in-progress"), "progress", include, limit, true), sorting)?;
    let (audio, ebook) = audio_and_ebook(items.iter().map(minified).collect());
    if !audio.is_empty() {
        shelves.push(shelf("continue-listening", "Continue Listening", "LabelContinueListening", "book", audio, count));
    }
    if !ebook.is_empty() {
        shelves.push(shelf("continue-reading", "Continue Reading", "LabelContinueReading", "book", ebook, count));
    }

    let later_only = settings.get("onlyShowLaterBooksInContinueSeries").is_some_and(book::truthy);
    let (items, count) = continue_series(db, &library_id, u, later_only, limit, sorting)?;
    if !items.is_empty() {
        shelves.push(shelf("continue-series", "Continue Series", "LabelContinueSeries", "book", items, count));
    }

    let (items, count) = filters::book_items(db, &library_id, u, &page_query("recent", None, "addedAt", include, limit, false), sorting)?;
    if !items.is_empty() {
        let entities = items.iter().map(|p| super::page_item_json(p, sorting)).collect();
        shelves.push(shelf("recently-added", "Recently Added", "LabelRecentlyAdded", "book", entities, count));
    }

    let hide_single = settings.get("hideSingleBookSeries").is_some_and(book::truthy);
    let (series, count) = recent_series(db, &library_id, u, hide_single, 5, sorting)?;
    if !series.is_empty() {
        shelves.push(shelf("recent-series", "Recent Series", "LabelRecentSeries", "series", series, count));
    }

    let (items, count) = discover(db, &library_id, u, limit, sorting)?;
    if !items.is_empty() {
        shelves.push(shelf("discover", "Discover", "LabelDiscover", "book", items, count));
    }

    let (items, count) =
        filters::book_items(db, &library_id, u, &page_query("progress", Some("finished"), "progress", include, limit, false), sorting)?;
    let (audio, ebook) = audio_and_ebook(items.iter().map(minified).collect());
    if !audio.is_empty() {
        shelves.push(shelf("listen-again", "Listen Again", "LabelListenAgain", "book", audio, count));
    }
    if !ebook.is_empty() {
        shelves.push(shelf("read-again", "Read Again", "LabelReadAgain", "book", ebook, count));
    }

    let (authors, count) = newest_authors(db, &library_id, u, limit)?;
    if !authors.is_empty() {
        shelves.push(shelf("newest-authors", "Newest Authors", "LabelNewestAuthors", "authors", authors, count));
    }
    Ok(shelves)
}

/// The `LIMIT` of a shelf query (none for 0, as the original passes `limit || null`).
fn limit_sql(limit: i64) -> String {
    if limit > 0 {
        format!(" LIMIT {limit}")
    } else {
        String::new()
    }
}

/// An item's book with its authors, for a shelf that shows one book of a series. `None` when the item or book is
/// gone.
fn item_with_book(db: &dyn Db, book_id: &str) -> Result<Option<(LibraryItem, book::BookFull)>> {
    let Some(row) = db.one("SELECT * FROM libraryItems WHERE mediaId = ? LIMIT 1", vec![json!(book_id)])? else { return Ok(None) };
    let item: LibraryItem = from_row(row)?;
    let mut books = book::load(db, &[book_id.to_string()])?;
    Ok(books.remove(book_id).map(|b| (item, b)))
}

/// The "Continue Series" shelf (`getContinueSeriesLibraryItems`): series with a finished book, an unfinished one
/// (later than the last finished when `later_only`) and none in progress, latest progress first; each shows its
/// first unfinished book the user may see.
fn continue_series(db: &dyn Db, library_id: &str, u: &User, later_only: bool, limit: i64, sorting: &Sorting) -> Result<(Vec<Value>, i64)> {
    let user_id = json!(u.id.clone().unwrap_or_default());
    let hidden: Vec<Value> =
        u.extra_data.as_ref().and_then(|x| x.get("seriesHideFromContinueListening")).and_then(Value::as_array).cloned().unwrap_or_default();
    let max_sequence = "(SELECT CAST(max(bs.sequence) as FLOAT) FROM bookSeries bs, mediaProgresses mp WHERE mp.mediaItemId = bs.bookId AND mp.isFinished = 1 AND mp.userId = ? AND bs.seriesId = series.id)";
    let mut not_finished = "SELECT count(*) FROM bookSeries bs LEFT OUTER JOIN mediaProgresses mp ON mp.mediaItemId = bs.bookId AND mp.userId = ? WHERE bs.seriesId = series.id AND (mp.isFinished = 0 OR mp.isFinished IS NULL)".to_string();
    let mut not_finished_params = vec![user_id.clone()];
    if later_only {
        not_finished.push_str(&format!(" AND CAST(bs.sequence as FLOAT) > {max_sequence}"));
        not_finished_params.push(user_id.clone());
    }
    let hidden_sql =
        if hidden.is_empty() { String::new() } else { format!(" AND series.id NOT IN ({})", vec!["?"; hidden.len()].join(", ")) };
    let wheres = format!(
        "series.libraryId = ?{hidden_sql} \
         AND (SELECT count(*) FROM mediaProgresses mp, bookSeries bs WHERE bs.seriesId = series.id AND mp.mediaItemId = bs.bookId AND mp.userId = ? AND mp.isFinished = 1) >= 1 \
         AND ({not_finished}) >= 1 \
         AND (SELECT count(*) FROM mediaProgresses mp, bookSeries bs WHERE mp.mediaItemId = bs.bookId AND mp.userId = ? AND bs.seriesId = series.id AND mp.isFinished = 0 AND mp.currentTime > 0) = 0"
    );
    let mut params = vec![json!(library_id)];
    params.extend(hidden);
    params.push(user_id.clone());
    params.extend(not_finished_params);
    params.push(user_id.clone());
    let count = db
        .one(&format!("SELECT count(DISTINCT series.id) AS n FROM series WHERE {wheres}"), params.clone())?
        .map(|r| r.i("n"))
        .unwrap_or(0);
    let mut select_params = vec![user_id.clone(), user_id.clone()];
    select_params.extend(params);
    let rows = db.query(
        &format!(
            "SELECT series.id AS id, series.name AS name, \
             (SELECT max(mp.updatedAt) FROM bookSeries bs, mediaProgresses mp WHERE mp.mediaItemId = bs.bookId AND mp.userId = ? AND bs.seriesId = series.id) AS recent_progress, \
             {max_sequence} AS maxSequence FROM series WHERE {wheres} ORDER BY recent_progress DESC, series.id{}",
            limit_sql(limit)
        ),
        select_params,
    )?;
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let mut out = Vec::new();
    for r in rows {
        let series_id = r.s("id");
        let mut p = vec![user_id.clone(), json!(series_id)];
        p.extend(perm_params.clone());
        let books = db.query(
            &format!(
                "SELECT bs.bookId AS bookId, bs.sequence AS sequence FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId \
                 LEFT OUTER JOIN mediaProgresses mp ON mp.mediaItemId = book.id AND mp.userId = ? \
                 WHERE bs.seriesId = ? AND (mp.isFinished IS NULL OR mp.isFinished = 0){perm_sql} ORDER BY CAST(bs.sequence AS FLOAT) ASC NULLS LAST"
            ),
            p,
        )?;
        let chosen = if later_only {
            let max = r.get("maxSequence").and_then(Value::as_f64);
            books.iter().find(|b| {
                let seq = b.get("sequence").and_then(Value::as_str).map(crate::abs::scanner::audio::parse_float).filter(|f| !f.is_nan());
                matches!((seq, max), (Some(s), Some(m)) if s > m)
            })
        } else {
            books.first()
        };
        let Some(chosen) = chosen else { continue };
        let Some((item, mut b)) = item_with_book(db, &chosen.s("bookId"))? else { continue };
        b.series.clear();
        let mut j = library_item::minified(&item, &b, sorting);
        if let Some(meta) = j.get_mut("media").and_then(|m| m.get_mut("metadata")).and_then(Value::as_object_mut) {
            meta.insert(
                "series".into(),
                json!({"id": series_id, "name": r.s("name"), "sequence": chosen.get("sequence").cloned().unwrap_or(Value::Null)}),
            );
        }
        out.push(Value::Object(j));
    }
    Ok((out, count))
}

/// A series as the apps get it (`Series.toOldJSON`).
pub fn series_json(se: &Series, sorting: &Sorting) -> Map<String, Value> {
    let name = se.name.clone().unwrap_or_default();
    let mut m = Map::new();
    m.insert("id".into(), json!(se.id));
    m.insert("name".into(), json!(se.name));
    m.insert("nameIgnorePrefix".into(), json!(title_prefix_at_end(&name, sorting)));
    m.insert("description".into(), json!(se.description));
    m.insert("addedAt".into(), super::util::ms0(&se.created_at));
    m.insert("updatedAt".into(), super::util::ms0(&se.updated_at));
    m.insert("libraryId".into(), json!(se.library_id));
    m
}

/// The "Recent Series" shelf (`getSeriesMostRecentlyAdded`): series added in the last 60 days, newest first,
/// each with its books in order.
fn recent_series(db: &dyn Db, library_id: &str, u: &User, hide_single: bool, limit: i64, sorting: &Sorting) -> Result<(Vec<Value>, i64)> {
    let (perm, perm_params) = permission_conditions(u, "b");
    let mut wheres = vec!["series.libraryId = ?".to_string(), "series.createdAt >= ?".to_string()];
    let mut params = vec![json!(library_id), json!(date_text(now_ms() - RECENT_MS))];
    if hide_single {
        wheres.push("(SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id) > 1".into());
    }
    if !perm.is_empty() {
        let conds: String = perm.iter().map(|c| format!(" AND {c}")).collect();
        wheres.push(format!("(SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id{conds}) > 0"));
        params.extend(perm_params);
    }
    let where_sql = wheres.join(" AND ");
    let count = db
        .one(&format!("SELECT count(DISTINCT series.id) AS n FROM series WHERE {where_sql}"), params.clone())?
        .map(|r| r.i("n"))
        .unwrap_or(0);
    let rows =
        db.query(&format!("SELECT * FROM series WHERE {where_sql} ORDER BY series.createdAt DESC, series.id{}", limit_sql(limit)), params)?;
    let (book_perm, book_perm_params) = permission_conditions(u, "book");
    let book_perm_sql: String = book_perm.iter().map(|c| format!(" AND {c}")).collect();
    let mut out = Vec::new();
    for row in rows {
        let se: Series = from_row(row)?;
        let mut p = vec![json!(se.id)];
        p.extend(book_perm_params.clone());
        let links = db.query(
            &format!(
                "SELECT bs.bookId AS bookId, bs.sequence AS sequence FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId \
                 WHERE bs.seriesId = ?{book_perm_sql} ORDER BY bs.rowid"
            ),
            p,
        )?;
        let mut links: Vec<(String, Option<String>)> = links
            .iter()
            .map(|l| (l.s("bookId"), l.get("sequence").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)))
            .collect();
        links.sort_by(|a, b| match (&a.1, &b.1) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, _) => std::cmp::Ordering::Greater,
            (_, None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => natural_cmp(x, y),
        });
        let mut books = Vec::new();
        for (book_id, _) in links {
            let Some((item, mut b)) = item_with_book(db, &book_id)? else { continue };
            b.authors.clear();
            b.series.clear();
            books.push(Value::Object(library_item::minified(&item, &b, sorting)));
        }
        let mut j = series_json(&se, sorting);
        j.insert("books".into(), Value::Array(books));
        out.push(Value::Object(j));
    }
    Ok((out, count))
}

/// The "Discover" shelf (`getDiscoverLibraryItems`): books the user hasn't started, in random order: books in no
/// series, and the first book of randomly picked series nobody of the user's has started.
fn discover(db: &dyn Db, library_id: &str, u: &User, limit: i64, sorting: &Sorting) -> Result<(Vec<Value>, i64)> {
    let user_id = json!(u.id.clone().unwrap_or_default());
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let series = db.query(
        &format!(
            "SELECT series.id AS id FROM series WHERE series.libraryId = ? AND (SELECT count(*) FROM bookSeries bs LEFT OUTER JOIN mediaProgresses mp ON mp.mediaItemId = bs.bookId \
             WHERE bs.seriesId = series.id AND mp.userId = ? AND (mp.isFinished = 1 OR mp.currentTime > 0)) = 0 \
             AND EXISTS (SELECT 1 FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId WHERE bs.seriesId = series.id{perm_sql}) \
             ORDER BY RANDOM(){}",
            limit_sql(limit)
        ),
        [vec![json!(library_id), user_id.clone()], perm_params.clone()].concat(),
    )?;
    let mut first_books = Vec::new();
    for s in series {
        let mut p = vec![json!(s.s("id"))];
        p.extend(perm_params.clone());
        let first = db.one(
            &format!(
                "SELECT bs.bookId AS bookId FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId WHERE bs.seriesId = ?{perm_sql} \
                 ORDER BY CAST(bs.sequence AS FLOAT) ASC NULLS LAST LIMIT 1"
            ),
            p,
        )?;
        if let Some(f) = first {
            first_books.push(json!(f.s("bookId")));
        }
    }
    let in_list =
        if first_books.is_empty() { "0".to_string() } else { format!("book.id IN ({})", vec!["?"; first_books.len()].join(", ")) };
    let from = format!(
        "FROM books book INNER JOIN libraryItems libraryItem ON libraryItem.mediaId = book.id AND libraryItem.libraryId = ? \
         LEFT OUTER JOIN mediaProgresses mp ON mp.mediaItemId = book.id AND mp.userId = ? \
         WHERE (mp.isFinished IS NULL OR mp.isFinished = 0) AND (mp.currentTime IS NULL OR mp.currentTime = 0) \
         AND ((SELECT COUNT(*) FROM bookSeries bs WHERE bs.bookId = book.id) = 0 OR {in_list}){perm_sql}"
    );
    let params = [vec![json!(library_id), user_id], first_books, perm_params].concat();
    let count = db.one(&format!("SELECT count(DISTINCT book.id) AS n {from}"), params.clone())?.map(|r| r.i("n")).unwrap_or(0);
    let picked = db.query(&format!("SELECT DISTINCT book.id AS id {from} ORDER BY RANDOM(){}", limit_sql(limit)), params)?;
    let mut out = Vec::new();
    for p in picked {
        if let Some((item, b)) = item_with_book(db, &p.s("id"))? {
            out.push(Value::Object(library_item::minified(&item, &b, sorting)));
        }
    }
    Ok((out, count))
}

/// The "Newest Authors" shelf (`getNewestAuthors`): authors added in the last 60 days with at least one book the
/// user may see, newest first, each with that number of books.
fn newest_authors(db: &dyn Db, library_id: &str, u: &User, limit: i64) -> Result<(Vec<Value>, i64)> {
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let num_books = format!("(SELECT count(DISTINCT book.id) FROM bookAuthors ba INNER JOIN books book ON book.id = ba.bookId WHERE ba.authorId = author.id{perm_sql})");
    let where_sql = format!("author.libraryId = ? AND author.createdAt >= ? AND {num_books} > 0");
    let where_params = [vec![json!(library_id), json!(date_text(now_ms() - RECENT_MS))], perm_params.clone()].concat();
    let count = db
        .one(&format!("SELECT count(DISTINCT author.id) AS n FROM authors author WHERE {where_sql}"), where_params.clone())?
        .map(|r| r.i("n"))
        .unwrap_or(0);
    let params = [perm_params, where_params].concat();
    let rows = db.query(
        &format!(
            "SELECT author.*, {num_books} AS numBooks FROM authors author WHERE {where_sql} ORDER BY author.createdAt DESC, author.id{}",
            limit_sql(limit)
        ),
        params,
    )?;
    let mut out = Vec::new();
    for mut row in rows {
        let n = row.remove("numBooks").and_then(|v| v.as_i64()).unwrap_or(0);
        let a: Author = from_row(row)?;
        let mut j = crate::abs::scanner::author_json(&a);
        j["numBooks"] = json!(n);
        out.push(j);
    }
    Ok((out, count))
}
