//! Readarr's authors (`Readarr.Api.V1/Author`): an author with its metadata, statistics, next and last book and
//! root folder, as `GET /api/v1/author` and `GET /api/v1/author/{id}` send them.
//!
//! ```text
//! GET /api/v1/author        every author, in the order they were added
//! GET /api/v1/author/{id}   one author; ASP.NET's "Not Found" when there is none
//! POST /api/v1/author       add an author (see [`super::add`])
//! PUT /api/v1/author/{id}   change an author's folder, profiles, tags and monitoring (202 with the author)
//! PUT /api/v1/author/editor change several authors at once (202 with them)
//! ```
//!
//! Not yet ported: moving an author's files (`moveFiles`, a command) and the refresh Readarr starts when the
//! metadata profile changes (its metadata server is gone; commands come with R5).

use super::json::{date, date_or_min, doc, list, local_covers, now_param, object, percent, ratings, Query};
use super::{Answer, Fail, Key};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The author routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/author", get(all).put(update_unrouted).post(super::add::add_author))
        .route("/api/v1/author/{id}", get(one).put(update).delete(remove))
        .route("/api/v1/author/editor", put(editor).delete(remove_many))
}

/// An author's columns and its metadata's, as one row (`Id` is the author's).
pub const SELECT: &str = "SELECT a.Id, a.CleanName, a.Path, a.Monitored, a.LastInfoSync, a.QualityProfileId, a.Tags, a.Added,
    a.AddOptions, a.MetadataProfileId, a.AuthorMetadataId, a.MonitorNewItems, m.ForeignAuthorId, m.TitleSlug, m.Name,
    m.Overview, m.Disambiguation, m.Status, m.Images, m.Links, m.Genres, m.Ratings, m.Aliases, m.SortName,
    m.NameLastFirst, m.SortNameLastFirst
    FROM Authors a LEFT JOIN AuthorMetadata m ON m.Id = a.AuthorMetadataId";

/// The authors matching `filter` (SQL after `WHERE`, or empty for all), in the order they were added.
pub fn rows(db: &SqliteDb, filter: &str, params: Vec<Value>) -> kit_server_core::Result<Vec<Row>> {
    let sql = if filter.is_empty() { format!("{SELECT} ORDER BY a.Id") } else { format!("{SELECT} WHERE {filter} ORDER BY a.Id") };
    db.query(&sql, params)
}

/// The author `id`, if there is one.
pub fn find(db: &SqliteDb, id: i64) -> kit_server_core::Result<Option<Row>> {
    Ok(rows(db, "a.Id = ?", vec![json!(id)])?.into_iter().next())
}

/// The author `id`, or Readarr's `ModelNotFoundException`.
pub fn get_or_404(db: &SqliteDb, id: i64) -> Answer<Row> {
    find(db, id)?.ok_or_else(|| Fail::model_not_found("Author", id))
}

/// `NewItemMonitorTypes` from a request (its name or number); `None` when it is neither.
pub fn parse_monitor_new_items(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().filter(|n| (0..=2).contains(n)),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "all" | "0" => Some(0),
            "none" | "1" => Some(1),
            "new" | "2" => Some(2),
            _ => None,
        },
        _ => None,
    }
}

/// How Readarr names `NewItemMonitorTypes` (stored as a number).
fn monitor_new_items(n: i64) -> &'static str {
    match n {
        1 => "none",
        2 => "new",
        _ => "all",
    }
}

/// An author as `AuthorResource.ToResource` makes it: its covers as stored, empty statistics, no next or last
/// book and no root folder (the controllers add those).
pub fn resource(r: &Row) -> Value {
    let status = r.i("Status");
    let tags = match doc(r.get("Tags")) {
        Value::Array(a) => Value::Array(a),
        _ => json!([]),
    };
    object([
        ("authorMetadataId", json!(r.i("AuthorMetadataId"))),
        ("status", json!(if status == 1 { "ended" } else { "continuing" })),
        ("ended", json!(status == 1)),
        ("authorName", r.get("Name").cloned().unwrap_or(Value::Null)),
        ("authorNameLastFirst", r.get("NameLastFirst").cloned().unwrap_or(Value::Null)),
        ("foreignAuthorId", r.get("ForeignAuthorId").cloned().unwrap_or(Value::Null)),
        ("titleSlug", r.get("TitleSlug").cloned().unwrap_or(Value::Null)),
        ("overview", r.get("Overview").cloned().unwrap_or(Value::Null)),
        ("disambiguation", r.get("Disambiguation").cloned().unwrap_or(Value::Null)),
        ("links", list(r.get("Links"))),
        ("images", Value::Array(super::json::images(r.get("Images")))),
        ("path", r.get("Path").cloned().unwrap_or(Value::Null)),
        ("qualityProfileId", json!(r.i("QualityProfileId"))),
        ("metadataProfileId", json!(r.i("MetadataProfileId"))),
        ("monitored", json!(r.i("Monitored") != 0)),
        ("monitorNewItems", json!(monitor_new_items(r.i("MonitorNewItems")))),
        ("genres", list(r.get("Genres"))),
        ("cleanName", r.get("CleanName").cloned().unwrap_or(Value::Null)),
        ("sortName", r.get("SortName").cloned().unwrap_or(Value::Null)),
        ("sortNameLastFirst", r.get("SortNameLastFirst").cloned().unwrap_or(Value::Null)),
        ("tags", tags),
        ("added", date_or_min(r.get("Added"))),
        ("addOptions", doc(r.get("AddOptions"))),
        ("ratings", ratings(r.get("Ratings"))),
        ("statistics", stats_json(&Stats::default())),
        ("id", json!(r.i("Id"))),
    ])
}

/// One book's numbers for the statistics (`BookStatistics`): only its monitored edition counts.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    /// the author's id
    pub author_id: i64,
    /// the book's id
    pub book_id: i64,
    /// the bytes of its files
    pub size_on_disk: i64,
    /// always 1 per book
    pub total_book_count: i64,
    /// 1 when it has a file
    pub available_book_count: i64,
    /// 1 when it is wanted now (monitored and out) or has a file
    pub book_count: i64,
    /// how many files it has
    pub book_file_count: i64,
}

impl Stats {
    /// The sum of several books' numbers (an author's).
    fn sum(list: &[Stats]) -> Stats {
        let mut s = Stats { author_id: list.first().map(|b| b.author_id).unwrap_or(0), ..Stats::default() };
        for b in list {
            s.size_on_disk += b.size_on_disk;
            s.total_book_count += b.total_book_count;
            s.available_book_count += b.available_book_count;
            s.book_count += b.book_count;
            s.book_file_count += b.book_file_count;
        }
        s
    }
}

/// Every book's statistics (`AuthorStatisticsRepository`), of one author or of all.
pub fn book_stats(db: &SqliteDb, author: Option<i64>) -> kit_server_core::Result<Vec<Stats>> {
    let filter = if author.is_some() { "AND a.Id = ?" } else { "" };
    let sql = format!(
        "SELECT a.Id AS AuthorId, b.Id AS BookId, SUM(COALESCE(f.Size, 0)) AS SizeOnDisk, 1 AS TotalBookCount,
         CASE WHEN MIN(f.Id) IS NULL THEN 0 ELSE 1 END AS AvailableBookCount,
         CASE WHEN (b.Monitored = 1 AND (b.ReleaseDate < ?) OR b.ReleaseDate IS NULL) OR MIN(f.Id) IS NOT NULL THEN 1 ELSE 0 END AS BookCount,
         CASE WHEN MIN(f.Id) IS NULL THEN 0 ELSE COUNT(f.Id) END AS BookFileCount
         FROM Editions e JOIN Books b ON e.BookId = b.Id JOIN Authors a ON b.AuthorMetadataId = a.AuthorMetadataId
         LEFT JOIN BookFiles f ON e.Id = f.EditionId
         WHERE e.Monitored = 1 {filter} GROUP BY a.Id, b.Id"
    );
    let mut params = vec![json!(now_param())];
    if let Some(id) = author {
        params.push(json!(id));
    }
    Ok(db
        .query(&sql, params)?
        .iter()
        .map(|r| Stats {
            author_id: r.i("AuthorId"),
            book_id: r.i("BookId"),
            size_on_disk: r.i("SizeOnDisk"),
            total_book_count: r.i("TotalBookCount"),
            available_book_count: r.i("AvailableBookCount"),
            book_count: r.i("BookCount"),
            book_file_count: r.i("BookFileCount"),
        })
        .collect())
}

/// An author's statistics as the API sends them (`AuthorStatisticsResource`).
fn stats_json(s: &Stats) -> Value {
    json!({
        "bookFileCount": s.book_file_count,
        "bookCount": s.book_count,
        "availableBookCount": s.available_book_count,
        "totalBookCount": s.total_book_count,
        "sizeOnDisk": s.size_on_disk,
        "percentOfBooks": percent(s.available_book_count, s.book_count),
    })
}

/// A book as the plain model Readarr puts in `nextBook` and `lastBook`: its stored fields, with the lazy-loaded
/// ones (author, editions, files, series) as nulls.
fn plain_book(r: &Row) -> Value {
    let mut v = object([
        ("authorMetadataId", json!(r.i("AuthorMetadataId"))),
        ("foreignBookId", r.get("ForeignBookId").cloned().unwrap_or(Value::Null)),
        ("titleSlug", r.get("TitleSlug").cloned().unwrap_or(Value::Null)),
        ("title", r.get("Title").cloned().unwrap_or(Value::Null)),
        ("releaseDate", date(r.get("ReleaseDate"))),
        ("links", list(r.get("Links"))),
        ("genres", list(r.get("Genres"))),
        ("relatedBooks", list(r.get("RelatedBooks"))),
        ("ratings", ratings(r.get("Ratings"))),
        ("lastSearchTime", date(r.get("LastSearchTime"))),
        ("cleanTitle", r.get("CleanTitle").cloned().unwrap_or(Value::Null)),
        ("monitored", json!(r.i("Monitored") != 0)),
        ("anyEditionOk", json!(r.i("AnyEditionOk") != 0)),
        ("lastInfoSync", date(r.get("LastInfoSync"))),
        ("added", date_or_min(r.get("Added"))),
        ("addOptions", super::books::add_options(r.get("AddOptions"))),
        ("id", json!(r.i("Id"))),
    ]);
    for k in ["authorMetadata", "author", "editions", "bookFiles", "seriesLinks"] {
        v[k] = Value::Null;
    }
    v
}

/// Each author's next book (the soonest one not out yet) or last book (the latest one out), by metadata id.
/// Like Readarr, a book only counts when it is also the author's lowest book id among those books.
fn next_or_last(db: &SqliteDb, next: bool) -> kit_server_core::Result<HashMap<i64, Value>> {
    let (pick, cmp) = if next { ("MIN", ">") } else { ("MAX", "<") };
    let sql = format!(
        "SELECT b.* FROM Books b JOIN (SELECT MIN(Id) AS id, {pick}(ReleaseDate) AS date FROM Books
         WHERE ReleaseDate {cmp} ? GROUP BY AuthorMetadataId) ids ON ids.id = b.Id AND ids.date = b.ReleaseDate"
    );
    Ok(db.query(&sql, vec![json!(now_param())])?.iter().map(|r| (r.i("AuthorMetadataId"), plain_book(r))).collect())
}

/// The root folder an author's path is in (the longest one), or the path's parent folder when none is.
fn root_folder_path(path: &str, roots: &[String]) -> String {
    let trimmed = |p: &str| p.trim_end_matches('/').to_string();
    let best = roots
        .iter()
        .filter(|r| {
            let r = trimmed(r);
            path == r || path.trim_end_matches('/') == r || path.starts_with(&format!("{r}/"))
        })
        .max_by_key(|r| r.len());
    match best {
        Some(r) => r.clone(),
        None => match path.trim_end_matches('/').rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => path[..i].to_string(),
            None => String::new(),
        },
    }
}

/// Authors as `GET /api/v1/author` sends them: covers as Folio's copies, next and last book, statistics (when
/// the author has any) and root folder.
pub fn full(db: &SqliteDb, authors: &[Row], one: bool) -> kit_server_core::Result<Vec<Value>> {
    let next = next_or_last(db, true)?;
    let last = next_or_last(db, false)?;
    let stats = book_stats(db, if one { authors.first().map(|a| a.i("Id")) } else { None })?;
    let mut by_author: HashMap<i64, Vec<Stats>> = HashMap::new();
    for s in stats {
        by_author.entry(s.author_id).or_default().push(s);
    }
    let roots: Vec<String> = db.query("SELECT Path FROM RootFolders ORDER BY Id", vec![])?.iter().map(|r| r.s("Path")).collect();
    Ok(authors
        .iter()
        .map(|a| {
            let id = a.i("Id");
            let meta = a.i("AuthorMetadataId");
            let mut v = resource(a);
            let covers = local_covers(super::json::images(a.get("Images")), "author", id);
            v["images"] = Value::Array(covers);
            if let Some(b) = next.get(&meta) {
                v["nextBook"] = b.clone();
            }
            if let Some(b) = last.get(&meta) {
                v["lastBook"] = b.clone();
            }
            if let Some(list) = by_author.get(&id) {
                v["statistics"] = stats_json(&Stats::sum(list));
            }
            v["rootFolderPath"] = json!(root_folder_path(&a.s("Path"), &roots));
            v
        })
        .collect())
}

/// `GET /api/v1/author`.
async fn all(State(st): State<AppState>, _: Key) -> Answer {
    let authors = rows(&st.readarr, "", vec![])?;
    Ok(Json(Value::Array(full(&st.readarr, &authors, false)?)))
}

/// `GET /api/v1/author/{id}`.
async fn one(State(st): State<AppState>, _: Key, Path(id): Path<String>) -> Answer {
    let id: i64 = id.parse().map_err(|_| Fail::not_found())?;
    let Some(a) = find(&st.readarr, id)? else { return Err(Fail::not_found()) };
    let mut list = full(&st.readarr, &[a], true)?;
    Ok(Json(list.remove(0)))
}

/// A number from a request body (0 when it is missing or not one, as .NET binds it).
fn int(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// `PUT /api/v1/author` (the id from the body).
async fn update_unrouted(st: State<AppState>, key: Key, raw: axum::extract::RawQuery, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    put_author(st, key, 0, raw, bytes).await
}

/// `PUT /api/v1/author/{id}`.
async fn update(
    st: State<AppState>,
    key: Key,
    Path(id): Path<i64>,
    raw: axum::extract::RawQuery,
    bytes: Bytes,
) -> Answer<(StatusCode, Json<Value>)> {
    put_author(st, key, id, raw, bytes).await
}

/// Changes an author like `AuthorController.UpdateAuthor`: the request's checks, then its folder, profiles,
/// tags and monitoring (never its add options); 202 with the author as `GET` sends it.
async fn put_author(
    State(st): State<AppState>,
    _: Key,
    route_id: i64,
    _q: axum::extract::RawQuery,
    bytes: Bytes,
) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    let id = match int(&r, "id") {
        0 => route_id,
        id => id,
    };
    let path = r.get("path").and_then(Value::as_str);
    let mut f = super::validation::Failures::default();
    f.valid_id("QualityProfileId", int(&r, "qualityProfileId"));
    f.valid_id("MetadataProfileId", int(&r, "metadataProfileId"));
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        super::validation::author_path(db, &mut f, id, p)?;
    }
    super::validation::profile_exists(db, &mut f, "QualityProfileId", int(&r, "qualityProfileId"))?;
    super::validation::profile_exists(db, &mut f, "MetadataProfileId", int(&r, "metadataProfileId"))?;
    f.valid_id("Id", id);
    super::validation::valid_path(&mut f, "Path", path);
    f.check()?;
    get_or_404(db, id)?;
    let tags = super::json::stored_doc(r.get("tags"));
    let monitor_new = r.get("monitorNewItems").and_then(parse_monitor_new_items).unwrap_or(0);
    db.execute(
        "UPDATE Authors SET Path = ?, QualityProfileId = ?, MetadataProfileId = ?, Tags = ?, Monitored = ?, MonitorNewItems = ? WHERE Id = ?",
        vec![
            json!(path),
            json!(int(&r, "qualityProfileId")),
            json!(int(&r, "metadataProfileId")),
            tags,
            json!(r.get("monitored").and_then(Value::as_bool).unwrap_or(false)),
            json!(monitor_new),
            json!(id),
        ],
    )?;
    let a = get_or_404(db, id)?;
    let mut list = full(db, &[a], true)?;
    Ok((StatusCode::ACCEPTED, Json(list.remove(0))))
}

/// The ids in a request body's list `k`.
pub fn ids(v: &Value, k: &str) -> Vec<i64> {
    v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
}

/// The rows found for `wanted` ids, or Readarr's `Expected query to return …` when one is missing.
pub fn get_many(rows: Vec<Row>, wanted: usize) -> Answer<Vec<Row>> {
    if rows.len() != wanted {
        return Err(Fail::exception(
            500,
            "System.ApplicationException",
            format!("Expected query to return {wanted} rows but returned {}", rows.len()),
        ));
    }
    Ok(rows)
}

/// `PUT /api/v1/author/editor` (`AuthorEditorController.SaveAll`): monitoring, new-item monitoring, profiles,
/// root folder (the author's folder keeps its name) and tags (`applyTags`: add, remove or replace) of several
/// authors; 202 with them as `ToResource` makes them.
async fn editor(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    let wanted = ids(&r, "authorIds");
    let found = if wanted.is_empty() {
        Vec::new()
    } else {
        rows(db, &format!("a.Id IN ({})", super::json::marks(wanted.len())), wanted.iter().map(|i| json!(i)).collect())?
    };
    let found = get_many(found, wanted.len())?;
    let apply = r.get("applyTags").map(|v| match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "remove" | "1" => 1,
            "replace" | "2" => 2,
            _ => 0,
        },
        _ => 0,
    });
    let root = r.get("rootFolderPath").and_then(Value::as_str).filter(|p| !p.trim().is_empty());
    for a in &found {
        let mut monitored = a.i("Monitored") != 0;
        let mut monitor_new = a.i("MonitorNewItems");
        let mut quality = a.i("QualityProfileId");
        let mut metadata = a.i("MetadataProfileId");
        let mut path = a.s("Path");
        if let Some(m) = r.get("monitored").and_then(Value::as_bool) {
            monitored = m;
        }
        if let Some(m) = r.get("monitorNewItems").and_then(parse_monitor_new_items) {
            monitor_new = m;
        }
        if let Some(q) = r.get("qualityProfileId").and_then(Value::as_i64) {
            quality = q;
        }
        if let Some(m) = r.get("metadataProfileId").and_then(Value::as_i64) {
            metadata = m;
        }
        if let Some(root) = root {
            let folder = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string();
            path = format!("{}/{folder}", root.trim_end_matches('/'));
        }
        let mut tags: Vec<i64> = match doc(a.get("Tags")) {
            Value::Array(t) => t.iter().filter_map(Value::as_i64).collect(),
            _ => Vec::new(),
        };
        if let Some(new) = r.get("tags").and_then(Value::as_array) {
            let new: Vec<i64> = new.iter().filter_map(Value::as_i64).collect();
            match apply.unwrap_or(0) {
                1 => tags.retain(|t| !new.contains(t)),
                2 => tags = dedup(new),
                _ => {
                    for t in new {
                        if !tags.contains(&t) {
                            tags.push(t);
                        }
                    }
                }
            }
        }
        db.execute(
            "UPDATE Authors SET Monitored = ?, MonitorNewItems = ?, QualityProfileId = ?, MetadataProfileId = ?, Path = ?, Tags = ? WHERE Id = ?",
            vec![json!(monitored), json!(monitor_new), json!(quality), json!(metadata), json!(path), json!(json!(tags).to_string()), json!(a.i("Id"))],
        )?;
    }
    let mut out = Vec::new();
    for a in &found {
        if let Some(row) = find(db, a.i("Id"))? {
            out.push(resource(&row));
        }
    }
    Ok((StatusCode::ACCEPTED, Json(Value::Array(out))))
}

/// The numbers once each, in their first order (a `HashSet<int>` of a few small numbers keeps that order).
fn dedup(list: Vec<i64>) -> Vec<i64> {
    let mut out = Vec::new();
    for t in list {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// `DELETE /api/v1/author/{id}?deleteFiles=&addImportListExclusion=`: 200 with no body.
async fn remove(State(st): State<AppState>, _: Key, Path(id): Path<String>, RawQuery(q): RawQuery) -> Answer<StatusCode> {
    let id: i64 = id.parse().map_err(|_| Fail::not_found())?;
    let q = Query::of(q.as_deref());
    let (files, exclude) = (q.bool("deleteFiles").unwrap_or(false), q.bool("addImportListExclusion").unwrap_or(false));
    super::delete::author(&st.readarr, &st.readarr_disk, id, files, exclude)?;
    Ok(StatusCode::OK)
}

/// `DELETE /api/v1/author/editor` (`AuthorEditorController.DeleteAuthor`): deletes each author asked for, keeping
/// their files, until one is missing; `{}`.
async fn remove_many(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer {
    let r = super::body(&bytes)?;
    for id in ids(&r, "authorIds") {
        super::delete::author(&st.readarr, &st.readarr_disk, id, false, false)?;
    }
    Ok(Json(json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_author_path_finds_its_root_folder() {
        let roots = vec!["/books/".to_string(), "/books/audio/".to_string()];
        assert_eq!(root_folder_path("/books/Terry Pratchett", &roots), "/books/");
        assert_eq!(root_folder_path("/books/audio/Someone", &roots), "/books/audio/");
        assert_eq!(root_folder_path("/elsewhere/Someone", &roots), "/elsewhere");
    }
}
