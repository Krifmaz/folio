//! The port of the original's user management (`server/controllers/UserController.js`), the password change
//! (`MeController.updatePassword` with `LocalAuthStrategy.changePassword`) and the sign-in settings
//! (`MiscController.getAuthSettings` / `updateAuthSettings`). Admins list, add, change and remove users; every
//! user may change their own password. Each answer, check and message is the original's (tests/abs_users.rs
//! compares them with the recorded answers).

use crate::abs::auth::routes::{body_fields, cookie, hash_password, refresh_cookie};
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, user, MediaProgress, User};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// The routes: `/api/users`, `/api/me/password` and `/api/auth-settings`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users", get(find_all).post(create))
        .route("/api/users/online", get(online))
        .route("/api/users/{id}", get(find_one).patch(update).delete(delete))
        .route("/api/users/{id}/openid-unlink", patch(unlink_openid))
        .route("/api/me/password", patch(update_password))
        .route("/api/auth-settings", get(auth_settings).patch(update_auth_settings))
}

/// A server error as an answer.
fn failed(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// The user a `/api/users/:id` address names, after the original's check (`UserController.middleware`): others
/// than admins may only read themselves, and only admins change anything. `Err` is the answer to send (403 or
/// 404).
fn req_user(st: &AppState, me: &User, method: &Method, id: &str) -> std::result::Result<User, Box<Response>> {
    let admin = user::is_admin_or_up(me);
    if (!admin && me.id.as_deref() != Some(id)) || (!admin && [Method::PATCH, Method::POST, Method::DELETE].contains(method)) {
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    match user::by_id(&st.abs, id) {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(Box::new(send_status(StatusCode::NOT_FOUND))),
        Err(e) => Err(Box::new(failed(e))),
    }
}

/// Whether a username is in use (`checkUserExistsWithUsername`, an exact match).
fn username_taken(db: &dyn Db, username: &str) -> Result<bool> {
    Ok(db.one("SELECT id FROM users WHERE username = ? LIMIT 1", vec![json!(username)])?.is_some())
}

/// A list of ids or tags from a request (`librariesAccessible`, `itemTagsSelected`), on the body or in its
/// permissions: `None` when missing, empty or not a list of texts (the original then ignores it).
fn text_list(body: &Map<String, Value>, key: &str) -> Option<Vec<Value>> {
    let v = Some(body.get(key))
        .filter(|v| crate::abs::settings::truthy(*v))
        .flatten()
        .or_else(|| body.get("permissions").and_then(|p| p.get(key)).filter(|v| crate::abs::settings::truthy(Some(v))))?;
    match v.as_array() {
        Some(a) if a.iter().all(Value::is_string) => Some(a.clone()),
        _ => {
            tracing::warn!("The {key} sent isn't a list of texts; it was ignored");
            None
        }
    }
}

/// The user as the apps know it, with their progress (`toOldJSONForBrowser`).
fn browser_json(db: &dyn Db, u: &User) -> Result<Value> {
    let progress = user::media_progresses(db, u.id.as_deref().unwrap_or_default())?;
    Ok(user::to_old_json_for_browser(u, &progress, None, false, false))
}

/// `GET /api/users`: every user, without progress and bookmarks (admins only; only root sees root's token).
async fn find_all(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let hide_root_token = !user::is_root(&me.user);
    match models::list::<User>(&st.abs, "1 = 1", "", vec![]) {
        Ok(all) => {
            let users: Vec<Value> = all.iter().map(|u| user::to_old_json_for_browser(u, &[], None, hide_root_token, true)).collect();
            Json(json!({ "users": users })).into_response()
        }
        Err(e) => failed(e),
    }
}

/// `GET /api/users/online`: who is connected, and the open playback sessions (admins only).
async fn online(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    Json(json!({ "usersOnline": st.sockets.users_online(), "openSessions": [] })).into_response()
}

/// A progress as the user page shows it: with the title, cover and change time of its book or episode's podcast
/// (`findOne`'s `displayTitle`, `displaySubtitle`, `coverPath`, `mediaUpdatedAt`).
fn progress_for_page(db: &dyn Db, mp: &MediaProgress) -> Result<Value> {
    let mut j = models::media_progress::old_json(mp);
    let id = mp.media_item_id.clone().unwrap_or_default();
    if mp.media_item_type.as_deref() == Some("podcastEpisode") {
        let row = db.one(
            "SELECT e.title AS title, p.title AS podcastTitle, p.coverPath AS coverPath, p.updatedAt AS updatedAt \
             FROM podcastEpisodes e LEFT JOIN podcasts p ON p.id = e.podcastId WHERE e.id = ?",
            vec![json!(id)],
        )?;
        if let Some(r) = row {
            j["displayTitle"] = r.get("title").cloned().unwrap_or(Value::Null);
            j["displaySubtitle"] = r.get("podcastTitle").cloned().unwrap_or(Value::Null);
            j["coverPath"] = r.get("coverPath").cloned().unwrap_or(Value::Null);
            j["mediaUpdatedAt"] = models::date_iso(r.get("updatedAt").and_then(Value::as_str));
        }
    } else if let Some(r) = db.one("SELECT title, coverPath, updatedAt FROM books WHERE id = ?", vec![json!(id)])? {
        j["displayTitle"] = r.get("title").cloned().unwrap_or(Value::Null);
        j["coverPath"] = r.get("coverPath").cloned().unwrap_or(Value::Null);
        j["mediaUpdatedAt"] = models::date_iso(r.get("updatedAt").and_then(Value::as_str));
    }
    Ok(j)
}

/// `GET /api/users/:id`: one user with their progress (admins only).
async fn find_one(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let target = match req_user(&st, &me.user, &Method::GET, &id) {
        Ok(u) => u,
        Err(r) => return *r,
    };
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to read a user but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let built = (|| -> Result<Value> {
        let progress = user::media_progresses(&st.abs, &id)?;
        let mut j = user::to_old_json_for_browser(&target, &[], None, !user::is_root(&me.user), false);
        j["mediaProgress"] = Value::Array(progress.iter().map(|mp| progress_for_page(&st.abs, mp)).collect::<Result<Vec<_>>>()?);
        Ok(j)
    })();
    match built {
        Ok(j) => Json(j).into_response(),
        Err(e) => failed(e),
    }
}

/// The permissions a request gives, checked like the original: known keys only, true or false only.
fn merge_permissions(into: &mut Map<String, Value>, known: &Value, sent: &Map<String, Value>) -> bool {
    let mut changed = false;
    for (k, v) in sent {
        if into.contains_key(k) || known.get(k).is_some() {
            if !v.is_boolean() {
                tracing::warn!("The permission {k} must be true or false; it was ignored");
            } else if into.get(k) != Some(v) {
                into.insert(k.clone(), v.clone());
                changed = true;
            }
        } else {
            tracing::warn!("There is no permission called {k}; it was ignored");
        }
    }
    changed
}

/// `POST /api/users`: adds a user (admins only) with a name, a password, a type and permissions.
async fn create(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let mut b = body_fields(&headers, &body);
    let (username, password) = match (b.get("username"), b.get("password")) {
        (Some(Value::String(u)), Some(Value::String(p))) if !u.is_empty() && !p.is_empty() => (u.clone(), p.clone()),
        _ => return send_text(StatusCode::BAD_REQUEST, "Username and password are required"),
    };
    let kind = b.get("type").filter(|t| crate::abs::settings::truthy(Some(t))).cloned();
    if let Some(t) = &kind {
        if !t.as_str().is_some_and(|t| user::ACCOUNT_TYPES.contains(&t)) {
            return send_text(StatusCode::BAD_REQUEST, "Invalid account type");
        }
    }
    match username_taken(&st.abs, &username) {
        Ok(true) => return send_text(StatusCode::BAD_REQUEST, "Username already taken"),
        Ok(false) => {}
        Err(e) => return failed(e),
    }
    let kind = kind.and_then(|t| t.as_str().map(str::to_string)).unwrap_or_else(|| "user".into());
    let libraries = text_list(&b, "librariesAccessible");
    let tags = text_list(&b, "itemTagsSelected");
    if let Some(Value::Object(p)) = b.get_mut("permissions") {
        p.remove("itemTagsSelected");
        p.remove("librariesAccessible");
    }
    let mut permissions = user::default_permissions(&kind).as_object().cloned().unwrap_or_default();
    if let Some(Value::Object(sent)) = b.get("permissions") {
        let defaults = Value::Object(permissions.clone());
        // only keys of the defaults count when adding a user
        let sent: Map<String, Value> = sent.clone();
        let mut p = permissions.clone();
        for (k, v) in &sent {
            if defaults.get(k).is_none() {
                tracing::warn!("There is no permission called {k}; it was ignored");
            } else if !v.is_boolean() {
                tracing::warn!("The permission {k} must be true or false; it was ignored");
            } else {
                p.insert(k.clone(), v.clone());
            }
        }
        permissions = p;
    }
    permissions.insert("itemTagsSelected".into(), Value::Array(tags.unwrap_or_default()));
    permissions.insert("librariesAccessible".into(), Value::Array(libraries.unwrap_or_default()));
    let id = models::new_id();
    let now = models::date_now();
    let new = User {
        token: Some(st.abs_server.tokens.legacy_token(&id, &username)),
        id: Some(id),
        kind: Some(kind),
        username: Some(username),
        email: b.get("email").and_then(Value::as_str).map(str::to_string),
        pash: Some(hash_password(&password)),
        is_active: Some(crate::abs::settings::truthy(b.get("isActive"))),
        is_locked: Some(false),
        permissions: Some(Value::Object(permissions)),
        bookmarks: Some(json!([])),
        extra_data: Some(json!({ "seriesHideFromContinueListening": [] })),
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..User::default()
    };
    if let Err(e) = models::insert(&st.abs, &new) {
        return failed(e);
    }
    let j = user::to_old_json_for_browser(&new, &[], None, false, false);
    st.sockets.admin_emitter("user_added", &j);
    Json(json!({ "user": j })).into_response()
}

/// `PATCH /api/users/:id`: changes a user (admins only; only root changes root, whose type never changes). A
/// new name makes a new old-style token; a new name or password signs the user out everywhere.
async fn update(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    let mut target = match req_user(&st, &me.user, &Method::PATCH, &id) {
        Ok(u) => u,
        Err(r) => return *r,
    };
    let mut b = body_fields(&headers, &body);
    if user::is_root(&target) && !user::is_root(&me.user) {
        tracing::error!("Admin \"{}\" tried to change the root user", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    } else if user::is_root(&target) {
        b.remove("type");
    }
    for key in ["id", "pash", "token", "extraData", "bookmarks"] {
        if b.contains_key(key) {
            return send_text(StatusCode::BAD_REQUEST, &format!("Key \"{key}\" cannot be updated"));
        }
    }
    let truthy = |k: &str| crate::abs::settings::truthy(b.get(k));
    if truthy("email") && !b["email"].is_string() {
        return send_text(StatusCode::BAD_REQUEST, "Invalid email");
    }
    if truthy("username") && !b["username"].is_string() {
        return send_text(StatusCode::BAD_REQUEST, "Invalid username");
    }
    if truthy("type") && !b["type"].as_str().is_some_and(|t| user::ACCOUNT_TYPES.contains(&t)) {
        return send_text(StatusCode::BAD_REQUEST, "Invalid account type");
    }
    if truthy("permissions") && !b["permissions"].is_object() && !b["permissions"].is_array() {
        return send_text(StatusCode::BAD_REQUEST, "Invalid permissions");
    }

    let mut has_updates = false;
    let mut new_token = false;
    let mut sign_out = false;
    if let Some(name) = b.get("username").and_then(Value::as_str).filter(|n| !n.is_empty()) {
        if Some(name) != target.username.as_deref() {
            match username_taken(&st.abs, name) {
                Ok(true) => return send_text(StatusCode::BAD_REQUEST, "Username already taken"),
                Ok(false) => {}
                Err(e) => return failed(e),
            }
            target.username = Some(name.to_string());
            new_token = true;
            sign_out = true;
            has_updates = true;
        }
    }
    if truthy("password") {
        let pw = match &b["password"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        target.pash = Some(hash_password(&pw));
        sign_out = true;
        has_updates = true;
    }

    let libraries = text_list(&b, "librariesAccessible");
    let tags = text_list(&b, "itemTagsSelected");
    if let Some(Value::Object(p)) = b.get_mut("permissions") {
        if crate::abs::settings::truthy(p.get("itemTagsSelected")) || crate::abs::settings::truthy(p.get("librariesAccessible")) {
            p.remove("itemTagsSelected");
            p.remove("librariesAccessible");
        }
    }
    if let Some(Value::Object(sent)) = b.get("permissions") {
        let mut permissions = target.permissions.clone().and_then(|p| p.as_object().cloned()).unwrap_or_default();
        let kind = b.get("type").and_then(Value::as_str).filter(|t| !t.is_empty()).or(target.kind.as_deref()).unwrap_or("user").to_string();
        let mut changed = merge_permissions(&mut permissions, &user::default_permissions(&kind), sent);
        let joined = |v: &[Value]| v.iter().map(|x| x.as_str().unwrap_or_default()).collect::<Vec<_>>().join(",");
        let current = |k: &str| target.permissions.as_ref().and_then(|p| p.get(k)).and_then(Value::as_array).cloned().unwrap_or_default();
        if let Some(t) = tags.filter(|t| joined(t) != joined(&current("itemTagsSelected"))) {
            permissions.insert("itemTagsSelected".into(), Value::Array(t));
            changed = true;
        }
        if let Some(l) = libraries.filter(|l| joined(l) != joined(&current("librariesAccessible"))) {
            permissions.insert("librariesAccessible".into(), Value::Array(l));
            changed = true;
        }
        if changed {
            target.permissions = Some(Value::Object(permissions));
            has_updates = true;
        }
    }
    if let Some(email) = b.get("email").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        if Some(email) != target.email.as_deref() {
            target.email = Some(email.to_string());
            has_updates = true;
        }
    }
    if let Some(kind) = b.get("type").and_then(Value::as_str).filter(|t| !t.is_empty()) {
        if Some(kind) != target.kind.as_deref() {
            target.kind = Some(kind.to_string());
            has_updates = true;
        }
    }
    if let Some(active) = b.get("isActive") {
        let active = crate::abs::settings::truthy(Some(active));
        if Some(active) != target.is_active {
            target.is_active = Some(active);
            has_updates = true;
        }
    }
    if let Some(seen) = b.get("lastSeen").and_then(Value::as_f64).filter(|s| *s != 0.0) {
        target.last_seen = Some(models::date_text(seen as i64));
        has_updates = true;
    }

    let mut set_cookie = None;
    if has_updates {
        if new_token {
            target.token = Some(st.abs_server.tokens.legacy_token(&id, target.username.as_deref().unwrap_or_default()));
            tracing::info!("User {} has a new API token", target.username.as_deref().unwrap_or_default());
        }
        if sign_out {
            let by_self = me.user.id == target.id;
            let current = current_refresh_token(&headers);
            match st.abs_server.tokens.invalidate_sessions_for_user(&st.abs, &target, by_self, current.as_deref()) {
                Ok(Some(tokens)) => set_cookie = Some(refresh_cookie(&st, &headers, &tokens.refresh_token)),
                Ok(None) => {}
                Err(e) => return failed(e),
            }
        }
        target.updated_at = Some(models::date_now());
        if let Err(e) = models::update(&st.abs, &target) {
            return failed(e);
        }
        match browser_json(&st.abs, &target) {
            Ok(j) => st.sockets.client_emitter(me.user.id.as_deref().unwrap_or_default(), "user_updated", &j),
            Err(e) => return failed(e),
        }
    }
    let mut r = match browser_json(&st.abs, &target) {
        Ok(j) => Json(json!({ "success": true, "user": j })).into_response(),
        Err(e) => return failed(e),
    };
    if let Some(c) = set_cookie {
        r.headers_mut().append(header::SET_COOKIE, c);
    }
    r
}

/// The refresh token the request came with (the cookie, else `x-refresh-token`).
fn current_refresh_token(headers: &HeaderMap) -> Option<String> {
    cookie(headers, "refresh_token")
        .filter(|t| !t.is_empty())
        .or_else(|| headers.get("x-refresh-token").and_then(|v| v.to_str().ok()).map(str::to_string).filter(|t| !t.is_empty()))
}

/// `DELETE /api/users/:id`: removes a user (admins only; not oneself, not root). Their playlists go too; their
/// listening sessions are kept without the user.
async fn delete(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let target = match req_user(&st, &me.user, &Method::DELETE, &id) {
        Ok(u) => u,
        Err(r) => return *r,
    };
    if me.user.id.as_deref() == Some(id.as_str()) {
        tracing::error!("\"{}\" tried to remove themselves", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::BAD_REQUEST);
    }
    if user::is_root(&target) {
        tracing::error!("Admin \"{}\" tried to remove the root user", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let done = (|| -> Result<Value> {
        let db = &st.abs;
        db.execute("DELETE FROM playlistMediaItems WHERE playlistId IN (SELECT id FROM playlists WHERE userId = ?)", vec![json!(id)])?;
        db.execute("DELETE FROM playlists WHERE userId = ?", vec![json!(id)])?;
        db.execute("UPDATE playbackSessions SET userId = NULL WHERE userId = ?", vec![json!(id)])?;
        let j = browser_json(db, &target)?;
        // the original's foreign keys remove the user's progress, sessions and API keys with the user
        for table in ["mediaProgresses", "sessions", "apiKeys"] {
            db.execute(&format!("DELETE FROM {table} WHERE userId = ?"), vec![json!(id)])?;
        }
        db.execute("UPDATE apiKeys SET createdByUserId = NULL WHERE createdByUserId = ?", vec![json!(id)])?;
        models::delete::<User>(db, &id)?;
        Ok(j)
    })();
    match done {
        Ok(j) => {
            st.sockets.admin_emitter("user_removed", &j);
            Json(json!({ "success": true })).into_response()
        }
        Err(e) => failed(e),
    }
}

/// `PATCH /api/users/:id/openid-unlink`: forgets the user's OpenID account (admins only).
async fn unlink_openid(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let mut target = match req_user(&st, &me.user, &Method::PATCH, &id) {
        Ok(u) => u,
        Err(r) => return *r,
    };
    if user::openid_sub(&target).is_none() {
        return send_status(StatusCode::OK);
    }
    let mut extra = target.extra_data.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    extra["authOpenIDSub"] = Value::Null;
    target.extra_data = Some(extra);
    target.updated_at = Some(models::date_now());
    if let Err(e) = models::update(&st.abs, &target) {
        return failed(e);
    }
    match browser_json(&st.abs, &target) {
        Ok(j) => st.sockets.client_emitter(me.user.id.as_deref().unwrap_or_default(), "user_updated", &j),
        Err(e) => return failed(e),
    }
    send_status(StatusCode::OK)
}

/// `PATCH /api/me/password`: changes the signed-in user's password (`password` → `newPassword`; only root may
/// have none). Signs the user out everywhere else; with `x-refresh-token` the answer has the new tokens.
async fn update_password(
    State(st): State<AppState>,
    me: AbsUser,
    crate::abs::auth::routes::Peer(peer): crate::abs::auth::routes::Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ip = crate::abs::auth::client_ip(&headers, peer);
    let limit = match st.abs_server.limiter.hit(&ip) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    let mut r = change_password(&st, me.user, &headers, &body);
    r.headers_mut().extend(limit);
    r
}

/// The password change itself, without the sign-in limit.
fn change_password(st: &AppState, mut me: User, headers: &HeaderMap, body: &Bytes) -> Response {
    if user::kind(&me) == "guest" {
        tracing::error!("Guest \"{}\" tried to change the password", me.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let b = body_fields(headers, body);
    let text = |k: &str| match b.get(k) {
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(Value::Null) => Some(None),
        _ => None,
    };
    let (Some(password), Some(new_password)) = (text("password"), text("newPassword")) else {
        return send_text(StatusCode::BAD_REQUEST, "Missing or invalid password or new password");
    };
    let new_password = new_password.unwrap_or_default();
    if !user::is_root(&me) && new_password.is_empty() {
        return send_text(StatusCode::BAD_REQUEST, "Invalid new password - Only root can have an empty password");
    }
    if !password_matches(&me, password.as_deref().unwrap_or_default()) {
        return send_text(StatusCode::BAD_REQUEST, "Invalid password");
    }
    me.pash = Some(hash_password(&new_password));
    me.updated_at = Some(models::date_now());
    if let Err(e) = models::update(&st.abs, &me) {
        tracing::error!("The password of \"{}\" couldn't be changed: {}", me.username.as_deref().unwrap_or_default(), e.detail);
        return send_text(StatusCode::BAD_REQUEST, "Unknown error");
    }
    tracing::info!("User \"{}\" changed their password", me.username.as_deref().unwrap_or_default());
    let return_tokens = headers.get("x-refresh-token").is_some_and(|v| !v.is_empty());
    let current = current_refresh_token(headers);
    match st.abs_server.tokens.invalidate_sessions_for_user(&st.abs, &me, true, current.as_deref()) {
        Ok(Some(tokens)) => {
            let mut r = if return_tokens {
                Json(json!({ "success": true, "user": { "accessToken": tokens.access_token, "refreshToken": tokens.refresh_token } }))
                    .into_response()
            } else {
                send_status(StatusCode::OK)
            };
            r.headers_mut().append(header::SET_COOKIE, refresh_cookie(st, headers, &tokens.refresh_token));
            r
        }
        Ok(None) => send_status(StatusCode::OK),
        Err(e) => failed(e),
    }
}

/// Whether a password is the user's (`comparePassword`): a user without a password matches only an empty one.
fn password_matches(u: &User, password: &str) -> bool {
    match u.pash.as_deref().filter(|p| !p.is_empty()) {
        None => password.is_empty(),
        Some(pash) => bcrypt::verify(password, pash).unwrap_or(false),
    }
}

/// `GET /api/auth-settings`: the sign-in settings (admins only).
async fn auth_settings(State(st): State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to read the sign-in settings but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    Json(st.abs_server.settings().authentication_settings()).into_response()
}

/// `PATCH /api/auth-settings`: changes the sign-in settings (admins only); answers whether anything changed and
/// the settings as the apps see them.
async fn update_auth_settings(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to change the sign-in settings but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let fields = body_fields(&headers, &body);
    let mut settings = st.abs_server.settings();
    let updated = settings.update_auth_settings(&fields);
    if updated {
        if let Err(e) = st.abs_server.save_settings(&st.abs, settings.clone()) {
            return failed(e);
        }
    }
    Json(json!({ "updated": updated, "serverSettings": settings.to_json_for_browser() })).into_response()
}
