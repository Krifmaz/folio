//! Series (PARITY A0009, A0010 and A0122, track C): a library's series page (`seriesFilters.getFilteredSeries`),
//! one series of a library (`LibraryController.getSeriesForLibrary`) and one series by id
//! (`SeriesController.findOne`), with the series JSON the original sends (`Series.toOldJSON`).

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::filters::{self, permission_conditions};
use crate::abs::library::util::{decode_filter, js_num, ms0, natural_cmp, title_prefix_at_end, Sorting};
use crate::abs::library::{self as lib};
use crate::abs::models::{self, book, from_row, library_item, LibraryItem, Series, User};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The series routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/libraries/{id}/series", get(library_series))
        .route("/api/libraries/{id}/series/{series_id}", get(library_one_series))
        .route("/api/series/{id}", get(find_one))
}

/// A series as the apps get it (`Series.toOldJSON`).
pub fn series_json(se: &Series, sorting: &Sorting) -> Map<String, Value> {
    let name = se.name.clone().unwrap_or_default();
    let mut m = Map::new();
    m.insert("id".into(), json!(se.id));
    m.insert("name".into(), json!(se.name));
    m.insert("nameIgnorePrefix".into(), json!(title_prefix_at_end(&name, sorting)));
    m.insert("description".into(), json!(se.description));
    m.insert("addedAt".into(), ms0(&se.created_at));
    m.insert("updatedAt".into(), ms0(&se.updated_at));
    m.insert("libraryId".into(), json!(se.library_id));
    m
}

/// A series' book links in the original's order: by number in the series (as text compared the natural way),
/// books without a number last (the `bookSeries.sort` of the original).
pub fn sort_by_sequence<T>(links: &mut [(T, Option<String>)]) {
    links.sort_by(|a, b| match (&a.1, &b.1) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, _) => std::cmp::Ordering::Greater,
        (_, None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => natural_cmp(x, y),
    });
}

/// The books of a series a user may see, as `(book id, sequence)` in link order, joined to `books` as `book`.
pub fn book_links(db: &dyn Db, series_id: &str, u: &User) -> Result<Vec<(String, Option<String>)>> {
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let rows = db.query(
        &format!(
            "SELECT bs.bookId AS bookId, bs.sequence AS sequence FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId \
             WHERE bs.seriesId = ?{perm_sql} ORDER BY bs.rowid"
        ),
        [vec![json!(series_id)], perm_params].concat(),
    )?;
    Ok(rows
        .iter()
        .map(|l| (l.s("bookId"), l.get("sequence").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)))
        .collect())
}

/// The item of a book with its authors and series, or `None` when it has no item.
pub fn item_of_book(db: &dyn Db, book_id: &str) -> Result<Option<(LibraryItem, book::BookFull)>> {
    let Some(row) = db.one("SELECT * FROM libraryItems WHERE mediaId = ? LIMIT 1", vec![json!(book_id)])? else { return Ok(None) };
    let item: LibraryItem = from_row(row)?;
    let mut books = book::load(db, &[book_id.to_string()])?;
    Ok(books.remove(book_id).map(|b| (item, b)))
}

/// A series page's `filter` split into its group and decoded value (`getFilteredSeries`).
fn split_filter(filter_by: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(f) = filter_by.filter(|f| !f.is_empty()) else { return (None, None) };
    let groups = ["genres", "tags", "authors", "progress", "narrators", "publishers", "languages"];
    match groups.iter().find(|g| f.starts_with(&format!("{g}."))) {
        Some(g) => (Some((*g).to_string()), decode_filter(&f.replacen(&format!("{g}."), "", 1))),
        None => (Some(f.to_string()), None),
    }
}

/// What a series page asks for.
struct SeriesQuery {
    /// The filter group and value.
    filter: (Option<String>, Option<String>),
    /// The sort (`numBooks`, `addedAt`, `name`, `totalDuration`, `lastBookAdded`, `lastBookUpdated`, `random`).
    sort_by: Option<String>,
    /// Whether the sort is reversed.
    desc: bool,
    /// Series per page, 0 for all.
    limit: i64,
    /// Series skipped.
    offset: i64,
}

/// The series of a book library a page shows, with their books, and how many there are in all.
fn filtered_series(db: &dyn Db, library: &models::Library, u: &User, q: &SeriesQuery, sorting: &Sorting) -> Result<(Vec<Value>, i64)> {
    let library_id = library.id.clone().unwrap_or_default();
    let settings = library.settings.clone().unwrap_or(Value::Null);
    let mut wheres = vec!["series.libraryId = ?".to_string()];
    let mut params = vec![json!(library_id)];
    if settings.get("hideSingleBookSeries").is_some_and(book::truthy) {
        wheres.push("(SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id) > 1".into());
    }
    let user_id = json!(u.id.clone().unwrap_or_default());
    let (group, value) = (q.filter.0.as_deref().unwrap_or_default(), q.filter.1.clone());
    let mut attr: Option<String> = None;
    let mut attr_params: Vec<Value> = Vec::new();
    let progress =
        "SELECT count(*) FROM books b, bookSeries bs LEFT OUTER JOIN mediaProgresses mp ON mp.mediaItemId = b.id AND mp.userId = ? \
                    WHERE bs.seriesId = series.id AND bs.bookId = b.id AND ";
    match group {
        "genres" | "tags" | "narrators" => {
            attr = Some(format!(
                "SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id AND \
                 (SELECT count(*) FROM json_each(b.{group}) WHERE json_valid(b.{group}) AND json_each.value = ?) > 0"
            ));
            attr_params.push(json!(value));
        }
        "authors" => {
            attr = Some("SELECT count(*) FROM books b, bookSeries bs, bookAuthors ba WHERE bs.seriesId = series.id AND bs.bookId = b.id AND ba.bookId = b.id AND ba.authorId = ?".into());
            attr_params.push(json!(value));
        }
        "publishers" | "languages" => {
            let col = if group == "publishers" { "publisher" } else { "language" };
            attr = Some(format!(
                "SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id AND b.{col} = ?"
            ));
            attr_params.push(json!(value));
        }
        "progress" => match value.as_deref() {
            Some("not-finished") => {
                attr = Some(format!("{progress}(mp.isFinished IS NULL OR mp.isFinished = 0)"));
                attr_params.push(user_id.clone());
            }
            Some("finished") => {
                wheres.push(format!("({progress}(mp.isFinished IS NULL OR mp.isFinished = 0)) = 0"));
                params.push(user_id.clone());
            }
            Some("not-started") => {
                wheres.push(format!("({progress}(mp.isFinished = 1 OR mp.currentTime > 0)) = 0"));
                params.push(user_id.clone());
            }
            Some("in-progress") => {
                attr = Some(format!("{progress}(mp.currentTime > 0 OR mp.ebookProgress > 0) AND mp.isFinished = 0"));
                attr_params.push(user_id.clone());
            }
            _ => {}
        },
        _ => {}
    }
    let (perm, perm_params) = permission_conditions(u, "b");
    if !perm.is_empty() {
        let base =
            attr.unwrap_or_else(|| "SELECT count(*) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND bs.bookId = b.id".into());
        attr = Some(format!("{base}{}", perm.iter().map(|c| format!(" AND {c}")).collect::<String>()));
        attr_params.extend(perm_params);
    }
    if let Some(a) = attr {
        wheres.push(format!("({a}) > 0"));
        params.extend(attr_params);
    }
    let where_sql = wheres.join(" AND ");

    let dir = if q.desc { "DESC" } else { "ASC" };
    let (extra, order) = match q.sort_by.as_deref().unwrap_or_default() {
        "numBooks" => (", (SELECT count(*) FROM bookSeries bs WHERE bs.seriesId = series.id) AS numBooks", format!("numBooks {dir}")),
        "addedAt" => ("", format!("series.createdAt {dir}")),
        "name" if sorting.ignore_prefix => ("", format!("series.nameIgnorePrefix COLLATE NOCASE {dir}")),
        "name" => ("", format!("`series`.`name` COLLATE NOCASE {dir}")),
        "totalDuration" => (
            ", (SELECT SUM(b.duration) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND b.id = bs.bookId) AS totalDuration",
            format!("totalDuration {dir}"),
        ),
        "lastBookAdded" => (
            ", (SELECT MAX(b.createdAt) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND b.id = bs.bookId) AS mostRecentBookAdded",
            format!("mostRecentBookAdded {dir}"),
        ),
        "lastBookUpdated" => (
            ", (SELECT MAX(b.updatedAt) FROM books b, bookSeries bs WHERE bs.seriesId = series.id AND b.id = bs.bookId) AS mostRecentBookUpdated",
            format!("mostRecentBookUpdated {dir}"),
        ),
        "random" => ("", "RANDOM()".into()),
        _ => ("", String::new()),
    };
    // Series a sort finds equal, and all of them without a sort, come in id order (see docs/DECISIONS.md).
    let order_sql = if order.is_empty() { " ORDER BY series.id".to_string() } else { format!(" ORDER BY {order}, series.id") };
    let limit_sql = match (q.limit, q.offset) {
        (0, 0) => String::new(),
        (0, off) => format!(" LIMIT -1 OFFSET {off}"),
        (lim, off) => format!(" LIMIT {lim} OFFSET {off}"),
    };
    let count = db
        .one(&format!("SELECT count(DISTINCT series.id) AS n FROM series WHERE {where_sql}"), params.clone())?
        .map(|r| r.i("n"))
        .unwrap_or(0);
    let rows = db.query(&format!("SELECT series.*{extra} FROM series WHERE {where_sql}{order_sql}{limit_sql}"), params)?;

    let mut out = Vec::new();
    for mut row in rows {
        let total = row.remove("totalDuration");
        for k in ["numBooks", "mostRecentBookAdded", "mostRecentBookUpdated"] {
            row.remove(k);
        }
        let se: Series = from_row(row)?;
        let mut j = series_json(&se, sorting);
        if let Some(t) = total.and_then(|v| v.as_f64()).filter(|t| *t != 0.0) {
            j.insert("totalDuration".into(), js_num(t));
        }
        let mut links = book_links(db, se.id.as_deref().unwrap_or_default(), u)?;
        sort_by_sequence(&mut links);
        let mut books = Vec::new();
        for (book_id, _) in links {
            if let Some((item, b)) = item_of_book(db, &book_id)? {
                books.push(Value::Object(library_item::minified(&item, &b, sorting)));
            }
        }
        j.insert("books".into(), Value::Array(books));
        out.push(Value::Object(j));
    }
    Ok((out, count))
}

/// `GET /api/libraries/:id/series` (A0009, `getAllSeriesForLibrary`): a page of the library's series with their
/// books.
async fn library_series(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let library = match lib::library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let include: Vec<String> =
        q.get("include").map(|i| i.split(',').map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect()).unwrap_or_default();
    let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(0);
    let page: i64 = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(0);
    let query = SeriesQuery {
        filter: split_filter(q.get("filter").map(String::as_str)),
        sort_by: q.get("sort").cloned(),
        desc: q.get("desc").map(String::as_str) == Some("1"),
        limit,
        offset: page * limit,
    };
    let sorting = Sorting::of(&st.abs_server.settings());
    let (results, total) = if library.media_type.as_deref() == Some("book") {
        match filtered_series(&st.abs, &library, &user, &query, &sorting) {
            Ok(x) => x,
            Err(e) => return e.into_response(),
        }
    } else {
        (Vec::new(), 0)
    };
    let mut payload = Map::new();
    payload.insert("results".into(), Value::Array(results));
    payload.insert("total".into(), json!(total));
    payload.insert("limit".into(), json!(limit));
    payload.insert("page".into(), json!(page));
    if let Some(s) = q.get("sort") {
        payload.insert("sortBy".into(), json!(s));
    }
    payload.insert("sortDesc".into(), json!(query.desc));
    if let Some(f) = q.get("filter") {
        payload.insert("filterBy".into(), json!(f));
    }
    payload.insert("minified".into(), json!(q.get("minified").map(String::as_str) == Some("1")));
    payload.insert("include".into(), json!(include.join(",")));
    Json(Value::Object(payload)).into_response()
}

/// The items of a series a user may see, in series order (`getLibraryItemsForSeries`).
fn items_in_series(db: &dyn Db, se: &Series, u: &User, sorting: &Sorting) -> Result<Vec<filters::PageItem>> {
    let q = filters::Query { group: Some("series".into()), value: se.id.clone(), ..Default::default() };
    Ok(filters::book_items(db, se.library_id.as_deref().unwrap_or_default(), u, &q, sorting)?.0)
}

/// The `progress` a series shows with `include=progress`: its items, the finished ones, and whether all are
/// finished (`all_finished` decides how an empty series counts).
fn series_progress(db: &dyn Db, u: &User, items: &[filters::PageItem], empty_is_finished: bool) -> Result<Value> {
    let progresses = models::user::media_progresses(db, u.id.as_deref().unwrap_or_default())?;
    let finished: Vec<&filters::PageItem> =
        items.iter().filter(|p| progresses.iter().any(|mp| mp.media_item_id == p.book.book.id && mp.is_finished == Some(true))).collect();
    let all = if items.is_empty() { empty_is_finished } else { finished.len() == items.len() };
    Ok(json!({
        "libraryItemIds": items.iter().map(|p| p.item.id.clone()).collect::<Vec<_>>(),
        "libraryItemIdsFinished": finished.iter().map(|p| p.item.id.clone()).collect::<Vec<_>>(),
        "isFinished": all,
    }))
}

/// The series JSON with what `include` asks for: `progress`, and `rssFeed` (feeds come with P5, so it is null).
fn series_with_includes(
    db: &dyn Db,
    se: &Series,
    u: &User,
    include: &[String],
    items: &[filters::PageItem],
    empty_is_finished: bool,
    sorting: &Sorting,
) -> Result<Value> {
    let mut j = series_json(se, sorting);
    if include.iter().any(|i| i == "progress") {
        j.insert("progress".into(), series_progress(db, u, items, empty_is_finished)?);
    }
    if include.iter().any(|i| i == "rssfeed") {
        j.insert("rssFeed".into(), Value::Null);
    }
    Ok(Value::Object(j))
}

/// `GET /api/libraries/:id/series/:seriesId` (A0010, `getSeriesForLibrary`): one series of the library, 404 when
/// it is another library's.
async fn library_one_series(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path((id, series_id)): Path<(String, String)>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let library = match lib::library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let include: Vec<String> =
        q.get("include").map(|i| i.split(',').map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect()).unwrap_or_default();
    let sorting = Sorting::of(&st.abs_server.settings());
    let run = || -> Result<Option<Value>> {
        let Some(se) = models::find::<Series>(&st.abs, &series_id)? else { return Ok(None) };
        if se.library_id != library.id {
            return Ok(None);
        }
        let items = items_in_series(&st.abs, &se, &user, &sorting)?;
        series_with_includes(&st.abs, &se, &user, &include, &items, true, &sorting).map(Some)
    };
    match run() {
        Ok(Some(j)) => Json(j).into_response(),
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/series/:id` (A0122, `SeriesController.findOne`): a series with at least one book the user may see,
/// else 404.
async fn find_one(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let include: Vec<String> =
        q.get("include").map(|i| i.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()).unwrap_or_default();
    let sorting = Sorting::of(&st.abs_server.settings());
    let run = || -> Result<Option<Value>> {
        let Some(se) = models::find::<Series>(&st.abs, &id)? else { return Ok(None) };
        let items = items_in_series(&st.abs, &se, &user, &sorting)?;
        if items.is_empty() {
            return Ok(None);
        }
        series_with_includes(&st.abs, &se, &user, &include, &items, false, &sorting).map(Some)
    };
    match run() {
        Ok(Some(j)) => Json(j).into_response(),
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => e.into_response(),
    }
}
