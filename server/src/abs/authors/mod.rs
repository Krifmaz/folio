//! Authors (PARITY A0017 and A0115, track C): a library's authors (`LibraryController.getAuthors`) and one author
//! (`AuthorController.findOne`), with the author JSON the original sends (`Author.toOldJSON`).

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::filters::{self, permission_conditions};
use crate::abs::library::util::Sorting;
use crate::abs::library::{self as lib};
use crate::abs::models::{self, from_row, library_item, Author, User};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// A series on an author's page: its name and its items with their numbers in it.
type SeriesGroup = (String, Vec<(Value, Option<String>)>);

/// The author routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/libraries/{id}/authors", get(library_authors)).route("/api/authors/{id}", get(find_one))
}

/// An author as the apps get it (`Author.toOldJSON`).
pub fn author_json(a: &Author) -> Value {
    json!({
        "id": a.id, "asin": a.asin, "name": a.name, "description": a.description, "imagePath": a.image_path,
        "libraryId": a.library_id,
        "addedAt": a.created_at.as_deref().and_then(models::date_ms), "updatedAt": a.updated_at.as_deref().and_then(models::date_ms),
    })
}

/// A library's authors with their number of books the user may see, in the order asked for. Users below admin
/// only get authors with such a book.
fn authors_of(db: &dyn Db, library_id: &str, u: &User, sort_by: &str, desc: bool, page: Option<(i64, i64)>) -> Result<(Vec<Value>, i64)> {
    let (perm, perm_params) = permission_conditions(u, "book");
    let perm_sql: String = perm.iter().map(|c| format!(" AND {c}")).collect();
    let num_books =
        format!("(SELECT count(DISTINCT book.id) FROM bookAuthors ba INNER JOIN books book ON book.id = ba.bookId WHERE ba.authorId = author.id{perm_sql})");
    let mut where_sql = "author.libraryId = ?".to_string();
    let mut where_params = vec![json!(library_id)];
    if !models::user::is_admin_or_up(u) {
        where_sql.push_str(&format!(" AND {num_books} > 0"));
        where_params.extend(perm_params.clone());
    }
    let dir = if desc { "DESC" } else { "ASC" };
    let order = match sort_by {
        "name" => format!("name COLLATE NOCASE {dir}, "),
        "lastFirst" => format!("lastFirst COLLATE NOCASE {dir}, "),
        "addedAt" => format!("author.createdAt {dir}, "),
        "updatedAt" => format!("author.updatedAt {dir}, "),
        _ => String::new(),
    };
    // The numbers-of-books sort is done after the query, on every author, like the original.
    let page_sql = match page {
        Some((limit, offset)) if sort_by != "numBooks" => format!(" LIMIT {limit} OFFSET {offset}"),
        _ => String::new(),
    };
    let count = db.one(&format!("SELECT count(DISTINCT author.id) AS n FROM authors author WHERE {where_sql}"), where_params.clone())?;
    let count = count.and_then(|r| r.get("n").and_then(Value::as_i64)).unwrap_or(0);
    // Authors a sort finds equal, and all of them without a sort, come in id order (see docs/DECISIONS.md).
    let rows = db.query(
        &format!("SELECT author.*, {num_books} AS numBooks FROM authors author WHERE {where_sql} ORDER BY {order}author.id{page_sql}"),
        [perm_params, where_params].concat(),
    )?;
    let mut out = Vec::new();
    for mut row in rows {
        let n = row.remove("numBooks").and_then(|v| v.as_i64()).unwrap_or(0);
        let a: Author = from_row(row)?;
        let mut j = author_json(&a);
        j["numBooks"] = json!(n);
        j["lastFirst"] = json!(a.last_first);
        out.push(j);
    }
    if sort_by == "numBooks" {
        let n = |v: &Value| v["numBooks"].as_i64().unwrap_or(0);
        out.sort_by(|a, b| if desc { n(b).cmp(&n(a)) } else { n(a).cmp(&n(b)) });
        if let Some((limit, offset)) = page {
            out = out.into_iter().skip(offset as usize).take(limit as usize).collect();
        }
    }
    Ok((out, count))
}

/// `GET /api/libraries/:id/authors` (A0017, `getAuthors`): the library's authors, as a page with `limit` and
/// `page`, else all of them under `authors`.
async fn library_authors(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let library = match lib::library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let limit: Option<i64> = q.get("limit").and_then(|v| v.parse().ok());
    let page: Option<i64> = q.get("page").and_then(|v| v.parse().ok());
    let paginated = match (limit, page) {
        (Some(l), Some(p)) if l > 0 => Some((l, p)),
        _ => None,
    };
    let sort_by = q.get("sort").cloned().unwrap_or_default();
    let desc = q.get("desc").map(String::as_str) == Some("1");
    let (authors, count) =
        match authors_of(&st.abs, library.id.as_deref().unwrap_or_default(), &user, &sort_by, desc, paginated.map(|(l, p)| (l, p * l))) {
            Ok(x) => x,
            Err(e) => return e.into_response(),
        };
    let Some((limit, page)) = paginated else { return Json(json!({"authors": authors})).into_response() };
    let mut payload = Map::new();
    payload.insert("results".into(), Value::Array(authors));
    payload.insert("total".into(), json!(count));
    payload.insert("limit".into(), json!(limit));
    payload.insert("page".into(), json!(page));
    if let Some(s) = q.get("sort") {
        payload.insert("sortBy".into(), json!(s));
    }
    payload.insert("sortDesc".into(), json!(desc));
    if let Some(f) = q.get("filter") {
        payload.insert("filterBy".into(), json!(f));
    }
    payload.insert("minified".into(), json!(q.get("minified").map(String::as_str) == Some("1")));
    if let Some(i) = q.get("include") {
        payload.insert("include".into(), json!(i));
    }
    Json(Value::Object(payload)).into_response()
}

/// `GET /api/authors/:id` (A0115, `AuthorController.findOne`): the author, 404 when it doesn't exist or is in a
/// library the user may not open; with `include=items` its items (newest first), and with `series` too the items
/// grouped by series.
async fn find_one(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let include: Vec<String> = q.get("include").map(|i| i.split(',').map(String::from).collect()).unwrap_or_default();
    let sorting = Sorting::of(&st.abs_server.settings());
    let run = || -> Result<Option<Value>> {
        let Some(a) = models::find::<Author>(&st.abs, &id)? else { return Ok(None) };
        if !models::user::can_access_library(&user, a.library_id.as_deref().unwrap_or_default()) {
            return Ok(None);
        }
        let mut j = author_json(&a);
        if include.iter().any(|i| i == "items") {
            let query = filters::Query {
                group: Some("authors".into()),
                value: a.id.clone(),
                sort_by: Some("addedAt".into()),
                sort_desc: true,
                ..Default::default()
            };
            let (items, _) = filters::book_items(&st.abs, a.library_id.as_deref().unwrap_or_default(), &user, &query, &sorting)?;
            if include.iter().any(|i| i == "series") {
                let mut order: Vec<String> = Vec::new();
                let mut groups: HashMap<String, SeriesGroup> = HashMap::new();
                for p in &items {
                    for se in &p.book.series {
                        let mut item = library_item::minified(&p.item, &p.book, &sorting);
                        if let Some(meta) = item.get_mut("media").and_then(|m| m.get_mut("metadata")).and_then(Value::as_object_mut) {
                            meta.insert(
                                "series".into(),
                                json!({"id": se.id, "name": se.name, "nameIgnorePrefix": se.name_ignore_prefix, "sequence": se.sequence}),
                            );
                        }
                        let entry = groups.entry(se.id.clone()).or_insert_with(|| {
                            order.push(se.id.clone());
                            (se.name.clone(), Vec::new())
                        });
                        entry.1.push((Value::Object(item), se.sequence.clone().filter(|s| !s.is_empty())));
                    }
                }
                let series: Vec<Value> = order
                    .iter()
                    .map(|sid| {
                        let (name, mut list) = groups.remove(sid).unwrap_or_default();
                        crate::abs::series::sort_by_sequence(&mut list);
                        json!({"id": sid, "name": name, "items": list.into_iter().map(|(v, _)| v).collect::<Vec<_>>()})
                    })
                    .collect();
                j["series"] = Value::Array(series);
            }
            j["libraryItems"] =
                Value::Array(items.iter().map(|p| Value::Object(library_item::minified(&p.item, &p.book, &sorting))).collect());
        }
        Ok(Some(j))
    };
    match run() {
        Ok(Some(j)) => Json(j).into_response(),
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => e.into_response(),
    }
}
