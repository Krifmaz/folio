//! The port of `server/controllers/LibraryController.js` (PARITY rows A0001–A0028, track C): the libraries and
//! what a library page shows, with the helpers it shares with the rest of the library side (names, filters,
//! sorting and the original's JSON conventions).
//!
//! Done so far: `POST /api/libraries`, `GET /api/libraries`, `GET /api/libraries/:id` and `GET /api/libraries/:id/items` for book
//! libraries. Every library route checks access first like the original's `middleware`.

pub mod filters;
pub mod names;
pub mod shelves;
pub mod util;

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, find, list, Library, LibraryFolder, User};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use util::{ms, ms0, Sorting};

/// The library routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/libraries", get(find_all).post(create))
        .route("/api/libraries/{id}", get(find_one))
        .route("/api/libraries/{id}/items", get(library_items))
        .route("/api/libraries/{id}/personalized", get(personalized))
}

/// An answer like Express's `res.status(code).send(text)`: the text as HTML.
pub fn send_text(code: StatusCode, text: &str) -> Response {
    let mut r = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], text.to_string()).into_response();
    *r.status_mut() = code;
    r
}

/// A database error as an answer.
fn fail(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// A library's folders, in the order they are stored.
pub fn folders(db: &dyn Db, library_id: &str) -> Result<Vec<LibraryFolder>> {
    list(db, "`libraryId` = ?", "", vec![Value::from(library_id)])
}

/// A library's settings, or the defaults for its media type (`get settings` / `getDefaultLibrarySettingsForMediaType`).
pub fn default_settings(media_type: &str) -> Value {
    if media_type == "podcast" {
        json!({
            "coverAspectRatio": 1,
            "disableWatcher": false,
            "autoScanCronExpression": null,
            "podcastSearchRegion": "us",
            "markAsFinishedPercentComplete": null,
            "markAsFinishedTimeRemaining": 10
        })
    } else {
        json!({
            "coverAspectRatio": 1,
            "disableWatcher": false,
            "autoScanCronExpression": null,
            "skipMatchingMediaWithAsin": false,
            "skipMatchingMediaWithIsbn": false,
            "audiobooksOnly": false,
            "epubsAllowScriptedContent": false,
            "hideSingleBookSeries": false,
            "onlyShowLaterBooksInContinueSeries": false,
            "metadataPrecedence": ["folderStructure", "audioMetatags", "nfoFile", "txtFiles", "opfFile", "absMetadata"],
            "markAsFinishedPercentComplete": null,
            "markAsFinishedTimeRemaining": 10
        })
    }
}

/// A library as the API shows it (`Library.toOldJSON`), with its folders (`LibraryFolder.toOldJSON`).
pub fn library_json(lib: &Library, folders: &[LibraryFolder]) -> Value {
    let folders: Vec<Value> =
        folders.iter().map(|f| json!({"id": f.id, "fullPath": f.path, "libraryId": f.library_id, "addedAt": ms0(&f.created_at)})).collect();
    let settings = match &lib.settings {
        Some(Value::Object(m)) => Value::Object(m.clone()),
        _ => Value::Object(Map::new()),
    };
    json!({
        "id": lib.id,
        "name": lib.name,
        "folders": folders,
        "displayOrder": lib.display_order,
        "icon": lib.icon,
        "mediaType": lib.media_type,
        "provider": lib.provider,
        "settings": settings,
        "lastScan": ms(&lib.last_scan).filter(|v| *v != 0),
        "lastScanVersion": lib.last_scan_version,
        "createdAt": ms0(&lib.created_at),
        "lastUpdate": ms0(&lib.updated_at),
    })
}

/// A JavaScript `Number(text)` for a query value: `None` when it isn't a number (`NaN`).
pub fn js_number(text: &str) -> Option<f64> {
    let t = text.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    let unsigned = t.trim_start_matches(['+', '-']);
    let sign = if t.starts_with('-') { -1.0 } else { 1.0 };
    if unsigned == "Infinity" {
        return Some(sign * f64::INFINITY);
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = t.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix).ok().map(|n| n as f64);
        }
    }
    if !unsigned.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
        return None;
    }
    t.parse::<f64>().ok()
}

/// The library a route is about, after the original's checks (`LibraryController.middleware`): 403 when the user
/// may not open it, 404 when it doesn't exist, 400 when `limit` or `page` isn't a whole number from 0.
pub(crate) fn library_for(
    st: &AppState,
    u: &User,
    id: &str,
    q: &mut HashMap<String, String>,
) -> std::result::Result<Library, Box<Response>> {
    if !models::user::can_access_library(u, id) {
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    let Some(lib) = find::<Library>(&st.abs, id).map_err(|e| Box::new(fail(e)))? else {
        return Err(Box::new(send_text(StatusCode::NOT_FOUND, "Library not found")));
    };
    for key in ["limit", "page"] {
        if let Some(v) = q.get(key) {
            let n = js_number(v).unwrap_or(0.0);
            if n.fract() != 0.0 || !n.is_finite() || n < 0.0 {
                return Err(Box::new(send_text(StatusCode::BAD_REQUEST, &format!("Invalid request. {key} must be a positive integer"))));
            }
            q.insert(key.to_string(), format!("{}", n as i64));
        }
    }
    Ok(lib)
}

/// Whether a JSON value counts as true in JavaScript (`!!value`).
pub fn js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// JavaScript's `typeof` for a JSON value (`null` and arrays are objects).
pub fn js_typeof(v: &Value) -> &'static str {
    match v {
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        _ => "object",
    }
}

/// JavaScript's `Number(value)` for a JSON value: `None` for `NaN`.
pub fn js_to_number(v: &Value) -> Option<f64> {
    match v {
        Value::Null => Some(0.0),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Number(n) => n.as_f64(),
        Value::String(s) => js_number(s),
        Value::Array(a) if a.is_empty() => Some(0.0),
        Value::Array(a) if a.len() == 1 => js_to_number(&Value::String(js_string(&a[0]))),
        _ => None,
    }
}

/// JavaScript's `String(value)` for the values an array of one turns into when made a number.
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
        other => other.to_string(),
    }
}

/// Node's `path.resolve(path)`: the path made absolute from the working folder and tidied.
fn resolve_path(path: &str) -> String {
    let full = if path.starts_with('/') {
        path.to_string()
    } else {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
        format!("{cwd}/{path}")
    };
    let n = crate::abs::scanner::files::normalize(&full);
    if n.len() > 1 {
        n.trim_end_matches('/').to_string()
    } else {
        n
    }
}

/// The settings of a new library: the defaults for its media type with the given ones checked and applied
/// (`LibraryController.create`'s "Validate settings"), or the reason they can't be used.
fn new_library_settings(media_type: &str, given: Option<&Map<String, Value>>) -> std::result::Result<Value, String> {
    let mut settings = default_settings(media_type);
    let Some(given) = given else { return Ok(settings) };
    for (key, value) in given {
        let Some(default) = settings.get(key).cloned() else { continue };
        let new = match key.as_str() {
            "metadataPrecedence" => {
                if !value.is_array() {
                    return Err("Invalid request. Settings \"metadataPrecedence\" must be an array".into());
                }
                value.clone()
            }
            "autoScanCronExpression" | "podcastSearchRegion" => {
                if !js_truthy(value) {
                    continue;
                }
                if !value.is_string() {
                    return Err(format!("Invalid request. Settings \"{key}\" must be a string"));
                }
                value.clone()
            }
            "markAsFinishedPercentComplete" | "markAsFinishedTimeRemaining" => {
                if value.is_null() {
                    Value::Null
                } else {
                    let Some(n) = js_to_number(value) else {
                        return Err(format!("Invalid request. Setting \"{key}\" must be a number"));
                    };
                    if key == "markAsFinishedPercentComplete" && !(0.0..=100.0).contains(&n) {
                        return Err(format!("Invalid request. Setting \"{key}\" must be between 0 and 100"));
                    }
                    if key == "markAsFinishedTimeRemaining" && n < 0.0 {
                        return Err(format!("Invalid request. Setting \"{key}\" must be greater than or equal to 0"));
                    }
                    util::js_num(n)
                }
            }
            _ => {
                if js_typeof(value) != js_typeof(&default) {
                    return Err(format!("Invalid request. Setting \"{key}\" must be of type {}", js_typeof(&default)));
                }
                value.clone()
            }
        };
        settings[key] = new;
    }
    Ok(settings)
}

/// `POST /api/libraries` (A0001, `create`): admins make a library from a name, folders (made when missing) and
/// optional media type, icon, provider and settings; answers the new library.
async fn create(State(st): State<AppState>, AbsUser { user, .. }: AbsUser, body: axum::body::Bytes) -> Response {
    if !models::user::is_admin_or_up(&user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let bad = |text: &str| send_text(StatusCode::BAD_REQUEST, text);
    let name = match body.get("name") {
        Some(Value::String(n)) if !n.is_empty() => n.clone(),
        _ => return bad("Invalid request. Name must be a string"),
    };
    let folder_path = |f: &Value| -> Option<String> {
        let full = f.get("fullPath").filter(|v| js_truthy(v)).or_else(|| f.get("path").filter(|v| js_truthy(v)))?;
        full.as_str().map(String::from)
    };
    let paths: Option<Vec<String>> = body.get("folders").and_then(Value::as_array).and_then(|l| l.iter().map(folder_path).collect());
    let Some(paths) = paths else {
        return bad("Invalid request. Folders must be a non-empty array of objects with path string");
    };
    for field in ["mediaType", "icon", "provider"] {
        if body.get(field).is_some_and(|v| js_truthy(v) && !v.is_string()) {
            return bad(&format!("Invalid request. {field} must be a string"));
        }
    }
    let given_settings = match body.get("settings") {
        Some(v) if js_truthy(v) => match v {
            Value::Object(m) => Some(m),
            _ => return bad("Invalid request. Settings must be an object"),
        },
        _ => None,
    };
    let text_or =
        |field: &str, default: &str| body.get(field).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(default).to_string();
    let media_type = text_or("mediaType", "book");
    let provider = text_or("provider", "google");
    let icon = text_or("icon", "database");
    let settings = match new_library_settings(&media_type, given_settings) {
        Ok(s) => s,
        Err(text) => return bad(&text),
    };
    if let Some(id) = provider.strip_prefix("custom-") {
        let exists = !id.is_empty()
            && match st.abs.one("SELECT id FROM customMetadataProviders WHERE id = ?", vec![json!(id)]) {
                Ok(r) => r.is_some(),
                Err(e) => return fail(e),
            };
        if !exists {
            return bad("Invalid request. Custom metadata provider does not exist");
        }
    }
    let paths: Vec<String> = paths.iter().map(|p| resolve_path(p)).collect();
    for p in &paths {
        if std::fs::create_dir_all(p).is_err() {
            return bad(&format!("Invalid request. Invalid folder directory \"{p}\""));
        }
    }
    let run = || -> Result<(Library, Vec<LibraryFolder>)> {
        let max = st
            .abs
            .one("SELECT max(displayOrder) AS n FROM libraries", vec![])?
            .and_then(|r| r.get("n").and_then(Value::as_i64))
            .unwrap_or(0);
        let now = models::date_now();
        let lib = Library {
            id: Some(models::new_id()),
            name: Some(name.clone()),
            display_order: Some(max + 1),
            icon: Some(icon.clone()),
            media_type: Some(media_type.clone()),
            provider: Some(provider.clone()),
            settings: Some(settings.clone()),
            created_at: Some(now.clone()),
            updated_at: Some(now.clone()),
            ..Default::default()
        };
        models::insert(&st.abs, &lib)?;
        let mut made = Vec::new();
        for p in &paths {
            let f = LibraryFolder {
                id: Some(models::new_id()),
                path: Some(p.clone()),
                created_at: Some(now.clone()),
                updated_at: Some(now.clone()),
                library_id: lib.id.clone(),
            };
            models::insert(&st.abs, &f)?;
            made.push(f);
        }
        Ok((lib, made))
    };
    let (lib, made) = match run() {
        Ok(x) => x,
        Err(e) => {
            tracing::error!("The library can't be made: {}", e.detail);
            return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create library");
        }
    };
    let mut json = library_json(&lib, &made);
    // The original answers with the object it just made, whose `lastScanVersion` is still unset (left out).
    if let Some(m) = json.as_object_mut() {
        m.remove("lastScanVersion");
    }
    let id = lib.id.clone().unwrap_or_default();
    st.sockets.emitter("library_added", &json, Some(&|u: &User| models::user::can_access_library(u, &id)));
    Json(json).into_response()
}

/// `GET /api/libraries` (A0002, `findAll`): the libraries the user may open, in display order, with `stats` when
/// `include=stats`.
async fn find_all(State(st): State<AppState>, AbsUser { user, .. }: AbsUser, Query(q): Query<HashMap<String, String>>) -> Response {
    let run = || -> Result<Value> {
        let mut libs: Vec<Library> = list(&st.abs, "1", "`displayOrder` ASC", vec![])?;
        let accessible: Vec<String> = user
            .permissions
            .as_ref()
            .and_then(|p| p.get("librariesAccessible"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        if !accessible.is_empty() {
            libs.retain(|l| l.id.as_ref().is_some_and(|id| accessible.contains(id)));
        }
        let with_stats = q.get("include").is_some_and(|i| i.split(',').any(|x| x == "stats"));
        let mut out = Vec::new();
        for lib in &libs {
            let id = lib.id.clone().unwrap_or_default();
            let mut j = library_json(lib, &folders(&st.abs, &id)?);
            if with_stats && lib.media_type.as_deref() == Some("book") {
                j["stats"] = filters::book_library_stats(&st.abs, &id)?;
            }
            out.push(j);
        }
        Ok(json!({ "libraries": out }))
    };
    run().map(Json).map_err(fail).into_response()
}

/// The filter data cache (`Database.libraryFilterData`): computed data per library, kept for 30 minutes and
/// reused while nothing changed.
static FILTER_DATA: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

/// The filter data of a library (`libraryFilters.getFilterData`), from the cache when it is fresh.
pub fn filter_data(db: &dyn Db, media_type: &str, library_id: &str) -> Result<Value> {
    let cached = FILTER_DATA.lock().ok().and_then(|c| c.as_ref().and_then(|m| m.get(library_id).cloned()));
    let now = models::now_ms();
    if let Some(c) = &cached {
        let loaded = c.get("loadedAt").and_then(Value::as_i64).unwrap_or(0);
        if now - loaded < 30 * 60 * 1000 {
            return Ok(c.clone());
        }
        if media_type != "podcast" && unchanged_since(db, library_id, loaded, c)? {
            let mut fresh = c.clone();
            fresh["loadedAt"] = Value::from(now);
            store_filter_data(library_id, &fresh);
            return Ok(fresh);
        }
    }
    let data = filters::book_filter_data(db, library_id)?;
    store_filter_data(library_id, &data);
    Ok(data)
}

/// Whether no book, series or author of a library changed since `loaded_at`, and the counts are the cached ones.
fn unchanged_since(db: &dyn Db, library_id: &str, loaded_at: i64, cached: &Value) -> Result<bool> {
    let since = Value::from(models::date_text(loaded_at));
    let lib = Value::from(library_id);
    let n = |sql: &str, params: Vec<Value>| -> Result<i64> {
        Ok(db.one(sql, params)?.and_then(|r| r.get("n").and_then(Value::as_i64)).unwrap_or(0))
    };
    let changed = n(
        "SELECT count(*) AS n FROM books b INNER JOIN libraryItems li ON li.mediaId = b.id AND li.libraryId = ? AND li.updatedAt > ? WHERE b.updatedAt > ?",
        vec![lib.clone(), since.clone(), since.clone()],
    )? + n("SELECT count(*) AS n FROM series WHERE libraryId = ? AND updatedAt > ?", vec![lib.clone(), since.clone()])?
        + n("SELECT count(*) AS n FROM authors WHERE libraryId = ? AND updatedAt > ?", vec![lib.clone(), since])?;
    if changed != 0 {
        return Ok(false);
    }
    let books =
        n("SELECT count(*) AS n FROM books b INNER JOIN libraryItems li ON li.mediaId = b.id AND li.libraryId = ?", vec![lib.clone()])?;
    let series = n("SELECT count(*) AS n FROM series WHERE libraryId = ?", vec![lib.clone()])?;
    let authors = n("SELECT count(*) AS n FROM authors WHERE libraryId = ?", vec![lib])?;
    let same = |k: &str, v: i64| cached.get(k).and_then(Value::as_i64) == Some(v);
    Ok(same("bookCount", books) && same("seriesCount", series) && same("authorCount", authors))
}

/// Keeps a library's filter data in the cache.
fn store_filter_data(library_id: &str, data: &Value) {
    if let Ok(mut c) = FILTER_DATA.lock() {
        c.get_or_insert_with(HashMap::new).insert(library_id.to_string(), data.clone());
    }
}

/// Changes a library's cached filter data in place, when it is cached (the original's `Database.add*ToFilterData`
/// and `remove*FromFilterData`, which do nothing for a library not loaded yet).
pub fn with_filter_data(library_id: &str, change: impl FnOnce(&mut Value)) {
    if let Ok(mut c) = FILTER_DATA.lock() {
        if let Some(fd) = c.as_mut().and_then(|m| m.get_mut(library_id)) {
            change(fd);
        }
    }
}

/// Adds `{id, name}` to a filter list unless the id is there already (`addAuthorToFilterData`,
/// `addSeriesToFilterData`).
pub fn push_named(fd: &mut Value, key: &str, id: &str, name: &str) {
    if let Some(list) = fd.get_mut(key).and_then(Value::as_array_mut) {
        if !list.iter().any(|v| v["id"] == id) {
            list.push(serde_json::json!({"id": id, "name": name}));
        }
    }
}

/// Removes the entry with this id from a filter list (`removeAuthorFromFilterData`, `removeSeriesFromFilterData`).
pub fn remove_named(fd: &mut Value, key: &str, id: &str) {
    if let Some(list) = fd.get_mut(key).and_then(Value::as_array_mut) {
        list.retain(|v| v["id"] != id);
    }
}

/// Adds values to a filter list that doesn't have them yet (`addNarratorsToFilterData` and the like).
pub fn push_values(fd: &mut Value, key: &str, values: &[String]) {
    if let Some(list) = fd.get_mut(key).and_then(Value::as_array_mut) {
        for v in values.iter().filter(|v| !v.is_empty()) {
            if !list.iter().any(|x| x == v.as_str()) {
                list.push(Value::String(v.clone()));
            }
        }
    }
}

/// The id of the library's author with exactly this name (`Database.getAuthorIdByName`): from the cached filter
/// data when the library is loaded, else from the database.
pub fn author_id_by_name(db: &dyn Db, library_id: &str, name: &str) -> Result<Option<String>> {
    named_id(db, library_id, "authors", name)
}

/// The id of the library's series with exactly this name (`Database.getSeriesIdByName`).
pub fn series_id_by_name(db: &dyn Db, library_id: &str, name: &str) -> Result<Option<String>> {
    named_id(db, library_id, "series", name)
}

/// Looks up a name in the cached filter list `key`, or in the table of that name.
fn named_id(db: &dyn Db, library_id: &str, key: &str, name: &str) -> Result<Option<String>> {
    let cached = FILTER_DATA.lock().ok().and_then(|c| c.as_ref().and_then(|m| m.get(library_id).cloned()));
    if let Some(fd) = cached {
        return Ok(fd[key].as_array().and_then(|l| l.iter().find(|v| v["name"] == name)).and_then(|v| v["id"].as_str().map(String::from)));
    }
    // Without the cache the original matches the name without regard to case (`getByNameAndLibrary`).
    let row = db.one(
        &format!("SELECT id FROM `{key}` WHERE lower(name) = ? AND libraryId = ? LIMIT 1"),
        vec![Value::from(name.to_lowercase()), Value::from(library_id)],
    )?;
    Ok(row.and_then(|r| r.get("id").and_then(Value::as_str).map(String::from)))
}

/// Counts a library's missing or invalid items again into its cached filter data, when it is cached
/// (`Database.resetLibraryIssuesFilterData`, after a scan).
pub fn reset_issues(db: &dyn Db, library_id: &str) -> Result<()> {
    let row = db.one(
        "SELECT count(*) AS n FROM libraryItems WHERE libraryId = ? AND (isMissing = 1 OR isInvalid = 1)",
        vec![Value::from(library_id)],
    )?;
    let n = row.and_then(|r| r.get("n").and_then(Value::as_i64)).unwrap_or(0);
    with_filter_data(library_id, |fd| fd["numIssues"] = Value::from(n));
    Ok(())
}

/// Forgets every cached filter data (when libraries change).
pub fn clear_filter_data() {
    if let Ok(mut c) = FILTER_DATA.lock() {
        *c = None;
    }
}

/// `GET /api/libraries/:id` (A0003, `findOne`): the library, or with `include=filterdata` the filter menus, the
/// number of issues and of the user's playlists too.
async fn find_one(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let lib = match library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let run = || -> Result<Value> {
        let lib_json = library_json(&lib, &folders(&st.abs, &id)?);
        if q.get("include").is_some_and(|i| i.split(',').any(|x| x == "filterdata")) {
            let data = filter_data(&st.abs, lib.media_type.as_deref().unwrap_or("book"), &id)?;
            let playlists = st
                .abs
                .one("SELECT count(*) AS n FROM playlists WHERE userId = ? AND libraryId = ?", vec![json!(user.id), json!(id)])?
                .and_then(|r| r.get("n").and_then(Value::as_i64))
                .unwrap_or(0);
            return Ok(json!({
                "filterdata": data,
                "issues": data["numIssues"],
                "numUserPlaylists": playlists,
                "library": lib_json,
            }));
        }
        Ok(lib_json)
    };
    run().map(Json).map_err(fail).into_response()
}

/// `GET /api/libraries/:id/items` (A0006, `getLibraryItems`): one page of a library, filtered and sorted, every
/// item minified.
async fn library_items(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let lib = match library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let run = || -> Result<Value> {
        let include: Vec<String> =
            q.get("include").map(|i| i.split(',').map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect()).unwrap_or_default();
        let limit: i64 = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(0);
        let page: i64 = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(0);
        let filter_by = q.get("filter").cloned();
        let (group, value) = filters::split_filter(filter_by.as_deref());
        let query = filters::Query {
            group,
            value,
            sort_by: q.get("sort").cloned(),
            sort_desc: q.get("desc").map(String::as_str) == Some("1"),
            collapse_series: q.get("collapseseries").map(String::as_str) == Some("1"),
            include: include.clone(),
            limit,
            offset: page * limit,
            home_page: false,
        };
        let sorting = Sorting::of(&st.abs_server.settings());
        let media_type = lib.media_type.clone().unwrap_or_default();
        let (results, total) = if media_type == "book" {
            let (items, total) = filters::book_items(&st.abs, &id, &user, &query, &sorting)?;
            (items.iter().map(|p| page_item_json(p, &sorting)).collect::<Vec<_>>(), total)
        } else {
            (Vec::new(), 0)
        };
        let mut payload = Map::new();
        payload.insert("results".into(), Value::Array(results));
        payload.insert("total".into(), Value::from(total));
        payload.insert("limit".into(), Value::from(limit));
        payload.insert("page".into(), Value::from(page));
        if let Some(sort) = q.get("sort") {
            payload.insert("sortBy".into(), Value::from(sort.as_str()));
        }
        payload.insert("sortDesc".into(), Value::from(query.sort_desc));
        if let Some(f) = filter_by {
            payload.insert("filterBy".into(), Value::from(f));
        }
        payload.insert("mediaType".into(), Value::from(media_type));
        payload.insert("minified".into(), Value::from(q.get("minified").map(String::as_str) == Some("1")));
        payload.insert("collapseseries".into(), Value::from(query.collapse_series));
        payload.insert("include".into(), Value::from(include.join(",")));
        payload.insert("offset".into(), Value::from(query.offset));
        Ok(Value::Object(payload))
    };
    run().map(Json).map_err(fail).into_response()
}

/// `GET /api/libraries/:id/personalized` (A0013, `getUserPersonalizedShelves`): the home page shelves of the
/// library for the user, `limit` entities each (10 by default).
async fn personalized(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let lib = match library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let limit = q.get("limit").filter(|v| !v.is_empty()).and_then(|v| v.parse::<i64>().ok()).unwrap_or(10);
    let include: Vec<String> =
        q.get("include").map(|i| i.split(',').map(|v| v.trim().to_lowercase()).filter(|v| !v.is_empty()).collect()).unwrap_or_default();
    let sorting = Sorting::of(&st.abs_server.settings());
    if lib.media_type.as_deref() != Some("book") {
        // Podcast shelves come with the podcast port (P5).
        return Json(json!([])).into_response();
    }
    match shelves::book_shelves(&st.abs, &lib, &user, &include, limit, &sorting) {
        Ok(s) => Json(Value::Array(s)).into_response(),
        Err(e) => fail(e),
    }
}

/// One item of a library page (`LibraryItem.getByFilterAndSort`): minified, with the collapsed series, the
/// filtered series, and the item's size when the book has none.
pub fn page_item_json(p: &filters::PageItem, sorting: &Sorting) -> Value {
    let mut j = models::library_item::minified(&p.item, &p.book, sorting);
    if let Some(c) = &p.collapsed_series {
        j.insert("collapsedSeries".into(), c.clone());
    }
    if let Some(Value::Object(media)) = j.get_mut("media") {
        if let Some(series) = &p.series {
            if let Some(Value::Object(meta)) = media.get_mut("metadata") {
                meta.insert("series".into(), series.clone());
            }
        }
        let size = p.item.size.unwrap_or(0);
        let media_size = media.get("size").and_then(Value::as_f64).unwrap_or(0.0);
        if size != 0 && media_size == 0.0 {
            media.insert("size".into(), Value::from(size));
        }
    }
    Value::Object(j)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_numbers_like_javascript() {
        assert_eq!(js_number("5"), Some(5.0));
        assert_eq!(js_number(""), Some(0.0));
        assert_eq!(js_number(" 2 "), Some(2.0));
        assert_eq!(js_number("1e2"), Some(100.0));
        assert_eq!(js_number("0x10"), Some(16.0));
        assert_eq!(js_number("abc"), None);
        assert_eq!(js_number("-1"), Some(-1.0));
    }
}
