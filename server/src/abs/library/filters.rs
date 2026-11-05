//! The library page's filters and sorting for book libraries (`server/utils/queries/libraryFilters.js` and
//! `libraryItemsBookFilters.js`): which items a filter like `genres.<base64>` or `progress.<base64>` keeps, the
//! order of each `sort`, collapsed series, and the data of the filter menus.
//!
//! The original builds its queries with Sequelize; here the same SQL is written out against the same tables, so
//! SQLite orders and counts the rows the same way.

use super::util::{decode_filter, natural_cmp, Sorting};
use crate::abs::models::book::{self, BookFull};
use crate::abs::models::user;
use crate::abs::models::{date_text, from_row, now_ms, LibraryItem, User};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The filter groups whose value follows a dot, base64-encoded (`searchGroups`).
const SEARCH_GROUPS: &[&str] = &[
    "genres",
    "tags",
    "series",
    "authors",
    "progress",
    "narrators",
    "publishers",
    "publishedDecades",
    "missing",
    "languages",
    "tracks",
    "ebooks",
];

/// A library page's `filter` split into its group and decoded value (`getFilteredLibraryItems`): `issues`,
/// `feed-open` and the like have no value.
pub fn split_filter(filter_by: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(f) = filter_by.filter(|f| !f.is_empty()) else { return (None, None) };
    match SEARCH_GROUPS.iter().find(|g| f.starts_with(&format!("{g}."))) {
        Some(g) => (Some((*g).to_string()), decode_filter(&f.replacen(&format!("{g}."), "", 1))),
        None => (Some(f.to_string()), None),
    }
}

/// What a library page asks for.
#[derive(Debug, Clone, Default)]
pub struct Query {
    /// The filter group, such as `genres`, `progress` or `issues`.
    pub group: Option<String>,
    /// The filter's decoded value.
    pub value: Option<String>,
    /// The `sort` key, such as `media.metadata.title` or `addedAt`.
    pub sort_by: Option<String>,
    /// Sort from the end.
    pub sort_desc: bool,
    /// Show each series as one entry.
    pub collapse_series: bool,
    /// Extra data wanted (`rssfeed`, `share`).
    pub include: Vec<String>,
    /// How many items at most; 0 for all.
    pub limit: i64,
    /// How many items to skip.
    pub offset: i64,
    /// For a home page shelf: leave out progress hidden from "Continue listening".
    pub home_page: bool,
}

/// A collapsed series an item stands for (`libraryItem.collapsedSeries`).
#[derive(Debug, Clone)]
pub struct CollapsedSeries {
    /// The JSON the page shows.
    pub json: Value,
}

/// One item of a library page, with what the page adds to it.
#[derive(Debug, Clone)]
pub struct PageItem {
    /// The item's row.
    pub item: LibraryItem,
    /// Its book, with authors and series.
    pub book: BookFull,
    /// When filtering by series: the series and this book's number in it.
    pub series: Option<Value>,
    /// When collapsing series: the series this item stands for.
    pub collapsed_series: Option<Value>,
}

/// SQL pieces and their values, kept in order.
#[derive(Default)]
struct Sql {
    joins: Vec<String>,
    item_on: Vec<String>,
    wheres: Vec<String>,
    params_joins: Vec<Value>,
    params_item: Vec<Value>,
    params_where: Vec<Value>,
}

/// The conditions that hide books a user may not see: explicit ones, and ones by tag
/// (`getUserPermissionBookWhereQuery`), on the books table named `book`, with their values.
pub fn permission_conditions(u: &User, book: &str) -> (Vec<String>, Vec<Value>) {
    let mut conds = Vec::new();
    let mut params = Vec::new();
    if !user::can(u, "accessExplicitContent") {
        conds.push(format!("{book}.explicit = 0"));
    }
    let perms = u.permissions.clone().unwrap_or(Value::Null);
    let all_tags = perms.get("accessAllTags").is_some_and(book::truthy);
    let tags: Vec<Value> = perms.get("itemTagsSelected").and_then(Value::as_array).cloned().unwrap_or_default();
    if !all_tags && !tags.is_empty() {
        let marks = vec!["?"; tags.len()].join(", ");
        let count = format!("(SELECT count(*) FROM json_each({book}.tags) WHERE json_valid({book}.tags) AND json_each.value IN ({marks}))");
        if perms.get("selectedTagsNotAccessible").is_some_and(book::truthy) {
            conds.push(format!("{count} = 0"));
        } else {
            conds.push(format!("{count} >= 1"));
        }
        params.extend(tags);
    }
    (conds, params)
}

/// Adds [`permission_conditions`] to a page query.
fn permission_where(u: &User, sql: &mut Sql) {
    let (conds, params) = permission_conditions(u, "book");
    sql.wheres.extend(conds);
    sql.params_where.extend(params);
}

/// The conditions of a filter on the book itself (`getMediaGroupQuery`).
fn media_where(group: &str, value: Option<&str>, sql: &mut Sql) {
    let v = value.unwrap_or_default();
    let w = &mut sql.wheres;
    match group {
        "progress" => match v {
            "not-finished" => w.push("(mediaProgresses.isFinished IS NULL OR mediaProgresses.isFinished = 0)".into()),
            "not-started" => w.push(
                "((mediaProgresses.currentTime IS NULL OR mediaProgresses.currentTime = 0) AND \
                 (mediaProgresses.isFinished IS NULL OR mediaProgresses.isFinished = 0))"
                    .into(),
            ),
            "finished" => w.push("mediaProgresses.isFinished = 1".into()),
            "in-progress" => {
                w.push("((mediaProgresses.currentTime > 0 OR mediaProgresses.ebookProgress > 0) AND mediaProgresses.isFinished = 0)".into())
            }
            "audio-in-progress" => w.push("(mediaProgresses.currentTime > 0 AND mediaProgresses.isFinished = 0)".into()),
            "ebook-in-progress" => w.push(
                "(json_array_length(book.audioFiles) = 0 AND mediaProgresses.ebookProgress > 0 AND mediaProgresses.isFinished = 0)".into(),
            ),
            "ebook-finished" => {
                w.push("(json_array_length(book.audioFiles) = 0 AND mediaProgresses.isFinished = 1 AND book.ebookFile IS NOT NULL)".into())
            }
            _ => {}
        },
        "series" if v == "no-series" => w.push(NO_SERIES.into()),
        "abridged" => w.push("book.abridged = 1".into()),
        "explicit" => w.push("book.explicit = 1".into()),
        "genres" | "tags" | "narrators" => {
            w.push(format!("(SELECT count(*) FROM json_each(book.{group}) WHERE json_valid(book.{group}) AND json_each.value = ?) >= 1"));
            sql.params_where.push(Value::from(v));
        }
        "publishers" => {
            w.push("book.publisher = ?".into());
            sql.params_where.push(Value::from(v));
        }
        "languages" => {
            w.push("book.language = ?".into());
            sql.params_where.push(Value::from(v));
        }
        "tracks" => w.push(match v {
            "none" => "json_array_length(book.audioFiles) = 0".into(),
            "multi" => "json_array_length(book.audioFiles) > 1".into(),
            _ => "json_array_length(book.audioFiles) = 1".into(),
        }),
        "ebooks" => match v {
            "ebook" => w.push("book.ebookFile IS NOT NULL".into()),
            "no-ebook" => w.push("book.ebookFile IS NULL".into()),
            _ => {}
        },
        "missing" => match v {
            "asin" | "isbn" | "subtitle" | "publishedYear" | "description" | "publisher" | "language" | "cover" => {
                let key = if v == "cover" { "coverPath" } else { v };
                w.push(format!("(book.{key} IS NULL OR book.{key} = '')"));
            }
            "genres" | "tags" | "narrators" | "chapters" => w.push(format!("(book.{v} IS NULL OR json_array_length(book.{v}) = 0)")),
            "authors" => w.push(NO_AUTHORS.into()),
            "series" => w.push(NO_SERIES.into()),
            _ => {}
        },
        "publishedDecades" => {
            let start: i64 = v.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>().parse().unwrap_or(0);
            w.push("CAST(book.publishedYear AS INTEGER) BETWEEN ? AND ?".into());
            sql.params_where.push(Value::from(start));
            sql.params_where.push(Value::from(start + 9));
        }
        _ => {}
    }
}

/// A book without a series (`$series.id$ = null` through the series link).
const NO_SERIES: &str = "NOT EXISTS (SELECT 1 FROM bookSeries nbs JOIN series ns ON ns.id = nbs.seriesId WHERE nbs.bookId = book.id)";
/// A book without an author.
const NO_AUTHORS: &str = "NOT EXISTS (SELECT 1 FROM bookAuthors nba JOIN authors na ON na.id = nba.authorId WHERE nba.bookId = book.id)";

/// The `ORDER BY` items of a sort (`getOrder`).
fn order(sort_by: &str, desc: bool, collapse: bool, sorting: &Sorting) -> Vec<String> {
    let dir = if desc { "DESC" } else { "ASC" };
    let title = if sorting.ignore_prefix {
        format!("`libraryItem`.`titleIgnorePrefix` COLLATE NOCASE {dir}")
    } else {
        format!("`libraryItem`.`title` COLLATE NOCASE {dir}")
    };
    match sort_by {
        "addedAt" => vec![format!("libraryItem.createdAt {dir}")],
        "size" => vec![format!("libraryItem.size {dir}")],
        "birthtimeMs" => vec![format!("libraryItem.birthtime {dir}")],
        "mtimeMs" => vec![format!("libraryItem.mtime {dir}")],
        "media.duration" => vec![format!("`book`.`duration` {dir}")],
        "media.metadata.publishedYear" => vec![format!("CAST(`book`.`publishedYear` AS INTEGER) {dir}")],
        "media.metadata.authorNameLF" => vec![format!("`libraryItem`.`authorNamesLastFirst` COLLATE NOCASE {dir}"), title],
        "media.metadata.authorName" => vec![format!("`libraryItem`.`authorNamesFirstLast` COLLATE NOCASE {dir}"), title],
        "media.metadata.title" if collapse => vec![format!("display_title COLLATE NOCASE {dir}")],
        "media.metadata.title" => vec![title],
        "sequence" => {
            vec![format!("CAST(`filterSeries`.`sequence` AS FLOAT) {}", if desc { "DESC NULLS FIRST" } else { "ASC NULLS LAST" })]
        }
        "progress" => vec![format!("mediaProgresses.updatedAt {dir} NULLS LAST")],
        "progress.createdAt" => vec![format!("mediaProgresses.createdAt {dir} NULLS LAST")],
        "progress.finishedAt" => vec![format!("mediaProgresses.finishedAt {dir} NULLS LAST")],
        "random" => vec!["RANDOM()".into()],
        _ => Vec::new(),
    }
}

/// The books of a book library a page shows, in order, and how many there are in all
/// (`libraryItemsBookFilters.getFilteredLibraryItems`).
pub fn book_items(db: &dyn Db, library_id: &str, u: &User, q: &Query, sorting: &Sorting) -> Result<(Vec<PageItem>, i64)> {
    let group = q.group.as_deref().unwrap_or_default();
    let value = q.value.as_deref();
    let collapse = q.collapse_series && group != "series";
    let mut sort_by = q.sort_by.clone().unwrap_or_default();
    if group != "series" && sort_by == "sequence" {
        sort_by = "media.metadata.title".into();
    }
    let mut sql = Sql::default();
    sql.item_on.push("`libraryItem`.`libraryId` = ?".into());
    sql.params_item.push(Value::from(library_id));
    let mut orders = order(&sort_by, q.sort_desc, collapse, sorting);

    match group {
        "feed-open" => sql
            .joins
            .push("INNER JOIN feeds AS openFeed ON openFeed.entityId = libraryItem.id AND openFeed.entityType = 'libraryItem'".into()),
        "share-open" => sql.joins.push("INNER JOIN mediaItemShares AS share ON share.mediaItemId = book.id".into()),
        "ebooks" if value == Some("supplementary") => {
            sql.item_on.push("`libraryItem`.`libraryFiles` LIKE '%\"isSupplementary\":true%'".into())
        }
        "ebooks" if value == Some("no-supplementary") => {
            sql.item_on.push("`libraryItem`.`libraryFiles` NOT LIKE '%\"isSupplementary\":true%'".into())
        }
        "authors" => {
            sql.wheres.push("EXISTS (SELECT 1 FROM bookAuthors fba WHERE fba.bookId = book.id AND fba.authorId = ?)".into());
            sql.params_where.push(Value::from(value.unwrap_or_default()));
        }
        "series" if value != Some("no-series") => {
            sql.joins.push("INNER JOIN bookSeries AS filterSeries ON filterSeries.bookId = book.id AND filterSeries.seriesId = ?".into());
            sql.params_joins.push(Value::from(value.unwrap_or_default()));
            if sort_by != "sequence" {
                orders.push("CAST(`filterSeries`.`sequence` AS FLOAT) ASC NULLS LAST".into());
            }
        }
        "issues" => sql.item_on.push("(`libraryItem`.`isMissing` = 1 OR `libraryItem`.`isInvalid` = 1)".into()),
        "progress" => {
            let hide = if q.home_page { " AND mediaProgresses.hideFromContinueListening = 0" } else { "" };
            sql.joins.push(format!(
                "LEFT OUTER JOIN mediaProgresses AS mediaProgresses ON mediaProgresses.mediaItemId = book.id AND mediaProgresses.userId = ?{hide}"
            ));
            sql.params_joins.push(Value::from(u.id.clone().unwrap_or_default()));
        }
        "recent" => {
            sql.item_on.push("`libraryItem`.`createdAt` >= ?".into());
            sql.params_item.push(Value::from(date_text(now_ms() - 60 * 24 * 60 * 60 * 1000)));
        }
        _ => {}
    }
    if group != "progress" && matches!(sort_by.as_str(), "progress" | "progress.createdAt" | "progress.finishedAt") {
        sql.joins.push(
            "LEFT OUTER JOIN mediaProgresses AS mediaProgresses ON mediaProgresses.mediaItemId = book.id AND mediaProgresses.userId = ?"
                .into(),
        );
        sql.params_joins.push(Value::from(u.id.clone().unwrap_or_default()));
    }
    if !group.is_empty() {
        media_where(group, value, &mut sql);
    }
    permission_where(u, &mut sql);

    let mut display_title = String::new();
    let mut collapsed: Vec<(String, i64, Vec<String>)> = Vec::new();
    if collapse {
        let (exclude, include) = collapse_series_books(db, &sql, group, value)?;
        if !exclude.is_empty() {
            sql.wheres.push(format!("book.id NOT IN ({})", vec!["?"; exclude.len()].join(", ")));
            sql.params_where.extend(exclude.into_iter().map(Value::from));
        }
        let ids = include.iter().map(|(id, _, _)| format!("\"{}\"", id.replace('"', "\"\""))).collect::<Vec<_>>().join(", ");
        let (name, fallback) = if sorting.ignore_prefix { ("nameIgnorePrefix", "titleIgnorePrefix") } else { ("name", "title") };
        display_title = format!(
            ", IFNULL((SELECT s.{name} FROM bookSeries AS bs, series AS s WHERE bs.seriesId = s.id AND bs.bookId = book.id AND bs.id IN ({ids})), `libraryItem`.`{fallback}`) AS display_title"
        );
        collapsed = include;
    }

    let from = format!(
        "FROM `books` AS `book` INNER JOIN `libraryItems` AS `libraryItem` ON `book`.`id` = `libraryItem`.`mediaId` AND {} {}",
        sql.item_on.join(" AND "),
        sql.joins.join(" ")
    );
    let where_sql = if sql.wheres.is_empty() { String::new() } else { format!(" WHERE {}", sql.wheres.join(" AND ")) };
    let mut params = sql.params_item.clone();
    params.extend(sql.params_joins.clone());
    params.extend(sql.params_where.clone());

    let count = db
        .one(&format!("SELECT count(DISTINCT(`book`.`id`)) AS n {from}{where_sql}"), params.clone())?
        .map(|r| r.get("n").and_then(Value::as_i64).unwrap_or(0))
        .unwrap_or(0);

    // The original's SQLite reads the items in the order of their ids (seen in every recording): that is the
    // order without a sort, and of items a sort finds equal. It is written out here so it doesn't depend on the
    // query plan.
    orders.push("`libraryItem`.`id`".into());
    let order_sql = format!(" ORDER BY {}", orders.join(", "));
    let limit_sql = match (q.limit, q.offset) {
        (0, 0) => String::new(),
        (0, off) => format!(" LIMIT -1 OFFSET {off}"),
        (lim, off) => format!(" LIMIT {lim} OFFSET {off}"),
    };
    let rows = db.query(
        &format!(
            "SELECT `libraryItem`.*, `book`.`id` AS `bookIdForPage`{display_title}{} {from}{where_sql}{order_sql}{limit_sql}",
            if group == "series" && value != Some("no-series") {
                ", `filterSeries`.`seriesId` AS `filterSeriesId`, `filterSeries`.`sequence` AS `filterSequence`"
            } else {
                ""
            }
        ),
        params,
    )?;
    let mut items = Vec::new();
    let mut book_ids = Vec::new();
    let mut extras = Vec::new();
    for mut row in rows {
        let book_id = row.s("bookIdForPage");
        let filter_series = row.remove("filterSeriesId").and_then(|v| v.as_str().map(String::from));
        let filter_sequence = row.remove("filterSequence").and_then(|v| v.as_str().map(String::from));
        row.remove("bookIdForPage");
        row.remove("display_title");
        let item: LibraryItem = from_row(row)?;
        book_ids.push(book_id.clone());
        extras.push((filter_series, filter_sequence));
        items.push((item, book_id));
    }
    let mut books = book::load(db, &book_ids)?;
    let mut out = Vec::new();
    for ((item, book_id), (fs_id, fs_seq)) in items.into_iter().zip(extras) {
        let Some(full) = books.remove(&book_id) else { continue };
        let series = fs_id.as_ref().map(|sid| {
            let name = full.series.iter().find(|s| &s.id == sid).map(|s| s.name.clone()).unwrap_or_default();
            json!({"id": sid, "name": name, "sequence": fs_seq.clone().filter(|s| !s.is_empty())})
        });
        let collapsed_series = if collapse {
            full.series.iter().find_map(|se| {
                let (_, num, ids) = collapsed.iter().find(|(id, _, _)| *id == se.book_series_id)?;
                Some(json!({
                    "id": se.id,
                    "name": se.name,
                    "nameIgnorePrefix": se.name_ignore_prefix,
                    "sequence": se.sequence,
                    "numBooks": num,
                    "libraryItemIds": ids,
                }))
            })
        } else {
            None
        };
        out.push(PageItem { item, book: full, series, collapsed_series });
    }
    Ok((out, count))
}

/// For collapsed series: the books to leave out (every book of a series but its first), and for each series kept,
/// its link id, its number of books and its items (`getCollapseSeriesBooksToExclude`).
#[allow(clippy::type_complexity)]
fn collapse_series_books(
    db: &dyn Db,
    sql: &Sql,
    group: &str,
    value: Option<&str>,
) -> Result<(Vec<String>, Vec<(String, i64, Vec<String>)>)> {
    let mut wheres = Vec::new();
    let mut params = sql.params_item.clone();
    params.extend(sql.params_joins.clone());
    if group == "progress" {
        let v = value.unwrap_or_default();
        match v {
            "not-finished" => wheres.push("(mediaProgresses.isFinished IS NULL OR mediaProgresses.isFinished = 0)".to_string()),
            "not-started" => wheres.push(
                "((mediaProgresses.currentTime IS NULL OR mediaProgresses.currentTime = 0) AND (mediaProgresses.isFinished IS NULL OR mediaProgresses.isFinished = 0))"
                    .into(),
            ),
            "finished" => wheres.push("mediaProgresses.isFinished = 1".into()),
            "in-progress" => wheres.push(
                "((mediaProgresses.currentTime > 0 OR mediaProgresses.ebookProgress > 0) AND mediaProgresses.isFinished = 0)".into(),
            ),
            _ => {}
        }
    } else if group == "missing" && value == Some("authors") {
        wheres.push(NO_AUTHORS.into());
    } else {
        wheres.extend(sql.wheres.clone());
        params.extend(sql.params_where.clone());
    }
    let where_sql = if wheres.is_empty() { String::new() } else { format!(" WHERE {}", wheres.join(" AND ")) };
    let rows = db.query(
        &format!(
            "SELECT `series`.`id` AS seriesId, (SELECT count(*) FROM bookSeries bs WHERE bs.seriesId = series.id) AS numBooks, \
             `book`.`id` AS bookId, `bookSeries`.`id` AS bookSeriesId, `libraryItem`.`id` AS libraryItemId \
             FROM `series` AS `series` INNER JOIN `bookSeries` AS `bookSeries` ON `bookSeries`.`seriesId` = `series`.`id` \
             INNER JOIN `books` AS `book` ON `book`.`id` = `bookSeries`.`bookId` \
             INNER JOIN `libraryItems` AS `libraryItem` ON `book`.`id` = `libraryItem`.`mediaId` AND {} {}{where_sql} \
             ORDER BY CAST(`bookSeries`.`sequence` AS FLOAT) ASC NULLS LAST",
            sql.item_on.join(" AND "),
            sql.joins.join(" ")
        ),
        params,
    )?;
    // Group by series in the order each first appears, like Sequelize builds its nested results.
    let mut order: Vec<String> = Vec::new();
    let mut by_series: HashMap<String, (i64, Vec<(String, String, String)>)> = HashMap::new();
    for r in rows {
        let sid = r.s("seriesId");
        let entry = by_series.entry(sid.clone()).or_insert_with(|| {
            order.push(sid.clone());
            (r.get("numBooks").and_then(Value::as_i64).unwrap_or(0), Vec::new())
        });
        let bid = r.s("bookId");
        if !entry.1.iter().any(|(b, _, _)| *b == bid) {
            entry.1.push((bid, r.s("bookSeriesId"), r.s("libraryItemId")));
        }
    }
    let mut to_include: Vec<String> = Vec::new();
    let mut to_exclude: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    for sid in order {
        let (num, books) = &by_series[&sid];
        let mut found = false;
        for (bid, bsid, _) in books {
            if !found && !to_include.contains(bid) {
                to_include.push(bid.clone());
                kept.push((bsid.clone(), *num, books.iter().map(|(_, _, li)| li.clone()).collect()));
                to_exclude.retain(|x| x != bid);
                found = true;
            } else if !to_exclude.contains(bid) && !to_include.contains(bid) {
                to_exclude.push(bid.clone());
            }
        }
    }
    Ok((to_exclude, kept))
}

/// The lists of a book library's filter menus (`getFilterData` for books), without the cache.
pub fn book_filter_data(db: &dyn Db, library_id: &str) -> Result<Value> {
    let lib = vec![Value::from(library_id)];
    let count = |sql: &str| -> Result<i64> { Ok(db.one(sql, lib.clone())?.and_then(|r| r.get("n").and_then(Value::as_i64)).unwrap_or(0)) };
    let book_count = count("SELECT count(*) AS n FROM books b INNER JOIN libraryItems li ON li.mediaId = b.id AND li.libraryId = ?")?;
    let series_count = count("SELECT count(*) AS n FROM series WHERE libraryId = ?")?;
    let author_count = count("SELECT count(*) AS n FROM authors WHERE libraryId = ?")?;
    let rows = db.query(
        "SELECT b.tags, b.genres, b.publisher, b.publishedYear, b.narrators, b.language, li.isMissing, li.isInvalid \
         FROM books b INNER JOIN libraryItems li ON li.mediaId = b.id AND li.libraryId = ?",
        lib.clone(),
    )?;
    let mut sets: HashMap<&str, Vec<String>> = HashMap::new();
    let mut add = |k: &'static str, v: String| {
        let set = sets.entry(k).or_default();
        if !set.contains(&v) {
            set.push(v);
        }
    };
    let mut issues = 0;
    for r in rows {
        if r.get("isMissing").and_then(Value::as_i64).unwrap_or(0) != 0 || r.get("isInvalid").and_then(Value::as_i64).unwrap_or(0) != 0 {
            issues += 1;
        }
        for (col, key) in [("tags", "tags"), ("genres", "genres"), ("narrators", "narrators")] {
            let list: Option<Value> = r.get(col).and_then(Value::as_str).and_then(|t| serde_json::from_str(t).ok());
            for v in book::strings(&list) {
                add(key, v);
            }
        }
        if let Some(p) = r.get("publisher").and_then(Value::as_str).filter(|p| !p.is_empty()) {
            add("publishers", p.to_string());
        }
        if let Some(y) = r.get("publishedYear").and_then(Value::as_str).filter(|y| !y.is_empty()) {
            if let Ok(n) = y.trim().parse::<f64>() {
                if n > 0.0 && n < 3000.0 {
                    add("publishedDecades", ((n / 10.0).floor() as i64 * 10).to_string());
                }
            }
        }
        if let Some(l) = r.get("language").and_then(Value::as_str).filter(|l| !l.is_empty()) {
            add("languages", l.to_string());
        }
    }
    let named = |sql: &str, empty: &str| -> Result<Vec<Value>> {
        let mut list: Vec<(String, String)> = db
            .query(sql, lib.clone())?
            .iter()
            .map(|r| {
                (
                    r.s("id"),
                    r.get("name").and_then(Value::as_str).filter(|n| !n.is_empty() || empty.is_empty()).unwrap_or(empty).to_string(),
                )
            })
            .collect();
        list.sort_by(|a, b| natural_cmp(&a.1, &b.1));
        Ok(list.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect())
    };
    let series = named("SELECT id, name FROM series WHERE libraryId = ?", "No Title")?;
    let authors = named("SELECT id, name FROM authors WHERE libraryId = ?", "")?;
    let mut sorted = |k: &str| {
        let mut v = sets.remove(k).unwrap_or_default();
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    };
    Ok(json!({
        "authors": authors,
        "genres": sorted("genres"),
        "tags": sorted("tags"),
        "series": series,
        "narrators": sorted("narrators"),
        "languages": sorted("languages"),
        "publishers": sorted("publishers"),
        "publishedDecades": sorted("publishedDecades"),
        "bookCount": book_count,
        "authorCount": author_count,
        "seriesCount": series_count,
        "podcastCount": 0,
        "numIssues": issues,
        "loadedAt": now_ms(),
    }))
}

/// A book library's totals (`getBookLibraryStats`).
pub fn book_library_stats(db: &dyn Db, library_id: &str) -> Result<Value> {
    let r = db.one(
        "SELECT SUM(li.size) AS totalSize, SUM(b.duration) AS totalDuration, SUM(json_array_length(b.audioFiles)) AS numAudioFiles, \
         COUNT(*) AS totalItems FROM libraryItems li, books b WHERE b.id = li.mediaId AND li.libraryId = ?",
        vec![Value::from(library_id)],
    )?;
    let get = |k: &str| -> Value {
        let v = r.as_ref().and_then(|r| r.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
        super::util::js_num(if v.is_nan() { 0.0 } else { v })
    };
    Ok(json!({
        "totalSize": get("totalSize"),
        "totalDuration": get("totalDuration"),
        "numAudioFiles": get("numAudioFiles"),
        "totalItems": get("totalItems"),
    }))
}
