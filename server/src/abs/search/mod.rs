//! Library search (PARITY A0015, track C): `GET /api/libraries/:id/search` (`LibraryController.search` and
//! `libraryItemsBookFilters.search`). It finds books by title, subtitle, ASIN or ISBN, and narrators, tags,
//! genres, series and authors by name.
//!
//! The original matches with SQLite's `LIKE '%q%'` on top of the nunicode extension its image loads: case is
//! ignored for every letter, and accents are ignored too unless the query has some (`TextSearchQuery`). Folio's
//! SQLite has no such extension, so the same match is done here on the rows read from the database.

use crate::abs::auth::AbsUser;
use crate::abs::library::filters::permission_conditions;
use crate::abs::library::util::Sorting;
use crate::abs::library::{self as lib, send_text};
use crate::abs::models::{self, book, from_row, library_item, Author, LibraryItem, Series, User};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// The search route.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/libraries/{id}/search", get(search))
}

/// A text without its accents (nunicode's `unaccent`).
fn unaccent(s: &str) -> String {
    s.nfd().filter(|c| !is_combining_mark(*c)).nfc().collect()
}

/// One query's text match (`TextSearchQuery.matchExpression`): `column LIKE '%query%'`, on the column without
/// accents when the query has none.
pub struct TextMatch {
    /// The query's characters in lower case, as a `LIKE` pattern without the `%` around it.
    pattern: Vec<char>,
    /// Whether the query has accents, so columns are compared as they are.
    has_accents: bool,
}

impl TextMatch {
    /// The match for a search query.
    pub fn new(query: &str) -> Self {
        TextMatch { pattern: fold(query), has_accents: unaccent(query) != query }
    }

    /// Whether a column's text matches.
    pub fn matches(&self, text: &str) -> bool {
        let text = if self.has_accents { fold(text) } else { fold(&unaccent(text)) };
        let mut pattern = vec!['%'];
        pattern.extend(&self.pattern);
        pattern.push('%');
        like(&pattern, &text)
    }
}

/// A text in lower case, letter by letter, as `LIKE` compares it.
fn fold(s: &str) -> Vec<char> {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// SQLite's `LIKE` on folded text: `%` stands for any run of characters, `_` for one.
fn like(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut back: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && pattern[p] == '%' {
            back = Some((p, t));
            p += 1;
        } else if p < pattern.len() && (pattern[p] == '_' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if let Some((bp, bt)) = back {
            p = bp + 1;
            t = bt + 1;
            back = Some((bp, bt + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '%')
}

/// The text of a JSON value as SQLite's `LIKE` reads it.
fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// `GET /api/libraries/:id/search` (A0015): 400 without a text `q`; `limit` defaults to 12.
async fn search(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let library = match lib::library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let Some(query) = q.get("q").filter(|v| !v.is_empty()) else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid request. Query param \"q\" must be a string");
    };
    let limit = q.get("limit").and_then(|l| l.parse::<usize>().ok()).filter(|l| *l > 0).unwrap_or(12);
    let sorting = Sorting::of(&st.abs_server.settings());
    let library_id = library.id.clone().unwrap_or_default();
    let found = if library.media_type.as_deref() == Some("podcast") {
        podcast_search(&st.abs, &library_id, query.trim(), limit)
    } else {
        book_search(&st.abs, &library_id, &user, query.trim(), limit, &sorting)
    };
    match found {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// How often each value of a JSON list column of a library's media matches (`json_each` grouped by value), in
/// value order.
fn list_values(db: &dyn Db, table: &str, column: &str, library_id: &str, m: &TextMatch) -> Result<BTreeMap<String, i64>> {
    let rows = db.query(
        &format!(
            "SELECT json_each.value AS value FROM {table} b, libraryItems li, json_each(b.{column}) \
             WHERE json_valid(b.{column}) AND b.id = li.mediaId AND li.libraryId = ?"
        ),
        vec![json!(library_id)],
    )?;
    let mut counts = BTreeMap::new();
    for v in rows.iter().filter_map(|r| r.get("value").and_then(text_of)).filter(|v| m.matches(v)) {
        *counts.entry(v).or_insert(0) += 1;
    }
    Ok(counts)
}

/// Values with their counts, most first (`ORDER BY numItems DESC`), at most `limit`.
fn most_first(counts: BTreeMap<String, i64>, limit: usize) -> Vec<Value> {
    let mut list: Vec<(String, i64)> = counts.into_iter().collect();
    list.sort_by_key(|b| std::cmp::Reverse(b.1));
    list.into_iter().take(limit).map(|(name, n)| json!({"name": name, "numItems": n})).collect()
}

/// A book library's search (`libraryItemsBookFilters.search`).
fn book_search(db: &dyn Db, library_id: &str, u: &User, query: &str, limit: usize, sorting: &Sorting) -> Result<Value> {
    let m = TextMatch::new(query);
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let contains = |v: &Option<String>| v.as_deref().is_some_and(|s| m.matches(s));

    // Books by title, subtitle, ASIN or ISBN, in stored order.
    let rows = db.query(
        &format!(
            "SELECT book.id AS id, book.title AS title, book.subtitle AS subtitle, book.asin AS asin, book.isbn AS isbn \
             FROM books book INNER JOIN libraryItems li ON li.mediaId = book.id AND li.libraryId = ? WHERE 1 = 1{perm_sql} \
             ORDER BY book.rowid"
        ),
        [vec![json!(library_id)], perm_params.clone()].concat(),
    )?;
    let ids: Vec<String> = rows
        .iter()
        .filter(|r| ["title", "subtitle", "asin", "isbn"].iter().any(|c| contains(&r.get(*c).and_then(Value::as_str).map(String::from))))
        .map(|r| r.s("id"))
        .take(limit)
        .collect();
    let books = items_of(db, &ids)?;
    let book_matches: Vec<Value> =
        books.iter().map(|(item, full)| json!({"libraryItem": Value::Object(library_item::expanded(item, full, sorting))})).collect();

    // Narrators, grouped by name.
    let narrators: Vec<Value> = list_values(db, "books", "narrators", library_id, &m)?
        .into_iter()
        .take(limit)
        .map(|(name, n)| json!({"name": name, "numBooks": n}))
        .collect();
    let tags = most_first(list_values(db, "books", "tags", library_id, &m)?, limit);
    let genres = most_first(list_values(db, "books", "genres", library_id, &m)?, limit);

    // Series by name, each with the books the user may see; series without any are left out.
    let all_series: Vec<Series> = models::list(db, "`libraryId` = ?", "rowid", vec![json!(library_id)])?;
    let mut series_matches = Vec::new();
    for se in all_series.iter().filter(|s| contains(&s.name)).take(limit) {
        let links = db.query(
            &format!(
                "SELECT bs.bookId AS bookId FROM bookSeries bs INNER JOIN books book ON book.id = bs.bookId \
                 WHERE bs.seriesId = ?{perm_sql} ORDER BY bs.rowid"
            ),
            [vec![json!(se.id)], perm_params.clone()].concat(),
        )?;
        let ids: Vec<String> = links.iter().map(|l| l.s("bookId")).collect();
        let books: Vec<Value> = items_of(db, &ids)?
            .into_iter()
            .map(|(item, full)| library_item::old_json(&item, &book::BookFull { book: full.book, ..Default::default() }))
            .collect();
        if !books.is_empty() {
            series_matches.push(json!({"series": Value::Object(crate::abs::series::series_json(se, sorting)), "books": books}));
        }
    }

    // Authors by name, with all their books counted.
    let rows = db.query(
        "SELECT author.*, (SELECT count(*) FROM bookAuthors ba WHERE ba.authorId = author.id) AS numBooks \
         FROM authors author WHERE author.libraryId = ? ORDER BY author.rowid",
        vec![json!(library_id)],
    )?;
    let mut authors = Vec::new();
    for mut row in rows {
        let n = row.remove("numBooks").and_then(|v| v.as_i64()).unwrap_or(0);
        let a: Author = from_row(row)?;
        if contains(&a.name) && authors.len() < limit {
            let mut j = crate::abs::authors::author_json(&a);
            j["numBooks"] = json!(n);
            authors.push(j);
        }
    }

    Ok(json!({
        "book": book_matches, "narrators": narrators, "tags": tags, "genres": genres, "series": series_matches, "authors": authors,
    }))
}

/// The items of books, in the order of the ids, with their authors and series; books without an item are left out.
fn items_of(db: &dyn Db, ids: &[String]) -> Result<Vec<(LibraryItem, book::BookFull)>> {
    let mut books = book::load(db, ids)?;
    let mut out = Vec::new();
    for id in ids {
        let Some(row) = db.one("SELECT * FROM libraryItems WHERE mediaId = ? LIMIT 1", vec![json!(id)])? else { continue };
        if let Some(full) = books.remove(id) {
            out.push((from_row(row)?, full));
        }
    }
    Ok(out)
}

/// A podcast library's search (`libraryItemsPodcastFilters.search`). Podcasts come with track C's podcast work
/// (P5); until then only their tags and genres are searched.
fn podcast_search(db: &dyn Db, library_id: &str, query: &str, limit: usize) -> Result<Value> {
    let m = TextMatch::new(query);
    let tags = most_first(list_values(db, "podcasts", "tags", library_id, &m)?, limit);
    let genres = most_first(list_values(db, "podcasts", "genres", library_id, &m)?, limit);
    Ok(json!({"podcast": [], "tags": tags, "genres": genres, "episodes": []}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_matches_as_sqlite_does() {
        let m = TextMatch::new("sys");
        assert!(m.matches("All Systems Red"));
        assert!(!m.matches("Ancillary Justice"));
        assert!(TextMatch::new("a%r").matches("All Systems Red"));
        assert!(TextMatch::new("r_d").matches("Red"));
        assert!(!TextMatch::new("r_d").matches("Rd"));
        assert!(TextMatch::new("").matches("anything"));
    }

    #[test]
    fn accents_are_ignored_unless_the_query_has_some() {
        assert!(TextMatch::new("bronte").matches("Charlotte Brontë"));
        assert!(TextMatch::new("BRONTË").matches("Charlotte Brontë"));
        assert!(!TextMatch::new("brontë").matches("Charlotte Bronte"));
    }
}
