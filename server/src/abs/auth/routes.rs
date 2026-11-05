//! The original's sign-in and server routes (`Auth.initAuthRoutes` and the routes in `Server.js`): `/login`,
//! `/auth/refresh`, `/logout`, `/init`, `/status`, `/ping`, `/healthcheck`, plus `POST /api/authorize` and
//! `GET /api/me`, and the answer for an unknown `/api` address. Each answers exactly like the original
//! (tests/abs_auth.rs compares them with the recorded answers).

use super::{client_ip, is_secure, send_status, unauthorized, AbsUser, TokenManager, Tokens};
use crate::abs::models::{user, User};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, OriginalUri, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use kit_server_core::Result;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::net::SocketAddr;

/// The sign-in and server routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/logout", post(logout))
        .route("/init", post(init))
        .route("/status", get(status))
        .route("/ping", get(ping))
        .route("/healthcheck", get(healthcheck))
        .route("/api/authorize", post(authorize))
        .route("/api/me", get(me))
        .route("/api/{*rest}", any(api_not_found))
}

/// The connection's address, when the server was started with it.
pub struct Peer(pub Option<SocketAddr>);

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut axum::http::request::Parts, _: &S) -> std::result::Result<Self, Self::Rejection> {
        Ok(Peer(parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0)))
    }
}

/// A request body as fields: JSON or a form, like Express's body parsers; anything else is empty.
pub fn body_fields(headers: &HeaderMap, body: &Bytes) -> Map<String, Value> {
    let ctype = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default().to_lowercase();
    if ctype.starts_with("application/x-www-form-urlencoded") {
        let pairs: Vec<(String, String)> = serde_urlencoded::from_bytes(body).unwrap_or_default();
        return pairs.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
    }
    match serde_json::from_slice(body) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// A body field as text, the way JavaScript turns it into a string; `None` when missing or null.
fn field_text(fields: &Map<String, Value>, key: &str) -> Option<String> {
    match fields.get(key)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        v => Some(v.to_string()),
    }
}

/// A cookie of the request by name.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// A request header as text.
fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string).filter(|s| !s.is_empty())
}

/// An HTTP date (`Expires=`), from milliseconds since 1970.
fn http_date(ms: i64) -> String {
    httpdate::fmt_http_date(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms.max(0) as u64))
}

/// The `refresh_token` cookie (`setRefreshTokenCookie`): HTTP only, lax, for the refresh token's lifetime.
fn refresh_cookie(st: &AppState, headers: &HeaderMap, token: &str) -> HeaderValue {
    let age = st.abs_server.tokens.refresh_expiry;
    let secure = if is_secure(headers) { "; Secure" } else { "" };
    let v = format!(
        "refresh_token={token}; Max-Age={age}; Path=/; Expires={}; HttpOnly{secure}; SameSite=Lax",
        http_date(crate::abs::models::now_ms() + age * 1000)
    );
    HeaderValue::from_str(&v).unwrap_or_else(|_| HeaderValue::from_static("refresh_token=; Path=/"))
}

/// A cookie that removes one (`res.clearCookie`).
fn clear_cookie(name: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{name}=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"))
        .unwrap_or_else(|_| HeaderValue::from_static("x=; Path=/"))
}

/// The answer to a successful sign-in or token check (`getUserLoginResponsePayload`).
pub fn login_payload(st: &AppState, u: &User, is_old_token: Option<bool>) -> Result<Value> {
    let db = &st.abs;
    let libraries = user::all_library_ids(db)?;
    let progress = user::media_progresses(db, u.id.as_deref().unwrap_or_default())?;
    let settings = st.abs_server.settings();
    Ok(json!({
        "user": user::to_old_json_for_browser(u, &progress, is_old_token, false, false),
        "userDefaultLibraryId": user::default_library_id(u, &libraries),
        "serverSettings": settings.to_json_for_browser(),
        "ereaderDevices": crate::abs::settings::ereader_devices(db, u)?,
        "Source": st.abs_server.source
    }))
}

/// `POST /login`: signs in with a name and password (the local strategy). The mobile app sends
/// `x-return-tokens: true` and gets the refresh token in the answer; the web app gets it as a cookie.
async fn login(State(st): State<AppState>, peer: Peer, headers: HeaderMap, body: Bytes) -> Response {
    let ip = client_ip(&headers, peer.0);
    let limit = match st.abs_server.limiter.hit(&ip) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    let fields = body_fields(&headers, &body);
    let (Some(username), Some(password)) = (field_text(&fields, "username"), field_text(&fields, "password")) else {
        let mut r = send_status(StatusCode::BAD_REQUEST);
        r.headers_mut().remove(header::CONTENT_TYPE);
        return (limit, r).into_response();
    };
    let u = match verify_credentials(&st, &username, &password, &ip) {
        Ok(Some(u)) => u,
        Ok(None) => return (limit, unauthorized()).into_response(),
        Err(e) => return e.into_response(),
    };
    let return_tokens = headers.get("x-return-tokens").and_then(|v| v.to_str().ok()) == Some("true");
    match login_success(&st, &headers, &ip, &u, return_tokens) {
        Ok((payload, cookie)) => {
            let mut r = (limit, Json(payload)).into_response();
            if let Some(c) = cookie {
                r.headers_mut().append(header::SET_COOKIE, c);
            }
            r
        }
        Err(e) => e.into_response(),
    }
}

/// The user a name and password sign in as (`verifyCredentials`), or `None`.
fn verify_credentials(st: &AppState, username: &str, password: &str, ip: &str) -> Result<Option<User>> {
    let found = user::by_username(&st.abs, &username.to_lowercase())?;
    let Some(u) = found.filter(user::is_active) else {
        tracing::warn!("Failed sign-in for \"{username}\" from {ip}: no such active user");
        return Ok(None);
    };
    let pash = u.pash.clone().unwrap_or_default();
    if pash.is_empty() {
        // only root may have no password, and then signs in with an empty one
        let ok = user::is_root(&u) && password.is_empty();
        if !ok {
            tracing::warn!("Failed sign-in for \"{username}\" from {ip}: no password set");
        }
        return Ok(ok.then_some(u));
    }
    if bcrypt::verify(password, &pash).unwrap_or(false) {
        tracing::info!("User \"{username}\" signed in from {ip}");
        return Ok(Some(u));
    }
    tracing::warn!("Failed sign-in for \"{username}\" from {ip}: wrong password");
    Ok(None)
}

/// A password stored like the original's (bcrypt, cost 8, `$2a$`), or empty for no password (`hashPassword`).
pub fn hash_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }
    bcrypt::hash_with_result(password, 8).map(|h| h.format_for_version(bcrypt::Version::TwoA)).unwrap_or_default()
}

/// After a sign-in (`handleLoginSuccess`): new tokens and session, the answer with the tokens in the user, and
/// the refresh cookie unless the tokens are returned.
fn login_success(st: &AppState, headers: &HeaderMap, ip: &str, u: &User, return_tokens: bool) -> Result<(Value, Option<HeaderValue>)> {
    let ua = header_text(headers, "user-agent");
    let Tokens { access_token, refresh_token } = st.abs_server.tokens.create_tokens_and_session(&st.abs, u, ip, ua.as_deref())?;
    let mut payload = login_payload(st, u, None)?;
    payload["user"]["refreshToken"] = if return_tokens { json!(refresh_token) } else { Value::Null };
    payload["user"]["accessToken"] = json!(access_token);
    let cookie = (!return_tokens).then(|| refresh_cookie(st, headers, &refresh_token));
    Ok((payload, cookie))
}

/// `POST /auth/refresh`: new tokens for the refresh token in the cookie or the `x-refresh-token` header (which
/// also gets the new refresh token back in the answer).
async fn refresh(State(st): State<AppState>, peer: Peer, headers: HeaderMap) -> Response {
    let ip = client_ip(&headers, peer.0);
    let limit = match st.abs_server.limiter.hit(&ip) {
        Ok(h) => h,
        Err(r) => return *r,
    };
    let from_header = header_text(&headers, "x-refresh-token");
    let return_refresh = from_header.is_some();
    let Some(token) = from_header.or_else(|| cookie(&headers, "refresh_token")).filter(|t| !t.is_empty()) else {
        return (StatusCode::UNAUTHORIZED, limit, Json(json!({ "error": "No refresh token provided" }))).into_response();
    };
    let result = st.abs_server.tokens.refresh(&st.abs, &token);
    let (tokens, u) = match result {
        Ok(Ok(t)) => t,
        Ok(Err(message)) => return (StatusCode::UNAUTHORIZED, limit, Json(json!({ "error": message }))).into_response(),
        Err(e) => return e.into_response(),
    };
    let mut payload = match login_payload(&st, &u, None) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    payload["user"]["accessToken"] = json!(tokens.access_token);
    payload["user"]["refreshToken"] = if return_refresh { json!(tokens.refresh_token) } else { Value::Null };
    let mut r = (limit, Json(payload)).into_response();
    r.headers_mut().append(header::SET_COOKIE, refresh_cookie(&st, &headers, &tokens.refresh_token));
    r
}

/// `POST /logout`: ends the session of the refresh token (all of the user's with `?allDevices=1`) and clears
/// the sign-in cookies. Answers where to go next (`redirect_url`, an OpenID provider's sign-out page or null).
async fn logout(State(st): State<AppState>, Query(q): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    let token = cookie(&headers, "refresh_token").filter(|t| !t.is_empty()).or_else(|| header_text(&headers, "x-refresh-token"));
    let done = match (q.get("allDevices").map(String::as_str), token) {
        (Some("1"), Some(t)) => TokenManager::invalidate_all_for_refresh_token(&st.abs, &t),
        (_, Some(t)) => TokenManager::invalidate_refresh_token(&st.abs, &t),
        _ => {
            tracing::info!("Sign-out without a refresh token");
            Ok(())
        }
    };
    if let Err(e) = done {
        return e.into_response();
    }
    let mut r = Json(json!({ "redirect_url": Value::Null })).into_response();
    for name in ["refresh_token", "auth_method"] {
        r.headers_mut().append(header::SET_COOKIE, clear_cookie(name));
    }
    r
}

/// `POST /init`: sets up a new server with its root user (`{"newRoot": {"username", "password"}}`). A server
/// that has a root user already answers 500, like the original.
async fn init(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    match user::has_root(&st.abs) {
        Ok(true) => {
            tracing::error!("Someone tried to set up the server again, but it already has a root user");
            return send_status(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Err(e) => return e.into_response(),
        Ok(false) => {}
    }
    let new_root = fields.get("newRoot").and_then(Value::as_object).cloned().unwrap_or_default();
    let username = field_text(&new_root, "username").filter(|u| !u.is_empty()).unwrap_or_else(|| "root".into());
    let pash = hash_password(&field_text(&new_root, "password").unwrap_or_default());
    if pash.is_empty() {
        tracing::warn!("The root user was made without a password");
    }
    let tokens = st.abs_server.tokens.clone();
    match user::create_root(&st.abs, &username, &pash, |id| tokens.legacy_token(id, &username)) {
        Ok(Some(_)) => send_status(StatusCode::OK),
        Ok(None) => send_status(StatusCode::INTERNAL_SERVER_ERROR),
        Err(e) => e.into_response(),
    }
}

/// `GET /status`: whether the server is set up, and what the sign-in page shows.
async fn status(State(st): State<AppState>) -> Response {
    let is_init = match user::has_root(&st.abs) {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let settings = st.abs_server.settings();
    let mut payload = json!({
        "app": "audiobookshelf",
        "serverVersion": crate::abs::ABS_VERSION,
        "isInit": is_init,
        "language": settings.get("language"),
        "authMethods": settings.get("authActiveAuthMethods"),
        "authFormData": settings.auth_form_data()
    });
    if !is_init {
        payload["ConfigPath"] = json!(st.abs_server.config_path.display().to_string());
        payload["MetadataPath"] = json!(st.abs_server.metadata_path.display().to_string());
    }
    Json(payload).into_response()
}

/// `GET /ping`.
async fn ping() -> Json<Value> {
    Json(json!({ "success": true }))
}

/// `GET /healthcheck`.
async fn healthcheck() -> Response {
    send_status(StatusCode::OK)
}

/// `POST /api/authorize`: checks a token and answers like a sign-in, without new tokens.
async fn authorize(State(st): State<AppState>, me: AbsUser) -> Response {
    match login_payload(&st, &me.user, me.is_old_token) {
        Ok(p) => Json(p).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/me`: the signed-in user.
async fn me(State(st): State<AppState>, me: AbsUser) -> Response {
    match user::media_progresses(&st.abs, me.user.id.as_deref().unwrap_or_default()) {
        Ok(p) => Json(user::to_old_json_for_browser(&me.user, &p, me.is_old_token, false, false)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Any other `/api` address: 401 without a valid token (the token is checked first, like the original), then
/// Express's 404 page.
async fn api_not_found(_me: AbsUser, method: Method, OriginalUri(uri): OriginalUri) -> Response {
    not_found(&method, uri.path())
}

/// Express's answer for an address nothing serves: a small HTML page, `Cannot GET /path`.
pub fn not_found(method: &Method, path: &str) -> Response {
    let escaped = path.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;");
    let html = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>Cannot {method} {escaped}</pre>\n</body>\n</html>\n"
    );
    (
        StatusCode::NOT_FOUND,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        html,
    )
        .into_response()
}
