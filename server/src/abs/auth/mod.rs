//! The port of the original's sign-in (`server/Auth.js`, `server/auth/TokenManager.js`,
//! `server/auth/LocalAuthStrategy.js`, `server/utils/rateLimiterFactory.js`): the tokens, the sessions behind
//! refresh tokens, the check every `/api` request goes through ([`AbsUser`]) and the limit on sign-in attempts.
//! The routes are in [`routes`].
//!
//! Tokens are the original's: an old token per user that never expires, access tokens of one hour, refresh tokens
//! of 30 days kept in the `sessions` table, and API keys. The lengths follow the same environment variables.

pub mod jwt;
pub mod routes;

use crate::abs::models::{self, user, ApiKey, Session, User};
use crate::AppState;
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};

/// A number of seconds from an environment variable, when it is a whole number above 0.
fn env_secs(name: &str, default: i64) -> i64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse::<i64>().ok()).filter(|n| *n > 0).unwrap_or(default)
}

/// Makes and checks tokens (`TokenManager`).
pub struct TokenManager {
    /// The signing secret: `JWT_SECRET_KEY`, or the one kept in the server settings.
    secret: RwLock<String>,
    /// How long a refresh token lasts, in seconds (`REFRESH_TOKEN_EXPIRY`, 30 days).
    pub refresh_expiry: i64,
    /// How long an access token lasts, in seconds (`ACCESS_TOKEN_EXPIRY`, 1 hour).
    pub access_expiry: i64,
    /// How long a replaced refresh token still works, in seconds (`REFRESH_TOKEN_GRACE_PERIOD`, 10 minutes).
    pub grace_period: i64,
}

/// New tokens for a session.
#[derive(Debug, Clone)]
pub struct Tokens {
    /// The short access token for API calls.
    pub access_token: String,
    /// The refresh token that gets new access tokens.
    pub refresh_token: String,
}

/// A user signed in by a token, and whether that token is an old one without an expiry (`isOldToken`).
#[derive(Debug, Clone)]
pub struct AbsUser {
    /// The user.
    pub user: User,
    /// `Some(true)` for an old never-expiring token; `None` otherwise (the original leaves the flag out).
    pub is_old_token: Option<bool>,
}

impl TokenManager {
    /// The token manager with the secret from `JWT_SECRET_KEY`, else `stored` (the server settings' secret).
    /// Answers whether a new secret was made, which the caller stores in the settings.
    pub fn new(stored: Option<&str>) -> (TokenManager, Option<String>) {
        let (secret, made) = match std::env::var("JWT_SECRET_KEY").ok().filter(|s| !s.is_empty()) {
            Some(s) => (s, None),
            None => match stored.filter(|s| !s.is_empty()) {
                Some(s) => (s.to_string(), None),
                None => {
                    use base64::Engine;
                    let bytes: Vec<u8> = (0..256).map(|_| rand::random::<u8>()).collect();
                    let s = base64::engine::general_purpose::STANDARD.encode(bytes);
                    (s.clone(), Some(s))
                }
            },
        };
        let tm = TokenManager {
            secret: RwLock::new(secret),
            refresh_expiry: env_secs("REFRESH_TOKEN_EXPIRY", 30 * 24 * 60 * 60),
            access_expiry: env_secs("ACCESS_TOKEN_EXPIRY", 60 * 60),
            grace_period: env_secs("REFRESH_TOKEN_GRACE_PERIOD", 10 * 60),
        };
        (tm, made)
    }

    /// The signing secret.
    pub fn secret(&self) -> String {
        self.secret.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// A token payload for a user, of a kind (`access` or `refresh`), with a fresh `jti`.
    fn payload(user_id: &str, username: &str, kind: Option<&str>) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("userId".into(), json!(user_id));
        p.insert("username".into(), json!(username));
        if let Some(k) = kind {
            p.insert("jti".into(), json!(models::new_id()));
            p.insert("type".into(), json!(k));
        }
        p
    }

    /// The old token that never expires (`generateAccessToken`), kept on the user as `token`.
    pub fn legacy_token(&self, user_id: &str, username: &str) -> String {
        jwt::sign(&Self::payload(user_id, username, None), &self.secret(), None)
    }

    /// A short access token (`generateTempAccessToken`).
    pub fn access_token(&self, u: &User) -> String {
        let p = Self::payload(u.id.as_deref().unwrap_or_default(), u.username.as_deref().unwrap_or_default(), Some("access"));
        jwt::sign(&p, &self.secret(), Some(self.access_expiry))
    }

    /// A refresh token (`generateRefreshToken`).
    pub fn refresh_token(&self, u: &User) -> String {
        let p = Self::payload(u.id.as_deref().unwrap_or_default(), u.username.as_deref().unwrap_or_default(), Some("refresh"));
        jwt::sign(&p, &self.secret(), Some(self.refresh_expiry))
    }

    /// New tokens and the session that keeps the refresh token (`createTokensAndSession`).
    pub fn create_tokens_and_session(&self, db: &dyn Db, u: &User, ip: &str, user_agent: Option<&str>) -> Result<Tokens> {
        let access_token = self.access_token(u);
        let refresh_token = self.refresh_token(u);
        let now = models::now_ms();
        let session = Session {
            id: Some(models::new_id()),
            ip_address: Some(ip.to_string()).filter(|s| !s.is_empty()),
            user_agent: user_agent.map(str::to_string),
            refresh_token: Some(refresh_token.clone()),
            expires_at: Some(models::date_text(now + self.refresh_expiry * 1000)),
            created_at: Some(models::date_text(now)),
            updated_at: Some(models::date_text(now)),
            user_id: u.id.clone(),
            ..Session::default()
        };
        models::insert(db, &session)?;
        Ok(Tokens { access_token, refresh_token })
    }

    /// Replaces a session's refresh token (`rotateTokensForSession`). With `grace`, the old one keeps working for
    /// the grace period. When another request rotated it first, that request's token is used.
    pub fn rotate(&self, db: &dyn Db, session: &Session, u: &User, grace: bool) -> Result<Tokens> {
        let previous = session.refresh_token.clone().unwrap_or_default();
        let access_token = self.access_token(u);
        let mut refresh_token = self.refresh_token(u);
        let now = models::now_ms();
        let (last, last_expires) =
            if grace { (json!(previous), json!(models::date_text(now + self.grace_period * 1000))) } else { (Value::Null, Value::Null) };
        let changed = db.execute(
            "UPDATE sessions SET refreshToken = ?, expiresAt = ?, lastRefreshToken = ?, lastRefreshTokenExpiresAt = ?, updatedAt = ? WHERE id = ? AND refreshToken = ?",
            vec![
                json!(refresh_token),
                json!(models::date_text(now + self.refresh_expiry * 1000)),
                last,
                last_expires,
                json!(models::date_text(now)),
                json!(session.id),
                json!(previous),
            ],
        )?;
        if changed == 0 {
            if let Some(s) = models::find::<Session>(db, session.id.as_deref().unwrap_or_default())? {
                refresh_token = s.refresh_token.unwrap_or_default();
            }
        }
        Ok(Tokens { access_token, refresh_token })
    }

    /// The session holding this refresh token, as the current or the previous one.
    fn session_for(db: &dyn Db, refresh_token: &str) -> Result<Option<Session>> {
        let row = db.one(
            "SELECT * FROM sessions WHERE refreshToken = ? OR lastRefreshToken = ? LIMIT 1",
            vec![json!(refresh_token), json!(refresh_token)],
        )?;
        row.map(models::from_row).transpose()
    }

    /// The user a request's token signs in as (`jwtAuthCheck`), or `None` when it must be refused. An expired
    /// API key is turned off.
    pub fn check(&self, db: &dyn Db, token: &str) -> Result<Option<AbsUser>> {
        let Ok(claims) = jwt::verify(token, &self.secret(), true) else { return Ok(None) };
        let expired = claims.get("exp").and_then(Value::as_f64).is_some_and(|exp| exp < models::now_ms() as f64 / 1000.0);
        if claims.get("type").and_then(Value::as_str) == Some("api") {
            let key_id = claims.get("keyId").and_then(Value::as_str).unwrap_or_default();
            let Some(mut key) = models::find::<ApiKey>(db, key_id)? else { return Ok(None) };
            if key.is_active != Some(true) {
                return Ok(None);
            }
            if expired {
                key.is_active = Some(false);
                key.updated_at = Some(models::date_now());
                models::update(db, &key)?;
                tracing::info!("API key {key_id} is expired - deactivated");
                return Ok(None);
            }
            let user = user::by_id(db, key.user_id.as_deref().unwrap_or_default())?;
            return Ok(user.filter(user::is_active).map(|user| AbsUser { user, is_old_token: None }));
        }
        if !is_bearer_access_payload(&claims) || expired {
            return Ok(None);
        }
        let user = user::by_id_or_old_id(db, claims.get("userId").and_then(Value::as_str).unwrap_or_default())?;
        let old = if claims.contains_key("exp") { None } else { Some(true) };
        Ok(user.filter(user::is_active).map(|user| AbsUser { user, is_old_token: old }))
    }

    /// New tokens for a refresh token (`handleRefreshToken`), or the original's message for why not.
    pub fn refresh(&self, db: &dyn Db, refresh_token: &str) -> Result<std::result::Result<(Tokens, User), &'static str>> {
        let claims = match jwt::verify(refresh_token, &self.secret(), false) {
            Ok(c) => c,
            Err(jwt::JwtError::Expired) => {
                db.execute("DELETE FROM sessions WHERE refreshToken = ?", vec![json!(refresh_token)])?;
                return Ok(Err("Refresh token expired"));
            }
            Err(jwt::JwtError::Invalid) => return Ok(Err("Invalid refresh token")),
        };
        if claims.get("type").and_then(Value::as_str) != Some("refresh") {
            return Ok(Err("Invalid token type"));
        }
        let Some(session) = Self::session_for(db, refresh_token)? else { return Ok(Err("Invalid refresh token")) };
        let now = models::now_ms();
        let ms = |t: &Option<String>| t.as_deref().and_then(models::date_ms);
        let in_grace = session.refresh_token.as_deref() != Some(refresh_token);
        if in_grace {
            if !ms(&session.last_refresh_token_expires_at).is_some_and(|t| t > now) {
                return Ok(Err("Invalid refresh token"));
            }
        } else if ms(&session.expires_at).is_some_and(|t| t < now) {
            models::delete::<Session>(db, session.id.as_deref().unwrap_or_default())?;
            return Ok(Err("Refresh token expired"));
        }
        let user = user::by_id(db, claims.get("userId").and_then(Value::as_str).unwrap_or_default())?;
        let Some(user) = user.filter(user::is_active) else { return Ok(Err("User not found or inactive")) };
        if in_grace {
            let tokens =
                Tokens { access_token: self.access_token(&user), refresh_token: session.refresh_token.clone().unwrap_or_default() };
            return Ok(Ok((tokens, user)));
        }
        let tokens = self.rotate(db, &session, &user, true)?;
        Ok(Ok((tokens, user)))
    }

    /// Ends the session of a refresh token (`invalidateRefreshToken`, sign out).
    pub fn invalidate_refresh_token(db: &dyn Db, refresh_token: &str) -> Result<()> {
        db.execute("DELETE FROM sessions WHERE refreshToken = ?", vec![json!(refresh_token)])?;
        Ok(())
    }

    /// Ends every session of the user who owns this refresh token (`invalidateAllSessionsForRefreshToken`).
    pub fn invalidate_all_for_refresh_token(db: &dyn Db, refresh_token: &str) -> Result<()> {
        if let Some(s) = Self::session_for(db, refresh_token)? {
            db.execute("DELETE FROM sessions WHERE userId = ?", vec![json!(s.user_id)])?;
        }
        Ok(())
    }

    /// Ends a user's sessions (`invalidateJwtSessionsForUser`, after a password change). When the user changes
    /// their own and `current_refresh` names one of their sessions, that one is kept with new tokens, returned.
    pub fn invalidate_sessions_for_user(
        &self,
        db: &dyn Db,
        u: &User,
        by_self: bool,
        current_refresh: Option<&str>,
    ) -> Result<Option<Tokens>> {
        let uid = u.id.clone().unwrap_or_default();
        if let (true, Some(rt)) = (by_self, current_refresh.filter(|r| !r.is_empty())) {
            let row = db.one(
                "SELECT * FROM sessions WHERE userId = ? AND (refreshToken = ? OR lastRefreshToken = ?) LIMIT 1",
                vec![json!(uid), json!(rt), json!(rt)],
            )?;
            if let Some(session) = row.map(models::from_row::<Session>).transpose()? {
                let tokens = self.rotate(db, &session, u, false)?;
                db.execute("DELETE FROM sessions WHERE userId = ? AND id != ?", vec![json!(uid), json!(session.id)])?;
                return Ok(Some(tokens));
            }
        }
        db.execute("DELETE FROM sessions WHERE userId = ?", vec![json!(uid)])?;
        Ok(None)
    }
}

/// Whether a token payload may sign in API and socket requests (`isBearerAccessTokenPayload`): it names a user
/// and isn't a refresh token.
pub fn is_bearer_access_payload(claims: &Map<String, Value>) -> bool {
    crate::abs::settings::truthy(claims.get("userId")) && claims.get("type").and_then(Value::as_str) != Some("refresh")
}

/// The limit on sign-in attempts per address (`getAuthRateLimiter`): 40 in 10 minutes unless
/// `RATE_LIMIT_AUTH_MAX` / `RATE_LIMIT_AUTH_WINDOW` say otherwise; `RATE_LIMIT_AUTH_MAX=0` turns it off.
pub struct RateLimiter {
    /// Attempts allowed per window; 0 when turned off.
    pub max: u32,
    /// The window, in milliseconds.
    pub window_ms: i64,
    /// The answer once over the limit (`RATE_LIMIT_AUTH_MESSAGE`).
    pub message: String,
    /// Attempts and window end per address.
    hits: Mutex<HashMap<String, (u32, i64)>>,
}

impl RateLimiter {
    /// The limiter from the environment.
    pub fn from_env() -> RateLimiter {
        let max = match std::env::var("RATE_LIMIT_AUTH_MAX").as_deref() {
            Ok("0") => 0,
            _ => env_secs("RATE_LIMIT_AUTH_MAX", 40) as u32,
        };
        RateLimiter {
            max,
            window_ms: env_secs("RATE_LIMIT_AUTH_WINDOW", 10 * 60 * 1000),
            message: std::env::var("RATE_LIMIT_AUTH_MESSAGE")
                .ok()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "Too many authentication requests".into()),
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Counts one attempt from `ip`. Answers the `RateLimit-*` headers to send, or the 429 answer when over.
    pub fn hit(&self, ip: &str) -> std::result::Result<HeaderMap, Box<Response>> {
        let mut headers = HeaderMap::new();
        if self.max == 0 {
            return Ok(headers);
        }
        let now = models::now_ms();
        let (count, reset) = {
            let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
            hits.retain(|_, (_, end)| *end > now);
            let e = hits.entry(ip.to_string()).or_insert((0, now + self.window_ms));
            e.0 += 1;
            *e
        };
        let secs = ((reset - now) as f64 / 1000.0).ceil().max(0.0) as i64;
        let put = |h: &mut HeaderMap, k: &'static str, v: String| {
            if let Ok(v) = HeaderValue::from_str(&v) {
                h.insert(k, v);
            }
        };
        put(&mut headers, "ratelimit-policy", format!("{};w={}", self.max, self.window_ms / 1000));
        put(&mut headers, "ratelimit-limit", self.max.to_string());
        put(&mut headers, "ratelimit-remaining", self.max.saturating_sub(count).to_string());
        put(&mut headers, "ratelimit-reset", secs.to_string());
        if count > self.max {
            tracing::warn!("Too many sign-in attempts from {ip}");
            put(&mut headers, "retry-after", secs.to_string());
            return Err(Box::new((StatusCode::TOO_MANY_REQUESTS, headers, axum::Json(json!({ "error": self.message }))).into_response()));
        }
        Ok(headers)
    }
}

/// Whether a header value is an IP address.
fn is_ip(s: &str) -> bool {
    s.trim().parse::<std::net::IpAddr>().is_ok()
}

/// The client's address as the original's `requestIp.getClientIp` finds it: the proxy headers first, then the
/// connection.
pub fn client_ip(headers: &HeaderMap, peer: Option<std::net::SocketAddr>) -> String {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(ip) = h("x-client-ip").filter(|v| is_ip(v)) {
        return ip.to_string();
    }
    if let Some(xff) = h("x-forwarded-for") {
        let found = xff.split(',').map(|e| {
            let ip = e.trim();
            match ip.split(':').collect::<Vec<_>>().as_slice() {
                [a, _] => a.to_string(),
                _ => ip.to_string(),
            }
        });
        if let Some(ip) = found.into_iter().find(|ip| is_ip(ip)) {
            return ip;
        }
    }
    for k in [
        "cf-connecting-ip",
        "fastly-client-ip",
        "true-client-ip",
        "x-real-ip",
        "x-cluster-client-ip",
        "x-forwarded",
        "forwarded-for",
        "forwarded",
    ] {
        if let Some(ip) = h(k).filter(|v| is_ip(v)) {
            return ip.to_string();
        }
    }
    peer.map(|p| p.ip().to_string()).unwrap_or_default()
}

/// Whether the request came over HTTPS, directly or through a proxy (`isRequestSecure`).
pub fn is_secure(headers: &HeaderMap) -> bool {
    let xfp = headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).unwrap_or_default().to_lowercase();
    xfp == "https" || xfp.split(',').any(|s| s.trim() == "https")
}

/// The token of a request: `Authorization: Bearer …`, else the `token` query value (passport-jwt's extractors).
pub fn request_token(parts: &Parts) -> Option<String> {
    let bearer = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| {
        let mut it = v.trim().splitn(2, ' ');
        match (it.next(), it.next()) {
            (Some(scheme), Some(tok)) if scheme.eq_ignore_ascii_case("bearer") => Some(tok.trim().to_string()),
            _ => None,
        }
    });
    bearer.or_else(|| {
        Query::<HashMap<String, String>>::try_from_uri(&parts.uri).ok().and_then(|q| q.0.get("token").cloned()).filter(|t| !t.is_empty())
    })
}

/// The original's answer to a request without a valid token: 401 with the text `Unauthorized`.
pub fn unauthorized() -> Response {
    let mut r = (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    r.headers_mut().remove(header::CONTENT_TYPE);
    r
}

/// An answer like Express's `res.sendStatus(code)`: the status and its standard text.
pub fn send_status(code: StatusCode) -> Response {
    let text = code.canonical_reason().unwrap_or_default().to_string();
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response().with_status(code)
}

/// Sets the status of an answer.
trait WithStatus {
    /// The answer with this status.
    fn with_status(self, code: StatusCode) -> Response;
}

impl WithStatus for Response {
    fn with_status(mut self, code: StatusCode) -> Response {
        *self.status_mut() = code;
        self
    }
}

impl FromRequestParts<AppState> for AbsUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> std::result::Result<Self, Self::Rejection> {
        let Some(token) = request_token(parts) else { return Err(unauthorized()) };
        match st.abs_server.tokens.check(&st.abs, &token) {
            Ok(Some(u)) => Ok(u),
            Ok(None) => Err(unauthorized()),
            Err(e) => Err(e.into_response()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tm() -> TokenManager {
        TokenManager { secret: RwLock::new("s".into()), refresh_expiry: 100, access_expiry: 10, grace_period: 5 }
    }

    fn db_with_root() -> (kit_server_core::SqliteDb, User) {
        let db = kit_server_core::SqliteDb::memory();
        crate::abs::create(&db).unwrap();
        let t = tm();
        let u = user::create_root(&db, "root", "", |id| t.legacy_token(id, "root")).unwrap().unwrap();
        (db, u)
    }

    #[test]
    fn tokens_sign_in_like_the_original() {
        let (db, u) = db_with_root();
        let t = tm();
        let legacy = t.check(&db, u.token.as_deref().unwrap()).unwrap().unwrap();
        assert_eq!(legacy.is_old_token, Some(true));
        let access = t.check(&db, &t.access_token(&u)).unwrap().unwrap();
        assert_eq!(access.is_old_token, None);
        assert!(t.check(&db, &t.refresh_token(&u)).unwrap().is_none(), "refresh tokens don't sign in API calls");
        assert!(t.check(&db, "nonsense").unwrap().is_none());
    }

    #[test]
    fn refresh_rotates_with_a_grace_period() {
        let (db, u) = db_with_root();
        let t = tm();
        let first = t.create_tokens_and_session(&db, &u, "1.2.3.4", Some("test")).unwrap();
        let (second, _) = t.refresh(&db, &first.refresh_token).unwrap().unwrap();
        assert_ne!(second.refresh_token, first.refresh_token);
        let (again, _) = t.refresh(&db, &first.refresh_token).unwrap().unwrap();
        assert_eq!(again.refresh_token, second.refresh_token, "the old token gives the current one during the grace period");
        assert_eq!(t.refresh(&db, &t.access_token(&u)).unwrap().unwrap_err(), "Invalid token type");
        TokenManager::invalidate_refresh_token(&db, &second.refresh_token).unwrap();
        assert_eq!(t.refresh(&db, &second.refresh_token).unwrap().unwrap_err(), "Invalid refresh token");
    }

    #[test]
    fn the_limit_counts_per_address() {
        let l = RateLimiter { max: 2, window_ms: 60_000, message: "Too many".into(), hits: Mutex::new(HashMap::new()) };
        assert!(l.hit("a").is_ok());
        let h = l.hit("a").unwrap();
        assert_eq!(h["ratelimit-remaining"], "0");
        assert_eq!(l.hit("a").unwrap_err().status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(l.hit("b").is_ok());
    }

    #[test]
    fn client_addresses_come_from_proxy_headers_first() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("unknown, 10.0.0.1:443, 10.0.0.2"));
        assert_eq!(client_ip(&h, None), "10.0.0.1");
        h.insert("x-client-ip", HeaderValue::from_static("192.168.1.9"));
        assert_eq!(client_ip(&h, None), "192.168.1.9");
    }
}
