//! An Audiobookshelf-compatible API, so the Audiobookshelf apps people already use (the official phone app,
//! ShelfPlayer, Plappa, Lissen …) play Folio's audiobooks and read its ebooks, with their place kept in Folio.
//! Written fresh from Audiobookshelf's public API documentation (api.audiobookshelf.org) and the shapes its
//! apps read; no Audiobookshelf code is used.
//!
//! The apps are given Folio's own address (`https://<folio>`); `https://<folio>/abs` works too. Every route
//! answers at both.
//!
//! ```text
//! GET  /ping, /healthcheck, /status            is this a server, and how to sign in
//! POST /login {username, password}             → {user (with its token), userDefaultLibraryId, serverSettings}
//! POST /logout, POST /api/authorize            end the token; who the token is
//! GET  /api/libraries                          Audiobooks and Ebooks
//! GET  /api/libraries/{id}[?include=filterdata], /filterdata, /personalized, /items, /series, /authors, /search
//! GET  /api/items/{id}[?expanded=1&include=progress], /cover, /file/{ino}[/download], /ebook[/{ino}]
//! POST /api/items/{id}/play                    a playback session: the tracks and where to start
//! POST /api/session/{id}/sync, /close          the place while playing (kept in Folio's progress)
//! POST /api/session/local, /local-all          places played offline
//! GET  /api/me, /api/me/progress/{id}, /api/me/items-in-progress, /api/me/listening-stats
//! PATCH /api/me/progress/{id}, /api/me/progress/batch/update; POST /api/me/sync-local-progress
//! POST|PATCH /api/me/item/{id}/bookmark; DELETE /api/me/item/{id}/bookmark/{time}
//! GET  /api/series/{id}, /api/authors/{id}
//! ```
//!
//! Places are Folio's progress (`playback::store`), so a chapter heard in an Audiobookshelf app is where the web
//! player and the Android app carry on, and the other way round.

pub mod play;
pub mod shape;

use super::model::Item;
use super::shelf::{self, Who};
use crate::AppState;
use axum::extract::{FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt};
use serde::Deserialize;
use serde_json::{json, Value};
use shape::{AUDIOBOOKS, EBOOKS};
use std::collections::BTreeMap;

/// The routes, at the root and again under `/abs`.
pub fn router() -> Router<AppState> {
    Router::new().merge(routes()).nest("/abs", routes())
}

/// Every route once.
fn routes() -> Router<AppState> {
    Router::new()
        .route("/ping", get(|| async { Json(json!({ "success": true })) }))
        .route("/healthcheck", get(|| async { Json(json!({ "success": true })) }))
        .route("/status", get(status))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/api/authorize", post(authorize))
        .route("/api/libraries", get(libraries))
        .route("/api/libraries/{id}", get(library))
        .route("/api/libraries/{id}/filterdata", get(filterdata_route))
        .route("/api/libraries/{id}/personalized", get(personalized))
        .route("/api/libraries/{id}/items", get(items))
        .route("/api/libraries/{id}/series", get(series_list))
        .route("/api/libraries/{id}/authors", get(authors))
        .route("/api/libraries/{id}/search", get(search))
        .route("/api/libraries/{id}/collections", get(empty_list))
        .route("/api/libraries/{id}/playlists", get(empty_list))
        .route("/api/playlists", get(empty_list))
        .route("/api/items/{id}", get(item))
        .route("/api/items/{id}/cover", get(cover))
        .route("/api/items/{id}/file/{ino}", get(file))
        .route("/api/items/{id}/file/{ino}/download", get(file))
        .route("/api/items/{id}/download", get(ebook))
        .route("/api/items/{id}/ebook", get(ebook))
        .route("/api/items/{id}/ebook/{ino}", get(ebook_file))
        .route("/api/series/{id}", get(one_series))
        .route("/api/authors/{id}", get(one_author))
        .route("/api/authors/{id}/image", get(|| async { StatusCode::NOT_FOUND }))
        .route("/api/me", get(me))
        .route("/api/me/items-in-progress", get(in_progress))
        .route("/api/me/listening-stats", get(play::listening_stats))
        .route("/api/me/listening-sessions", get(play::listening_sessions))
        .route("/api/me/progress/batch/update", patch(play::batch_progress))
        .route("/api/me/progress/{id}", get(play::get_progress).patch(play::set_progress).delete(play::clear_progress))
        .route("/api/me/progress/{id}/remove-from-continue-listening", get(ok).post(ok))
        .route("/api/me/series/{id}/remove-from-continue-listening", get(ok).post(ok))
        .route("/api/me/sync-local-progress", post(play::sync_local_progress))
        .route("/api/me/item/{id}/bookmark", post(play::add_bookmark).patch(play::edit_bookmark))
        .route("/api/me/item/{id}/bookmark/{time}", delete(play::delete_bookmark))
        .route("/api/items/{id}/play", post(play::start))
        .route("/api/session/local", post(play::local))
        .route("/api/session/local-all", post(play::local_all))
        .route("/api/session/{id}/sync", post(play::sync))
        .route("/api/session/{id}/close", post(play::close))
}

/// The account of an Audiobookshelf request: `Authorization: Bearer <token>`, `?token=` (apps add it to file
/// and cover addresses), or how the rest of Folio signs in.
#[derive(Debug, Clone)]
pub struct Abs {
    /// the account
    pub who: Who,
    /// the account's role: `owner`, `admin` or `user`
    pub role: String,
    /// the token the request came with, when it came with one
    pub token: Option<String>,
}

impl FromRequestParts<AppState> for Abs {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> std::result::Result<Abs, Response> {
        let query_token =
            parts.uri.query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")).map(super::epub::percent_decode));
        let token = crate::auth::bearer(&parts.headers).or(query_token);
        let unauthorized = || (StatusCode::UNAUTHORIZED, Json(json!({ "detail": "Sign in to Folio again in this app." }))).into_response();
        if let Some(t) = &token {
            let row = st
                .db
                .one(
                    "SELECT u.id, u.username, u.role FROM sessions s JOIN users u ON u.id=s.user_id
                     WHERE s.token=? AND s.kind='app' AND s.expires_at > ? AND u.disabled=0",
                    p![crate::auth::app_token_key(t), super::now()],
                )
                .map_err(IntoResponse::into_response)?
                .ok_or_else(unauthorized)?;
            return Ok(Abs { who: Who { id: row.i("id"), username: row.s("username") }, role: row.s("role"), token });
        }
        let a = crate::auth::signed_in(st, parts).map_err(|_| unauthorized())?;
        let role = if a.owner {
            "owner"
        } else if a.admin {
            "admin"
        } else {
            "user"
        };
        Ok(Abs { who: Who { id: a.id, username: a.username }, role: role.into(), token: None })
    }
}

/// An empty answer that says it worked.
async fn ok() -> Json<Value> {
    Json(json!({ "success": true }))
}

/// An empty list of collections or playlists (Folio has none).
async fn empty_list() -> Json<Value> {
    Json(json!({ "results": [], "total": 0, "limit": 0, "page": 0 }))
}

/// The server's settings as the apps read them.
fn server_settings() -> Value {
    json!({
        "id": "server-settings",
        "scannerFindCovers": false,
        "scannerParseSubtitle": false,
        "storeCoverWithItem": true,
        "storeMetadataWithItem": false,
        "rateLimitLoginRequests": 10,
        "rateLimitLoginWindow": 300000,
        "backupSchedule": false,
        "loggerDailyLogsToKeep": 7,
        "loggerScannerLogsToKeep": 2,
        "homeBookshelfView": 1,
        "bookshelfView": 1,
        "sortingIgnorePrefix": true,
        "sortingPrefixes": ["the", "a", "an"],
        "chromecastEnabled": false,
        "dateFormat": "yyyy-MM-dd",
        "timeFormat": "HH:mm",
        "language": "en-us",
        "logLevel": 2,
        "version": shape::SERVER_VERSION,
        "buildNumber": 1,
        "authActiveAuthMethods": ["local"],
        "authOpenIDAutoLaunch": false,
        "authOpenIDAutoRegister": false
    })
}

/// `GET /status`.
async fn status() -> Json<Value> {
    Json(json!({
        "app": "audiobookshelf",
        "serverVersion": shape::SERVER_VERSION,
        "isInit": true,
        "language": "en-us",
        "authMethods": ["local"],
        "authFormData": {}
    }))
}

/// The user as the apps read it, with every place and bookmark; `token` is the token the app signs in with.
fn user(st: &AppState, a: &Abs, token: &str) -> Result<Value> {
    let items = shelf::items(st, &a.who)?;
    let by_id: BTreeMap<i64, &Item> = items.iter().map(|i| (i.id, i)).collect();
    let progress: Vec<Value> = crate::playback::store::all_progress(&st.db, a.who.id, None)?
        .iter()
        .filter_map(|p| by_id.get(&p.edition_id).map(|it| shape::progress(a.who.id, it, p)))
        .collect();
    let mut bookmarks = Vec::new();
    for it in items.iter().filter(|i| i.is_audio()) {
        bookmarks.extend(crate::playback::store::bookmarks(&st.db, a.who.id, it.id)?.iter().map(shape::bookmark));
    }
    let created = st.db.one("SELECT created_at FROM users WHERE id=?", p![a.who.id])?.map(|r| r.f("created_at")).unwrap_or(0.0);
    let kind = match a.role.as_str() {
        "owner" => "root",
        "admin" => "admin",
        _ => "user",
    };
    let admin = kind != "user";
    Ok(json!({
        "id": a.who.id.to_string(),
        "username": a.who.username,
        "email": null,
        "type": kind,
        "token": token,
        "accessToken": token,
        "mediaProgress": progress,
        "seriesHideFromContinueListening": [],
        "bookmarks": bookmarks,
        "isActive": true,
        "isLocked": false,
        "lastSeen": shape::ms(super::now()),
        "createdAt": shape::ms(created),
        "permissions": {
            "download": true,
            "update": admin,
            "delete": false,
            "upload": false,
            "createEreader": false,
            "accessAllLibraries": true,
            "accessAllTags": true,
            "accessExplicitContent": true
        },
        "librariesAccessible": [],
        "itemTagsSelected": [],
        "hasOpenIDLink": false
    }))
}

/// What `/login` and `/api/authorize` answer.
fn signed_in(st: &AppState, a: &Abs, token: &str) -> Result<Value> {
    Ok(json!({
        "user": user(st, a, token)?,
        "userDefaultLibraryId": AUDIOBOOKS,
        "serverSettings": server_settings(),
        "ereaderDevices": [],
        "Source": "docker"
    }))
}

/// `POST /login` body.
#[derive(Debug, Deserialize)]
pub struct Login {
    /// the Folio username
    #[serde(default)]
    pub username: String,
    /// the Folio password
    #[serde(default)]
    pub password: String,
}

/// `POST /login`: sign in with the Folio username and password; a new app token for this app.
async fn login(State(st): State<AppState>, headers: HeaderMap, Json(b): Json<Login>) -> Result<Json<Value>> {
    let who = shelf::password_user(&st, &b.username, &b.password)
        .await
        .ok_or_else(|| err(401, "The username or password is wrong. Check them and try again."))?;
    let role = st.db.one("SELECT role FROM users WHERE id=?", p![who.id])?.map(|r| r.s("role")).unwrap_or_default();
    let token = crate::auth::random_token(32);
    st.db.execute(
        "INSERT INTO sessions (token, user_id, kind, created_at, expires_at) VALUES (?,?,'app',?,?)",
        p![crate::auth::app_token_key(&token), who.id, super::now(), super::now() + crate::auth::APP_SESSION_SECS],
    )?;
    let _ = headers;
    let a = Abs { who, role, token: Some(token.clone()) };
    Ok(Json(signed_in(&st, &a, &token)?))
}

/// `POST /logout`: the token stops working.
async fn logout(State(st): State<AppState>, a: Abs) -> Result<Json<Value>> {
    if let Some(t) = &a.token {
        st.db.execute("DELETE FROM sessions WHERE token=? AND kind='app'", p![crate::auth::app_token_key(t)])?;
    }
    Ok(Json(json!({ "success": true })))
}

/// `POST /api/authorize`: who the token is.
async fn authorize(State(st): State<AppState>, a: Abs) -> Result<Json<Value>> {
    let token = a.token.clone().unwrap_or_default();
    Ok(Json(signed_in(&st, &a, &token)?))
}

/// `GET /api/me`.
async fn me(State(st): State<AppState>, a: Abs) -> Result<Json<Value>> {
    let token = a.token.clone().unwrap_or_default();
    Ok(Json(user(&st, &a, &token)?))
}

/// The items of library `id` (404 for another id).
fn library_items(st: &AppState, a: &Abs, id: &str) -> Result<Vec<Item>> {
    let audio = match id {
        AUDIOBOOKS => true,
        EBOOKS => false,
        _ => return Err(err(404, "There is no library like that. Pick Audiobooks or Ebooks.")),
    };
    Ok(shelf::items(st, &a.who)?.into_iter().filter(|i| i.is_audio() == audio).collect())
}

/// `GET /api/libraries`.
async fn libraries(_a: Abs) -> Json<Value> {
    Json(json!({ "libraries": [shape::library(AUDIOBOOKS), shape::library(EBOOKS)] }))
}

/// The authors, series, genres, narrators and languages of `items`, for the apps' filters.
fn filterdata(items: &[Item]) -> Value {
    let mut authors = BTreeMap::new();
    let mut series = BTreeMap::new();
    let mut genres = std::collections::BTreeSet::new();
    let mut narrators = std::collections::BTreeSet::new();
    let mut languages = std::collections::BTreeSet::new();
    let mut publishers = std::collections::BTreeSet::new();
    for it in items {
        for a in &it.meta.authors {
            authors.insert(shape::author_id(&a.name), a.name.clone());
        }
        for s in &it.meta.series {
            series.insert(shape::series_id(&s.name), s.name.clone());
        }
        genres.extend(it.meta.genres.iter().cloned());
        narrators.extend(it.meta.narrators.iter().cloned());
        if !it.meta.language.is_empty() {
            languages.insert(it.meta.language.clone());
        }
        if !it.meta.publisher.is_empty() {
            publishers.insert(it.meta.publisher.clone());
        }
    }
    json!({
        "authors": authors.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "series": series.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "genres": genres,
        "tags": [],
        "narrators": narrators,
        "languages": languages,
        "publishers": publishers,
        "publishedDecades": []
    })
}

/// `?include=` and the list options of the library routes.
#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    /// extra parts to include, comma separated (`filterdata`, `progress`)
    #[serde(default)]
    pub include: String,
    /// how many on a page; 0 for all
    #[serde(default)]
    pub limit: Option<usize>,
    /// the page, from 0
    #[serde(default)]
    pub page: Option<usize>,
    /// what to sort by (`media.metadata.title`, `addedAt` …)
    #[serde(default)]
    pub sort: String,
    /// `1` for the other way round
    #[serde(default)]
    pub desc: Option<String>,
    /// `<group>.<base64 value>`: only the items in it
    #[serde(default)]
    pub filter: String,
    /// the words searched for
    #[serde(default)]
    pub q: String,
    /// `1` for whole items in a list
    #[serde(default)]
    pub expanded: Option<String>,
}

impl ListQuery {
    /// Whether `part` is in `include`.
    fn includes(&self, part: &str) -> bool {
        self.include.split(',').any(|p| p.trim() == part)
    }

    /// Whether the list goes the other way round.
    fn descending(&self) -> bool {
        matches!(self.desc.as_deref(), Some("1" | "true"))
    }
}

/// `GET /api/libraries/{id}`.
async fn library(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let items = library_items(&st, &a, &id)?;
    if q.includes("filterdata") {
        return Ok(Json(json!({ "filterdata": filterdata(&items), "issues": 0, "numUserPlaylists": 0, "library": shape::library(&id) })));
    }
    Ok(Json(shape::library(&id)))
}

/// `GET /api/libraries/{id}/filterdata`.
async fn filterdata_route(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    Ok(Json(filterdata(&library_items(&st, &a, &id)?)))
}

/// The person's places by edition.
fn places(st: &AppState, a: &Abs) -> Result<BTreeMap<i64, crate::playback::store::Progress>> {
    Ok(crate::playback::store::all_progress(&st.db, a.who.id, None)?.into_iter().map(|p| (p.edition_id, p)).collect())
}

/// A list item, with the person's place in it when `with_progress`.
fn list_item(st: &AppState, a: &Abs, it: &Item, expanded: bool, place: Option<&crate::playback::store::Progress>) -> Value {
    let root = shelf::library_dir(st);
    let mut v = shape::item(it, &root, !expanded);
    if let Some(p) = place {
        v["userMediaProgress"] = shape::progress(a.who.id, it, p);
        v["progressLastUpdate"] = json!(shape::ms(p.updated_at));
    }
    v
}

/// `GET /api/libraries/{id}/personalized`: the home screen's shelves.
async fn personalized(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let items = library_items(&st, &a, &id)?;
    let places = places(&st, &a)?;
    let audio = id == AUDIOBOOKS;
    let mut going: Vec<&Item> = items.iter().filter(|i| places.get(&i.id).is_some_and(|p| !p.finished && p.fraction > 0.0)).collect();
    going.sort_by(|x, y| places[&y.id].updated_at.total_cmp(&places[&x.id].updated_at));
    let mut newest: Vec<&Item> = items.iter().collect();
    newest.sort_by(|x, y| y.added_at.total_cmp(&x.added_at).then(y.id.cmp(&x.id)));
    newest.truncate(10);
    let mut again: Vec<&Item> = items.iter().filter(|i| places.get(&i.id).is_some_and(|p| p.finished)).collect();
    again.sort_by(|x, y| places[&y.id].updated_at.total_cmp(&places[&x.id].updated_at));
    let shelf_of = |sid: &str, label: &str, key: &str, list: &[&Item]| {
        json!({
            "id": sid,
            "label": label,
            "labelStringKey": key,
            "type": "book",
            "entities": list.iter().map(|it| list_item(&st, &a, it, false, places.get(&it.id))).collect::<Vec<_>>(),
            "total": list.len()
        })
    };
    let mut shelves = Vec::new();
    if !going.is_empty() {
        if audio {
            shelves.push(shelf_of("continue-listening", "Continue Listening", "LabelContinueListening", &going));
        } else {
            shelves.push(shelf_of("continue-reading", "Continue Reading", "LabelContinueReading", &going));
        }
    }
    if !newest.is_empty() {
        shelves.push(shelf_of("recently-added", "Recently Added", "LabelRecentlyAdded", &newest));
    }
    if !again.is_empty() {
        let (sid, label, key) =
            if audio { ("listen-again", "Listen Again", "LabelListenAgain") } else { ("read-again", "Read Again", "LabelReadAgain") };
        shelves.push(shelf_of(sid, label, key, &again));
    }
    Ok(Json(json!(shelves)))
}

/// Whether `it` is in the filter `<group>.<base64 value>`.
fn in_filter(it: &Item, filter: &str, place: Option<&crate::playback::store::Progress>) -> bool {
    use base64::Engine;
    let Some((group, value)) = filter.split_once('.') else { return true };
    let value = super::epub::percent_decode(value);
    let value =
        base64::engine::general_purpose::STANDARD.decode(value.trim()).ok().and_then(|b| String::from_utf8(b).ok()).unwrap_or(value);
    let m = &it.meta;
    match group {
        "authors" => m.authors.iter().any(|a| shape::author_id(&a.name) == value),
        "series" => m.series.iter().any(|s| shape::series_id(&s.name) == value),
        "genres" => m.genres.iter().any(|g| g == &value),
        "narrators" => m.narrators.iter().any(|n| n == &value),
        "languages" => m.language == value,
        "publishers" => m.publisher == value,
        "progress" => match value.as_str() {
            "finished" => place.is_some_and(|p| p.finished),
            "not-started" => place.is_none_or(|p| p.fraction <= 0.0 && !p.finished),
            "in-progress" => place.is_some_and(|p| p.fraction > 0.0 && !p.finished),
            "not-finished" => !place.is_some_and(|p| p.finished),
            _ => true,
        },
        "ebooks" => !it.is_audio() || value == "supplementary",
        _ => true,
    }
}

/// One page of `list` (page from 0; limit 0 is everything); the page and the whole count.
fn page<T: Clone>(list: &[T], limit: Option<usize>, page: Option<usize>) -> (Vec<T>, usize, usize) {
    let limit = limit.unwrap_or(0);
    if limit == 0 {
        return (list.to_vec(), 0, 0);
    }
    let page = page.unwrap_or(0);
    (list.iter().skip(page * limit).take(limit).cloned().collect(), limit, page)
}

/// `GET /api/libraries/{id}/items`: the library's items, filtered, sorted and a page of them.
async fn items(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let places = places(&st, &a)?;
    let mut list: Vec<Item> = library_items(&st, &a, &id)?.into_iter().filter(|i| in_filter(i, &q.filter, places.get(&i.id))).collect();
    let title = |i: &Item| crate::library::text::title_key(&i.meta.title);
    match q.sort.as_str() {
        "addedAt" => list.sort_by(|x, y| x.added_at.total_cmp(&y.added_at)),
        "updatedAt" | "mtimeMs" => list.sort_by(|x, y| x.updated_at.total_cmp(&y.updated_at)),
        "media.duration" => list.sort_by(|x, y| x.duration().total_cmp(&y.duration())),
        "size" => list.sort_by_key(Item::size),
        "media.metadata.publishedYear" => list.sort_by_key(|i| i.meta.year()),
        "media.metadata.authorName" | "media.metadata.authorNameLF" => {
            list.sort_by_key(|i| i.meta.authors.first().map(|a| crate::library::text::sort_name(&a.name).to_lowercase()))
        }
        "progress" => list.sort_by(|x, y| {
            let at = |i: &Item| places.get(&i.id).map(|p| p.updated_at).unwrap_or(0.0);
            at(x).total_cmp(&at(y))
        }),
        _ => list.sort_by_key(title),
    }
    if q.descending() {
        list.reverse();
    }
    let total = list.len();
    let (shown, limit, pg) = page(&list, q.limit, q.page);
    let expanded = matches!(q.expanded.as_deref(), Some("1" | "true"));
    let results: Vec<Value> = shown.iter().map(|it| list_item(&st, &a, it, expanded, places.get(&it.id))).collect();
    Ok(Json(json!({
        "results": results,
        "total": total,
        "limit": limit,
        "page": pg,
        "sortBy": q.sort,
        "sortDesc": q.descending(),
        "filterBy": q.filter,
        "mediaType": "book",
        "minified": !expanded,
        "collapseseries": false,
        "include": q.include
    })))
}

/// A series' name and its books, each with its place in the series.
type SeriesBooks<'a> = (String, Vec<(&'a Item, String)>);

/// The series in `items`, each with its books in order.
fn series_of(items: &[Item]) -> Vec<SeriesBooks<'_>> {
    let mut by: BTreeMap<String, SeriesBooks<'_>> = BTreeMap::new();
    for it in items {
        for s in &it.meta.series {
            by.entry(shape::series_id(&s.name)).or_insert_with(|| (s.name.clone(), Vec::new())).1.push((it, s.position.clone()));
        }
    }
    let mut out: Vec<SeriesBooks<'_>> = by.into_values().collect();
    for (_, books) in &mut out {
        let place = |p: &str| p.split(['-', ' ']).next().and_then(|n| n.parse::<f64>().ok()).unwrap_or(f64::MAX);
        books.sort_by(|x, y| place(&x.1).total_cmp(&place(&y.1)));
    }
    out.sort_by_key(|(name, _)| crate::library::text::title_key(name));
    out
}

/// A series with its books, as the series list gives it.
fn series_entry(st: &AppState, a: &Abs, library: &str, name: &str, books: &[(&Item, String)]) -> Value {
    let mut v = shape::series(name, library);
    let list: Vec<Value> = books
        .iter()
        .map(|(it, seq)| {
            let mut b = list_item(st, a, it, false, None);
            b["sequence"] = json!(seq);
            b
        })
        .collect();
    v["books"] = json!(list);
    v["numBooks"] = json!(books.len());
    v
}

/// `GET /api/libraries/{id}/series`.
async fn series_list(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let items = library_items(&st, &a, &id)?;
    let all: Vec<Value> = series_of(&items).iter().map(|(name, books)| series_entry(&st, &a, &id, name, books)).collect();
    let total = all.len();
    let (results, limit, pg) = page(&all, q.limit, q.page);
    Ok(Json(
        json!({ "results": results, "total": total, "limit": limit, "page": pg, "sortBy": q.sort, "sortDesc": q.descending(), "filterBy": q.filter, "minified": true, "include": q.include }),
    ))
}

/// The authors of `items` with how many books each, in sort-name order.
fn authors_of(items: &[Item]) -> Vec<(String, usize)> {
    let mut by: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for it in items {
        for a in &it.meta.authors {
            by.entry(shape::author_id(&a.name)).or_insert_with(|| (a.name.clone(), 0)).1 += 1;
        }
    }
    let mut out: Vec<(String, usize)> = by.into_values().collect();
    out.sort_by_key(|(n, _)| crate::library::text::sort_name(n).to_lowercase());
    out
}

/// `GET /api/libraries/{id}/authors`.
async fn authors(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let items = library_items(&st, &a, &id)?;
    Ok(Json(json!({ "authors": authors_of(&items).iter().map(|(n, c)| shape::author(n, *c, &id)).collect::<Vec<_>>() })))
}

/// `GET /api/libraries/{id}/search?q=`.
async fn search(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let items = library_items(&st, &a, &id)?;
    let words = crate::library::text::fold(&q.q);
    let limit = q.limit.unwrap_or(12).max(1);
    let found: Vec<Value> = items
        .iter()
        .filter(|i| super::opds::matches(i, &q.q))
        .take(limit)
        .map(|it| json!({ "libraryItem": list_item(&st, &a, it, true, None), "matchKey": "title", "matchText": it.meta.title }))
        .collect();
    let hit = |name: &str| !words.is_empty() && crate::library::text::fold(name).contains(&words);
    let authors: Vec<Value> =
        authors_of(&items).iter().filter(|(n, _)| hit(n)).take(limit).map(|(n, c)| shape::author(n, *c, &id)).collect();
    let series: Vec<Value> = series_of(&items)
        .iter()
        .filter(|(n, _)| hit(n))
        .take(limit)
        .map(|(n, books)| json!({ "series": shape::series(n, &id), "books": books.iter().map(|(it, _)| list_item(&st, &a, it, false, None)).collect::<Vec<_>>() }))
        .collect();
    let mut narrators: BTreeMap<String, usize> = BTreeMap::new();
    for it in &items {
        for n in it.meta.narrators.iter().filter(|n| hit(n)) {
            *narrators.entry(n.clone()).or_default() += 1;
        }
    }
    Ok(Json(json!({
        "book": found,
        "authors": authors,
        "series": series,
        "narrators": narrators.into_iter().map(|(name, n)| json!({ "name": name, "numBooks": n })).collect::<Vec<_>>(),
        "tags": [],
        "genres": []
    })))
}

/// The item of an Audiobookshelf item id that `a` can see (404 otherwise).
pub fn find(st: &AppState, a: &Abs, id: &str) -> Result<Item> {
    let edition = shape::edition_of(id).ok_or_else(|| err(404, "That book isn't in the library."))?;
    shelf::item(st, &a.who, edition)
}

/// `GET /api/items/{id}`.
async fn item(State(st): State<AppState>, a: Abs, Path(id): Path<String>, Query(q): Query<ListQuery>) -> Result<Json<Value>> {
    let it = find(&st, &a, &id)?;
    let expanded = !matches!(q.expanded.as_deref(), Some("0" | "false"));
    let place = if q.includes("progress") { crate::playback::store::progress(&st.db, a.who.id, it.id)? } else { None };
    Ok(Json(list_item(&st, &a, &it, expanded, place.as_ref())))
}

/// `GET /api/items/{id}/cover`.
async fn cover(State(st): State<AppState>, a: Abs, Path(id): Path<String>, req: Request) -> Result<Response> {
    let it = find(&st, &a, &id)?;
    match &it.cover {
        Some((path, mime)) => super::send_file(path, mime, None, req).await,
        None => Err(err(404, "This book has no cover.")),
    }
}

/// `GET /api/items/{id}/file/{ino}[/download]`.
async fn file(State(st): State<AppState>, a: Abs, Path(params): Path<Vec<(String, String)>>, req: Request) -> Result<Response> {
    let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let it = find(&st, &a, &get("id"))?;
    let fid = shape::file_of(&get("ino")).ok_or_else(|| err(404, "That file isn't in the library."))?;
    let f = it.files.iter().find(|f| f.id == fid).ok_or_else(|| err(404, "That file isn't in the library."))?;
    let download = req.uri().path().ends_with("/download").then_some(f.name.as_str());
    super::send_file(&f.path, &f.mime, download, req).await
}

/// `GET /api/items/{id}/ebook`: the ebook's file.
async fn ebook(State(st): State<AppState>, a: Abs, Path(id): Path<String>, req: Request) -> Result<Response> {
    let it = find(&st, &a, &id)?;
    let f = it.ebook_file().ok_or_else(|| err(404, "This book has no ebook file."))?;
    super::send_file(&f.path, &f.mime, Some(&f.name), req).await
}

/// `GET /api/items/{id}/ebook/{ino}`.
async fn ebook_file(State(st): State<AppState>, a: Abs, Path((id, ino)): Path<(String, String)>, req: Request) -> Result<Response> {
    let it = find(&st, &a, &id)?;
    let fid = shape::file_of(&ino).ok_or_else(|| err(404, "That file isn't in the library."))?;
    let f = it.files.iter().find(|f| f.id == fid && f.kind != "audio").ok_or_else(|| err(404, "That file isn't in the library."))?;
    super::send_file(&f.path, &f.mime, Some(&f.name), req).await
}

/// `GET /api/series/{id}`.
async fn one_series(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let items = shelf::items(&st, &a.who)?;
    let (name, books) = series_of(&items)
        .into_iter()
        .find(|(n, _)| shape::series_id(n) == id)
        .ok_or_else(|| err(404, "There is no series like that in the library."))?;
    let library = books.first().map(|(it, _)| shape::library_of(it)).unwrap_or(AUDIOBOOKS);
    Ok(Json(series_entry(&st, &a, library, &name, &books)))
}

/// `GET /api/authors/{id}`: the author, with their books and series.
async fn one_author(State(st): State<AppState>, a: Abs, Path(id): Path<String>) -> Result<Json<Value>> {
    let items: Vec<Item> =
        shelf::items(&st, &a.who)?.into_iter().filter(|i| i.meta.authors.iter().any(|x| shape::author_id(&x.name) == id)).collect();
    let name = items.iter().flat_map(|i| i.meta.authors.iter()).find(|x| shape::author_id(&x.name) == id).map(|x| x.name.clone());
    let name = name.ok_or_else(|| err(404, "There is no author like that in the library."))?;
    let library = items.first().map(shape::library_of).unwrap_or(AUDIOBOOKS);
    let mut v = shape::author(&name, items.len(), library);
    v["libraryItems"] = json!(items.iter().map(|it| list_item(&st, &a, it, false, None)).collect::<Vec<_>>());
    v["series"] = json!(series_of(&items).iter().map(|(n, books)| series_entry(&st, &a, library, n, books)).collect::<Vec<_>>());
    Ok(Json(v))
}

/// `GET /api/me/items-in-progress`.
async fn in_progress(State(st): State<AppState>, a: Abs) -> Result<Json<Value>> {
    let places = places(&st, &a)?;
    let mut items: Vec<Item> =
        shelf::items(&st, &a.who)?.into_iter().filter(|i| places.get(&i.id).is_some_and(|p| !p.finished && p.fraction > 0.0)).collect();
    items.sort_by(|x, y| places[&y.id].updated_at.total_cmp(&places[&x.id].updated_at));
    Ok(Json(json!({ "libraryItems": items.iter().map(|it| list_item(&st, &a, it, false, places.get(&it.id))).collect::<Vec<_>>() })))
}
