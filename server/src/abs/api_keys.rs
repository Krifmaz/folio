//! The port of the original's API keys (`server/controllers/ApiKeyController.js` and `models/ApiKey.js`): admins
//! make keys that sign in as a user, turn them on or off, move them to another user and remove them. A key is a
//! token signed with the server's secret (`{keyId, name, type: "api"}`, with an expiry when asked); its name and
//! expiry can't change afterwards. Answers are the original's (tests/abs_users.rs compares them).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{jwt, send_status, AbsUser};
use crate::abs::models::{self, user, ApiKey, User};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// The routes, under `/api/api-keys`.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/api-keys", get(all).post(create)).route("/api/api-keys/{id}", patch(update).delete(delete))
}

/// A server error as an answer.
fn failed(e: kit_server_core::Error) -> Response {
    e.into_response()
}

/// The original's check for every key route (`middleware`): admins only.
fn admin_only(me: &User) -> Option<Response> {
    if user::is_admin_or_up(me) {
        return None;
    }
    tracing::error!("\"{}\" tried to use the API keys but isn't an admin", me.username.as_deref().unwrap_or_default());
    Some(send_status(StatusCode::FORBIDDEN))
}

/// A user as a key shows its owner or maker (`id`, `username`, `type`), or null.
fn short_user(db: &dyn Db, id: Option<&str>) -> Result<Value> {
    let Some(id) = id.filter(|i| !i.is_empty()) else { return Ok(Value::Null) };
    Ok(match user::by_id(db, id)? {
        Some(u) => json!({ "id": u.id, "username": u.username, "type": u.kind }),
        None => Value::Null,
    })
}

/// Every column of a key as Sequelize's `toJSON` sends it (dates as ISO text).
fn key_json(k: &ApiKey) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), json!(k.id));
    m.insert("name".into(), json!(k.name));
    m.insert("description".into(), json!(k.description));
    m.insert("expiresAt".into(), models::date_iso(k.expires_at.as_deref()));
    m.insert("lastUsedAt".into(), models::date_iso(k.last_used_at.as_deref()));
    m.insert("isActive".into(), json!(k.is_active.unwrap_or(false)));
    m.insert("permissions".into(), k.permissions.clone().unwrap_or(Value::Null));
    m.insert("createdAt".into(), models::date_iso(k.created_at.as_deref()));
    m.insert("updatedAt".into(), models::date_iso(k.updated_at.as_deref()));
    m.insert("userId".into(), json!(k.user_id));
    m.insert("createdByUserId".into(), json!(k.created_by_user_id));
    m
}

/// `GET /api/api-keys`: every key with its user and maker.
async fn all(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let built = (|| -> Result<Vec<Value>> {
        let keys = models::list::<ApiKey>(&st.abs, "1 = 1", "", vec![])?;
        keys.iter()
            .map(|k| {
                let mut m = key_json(k);
                m.insert("user".into(), short_user(&st.abs, k.user_id.as_deref())?);
                m.insert("createdByUser".into(), short_user(&st.abs, k.created_by_user_id.as_deref())?);
                Ok(Value::Object(m))
            })
            .collect()
    })();
    match built {
        Ok(keys) => Json(json!({ "apiKeys": keys })).into_response(),
        Err(e) => failed(e),
    }
}

/// The key's token (`generateApiKey`): `{keyId, name, type: "api"}` signed with the server's secret, expiring
/// after `expires_in` seconds when given.
fn generate(secret: &str, key_id: &str, name: &str, expires_in: Option<i64>) -> String {
    let mut claims = Map::new();
    claims.insert("keyId".into(), json!(key_id));
    claims.insert("name".into(), json!(name));
    claims.insert("type".into(), json!("api"));
    jwt::sign(&claims, secret, expires_in.filter(|s| *s > 0))
}

/// `POST /api/api-keys`: makes a key for a user (`name`, `userId`, optional `expiresIn` seconds and
/// `isActive`). Only root makes keys for root. The key itself is only shown in this answer.
async fn create(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let b = body_fields(&headers, &body);
    let Some(name) = b.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else {
        tracing::warn!("A key was asked for without a name");
        return send_status(StatusCode::BAD_REQUEST);
    };
    let expires_in = match b.get("expiresIn") {
        None => None,
        Some(v) if !crate::abs::settings::truthy(Some(v)) => None,
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| f > 0.0) => n.as_f64(),
        Some(_) => {
            tracing::warn!("A key was asked for with an expiry that isn't a positive number");
            return send_status(StatusCode::BAD_REQUEST);
        }
    };
    let Some(user_id) = b.get("userId").and_then(Value::as_str).filter(|u| !u.is_empty()) else {
        tracing::warn!("A key was asked for without a user");
        return send_status(StatusCode::BAD_REQUEST);
    };
    let owner = match user::by_id(&st.abs, user_id) {
        Ok(Some(u)) => u,
        Ok(None) => {
            tracing::warn!("A key was asked for a user that doesn't exist");
            return send_status(StatusCode::BAD_REQUEST);
        }
        Err(e) => return failed(e),
    };
    if user::is_root(&owner) && !user::is_root(&me.user) {
        tracing::warn!("Only root may make keys for root");
        return send_status(StatusCode::FORBIDDEN);
    }
    let key_id = models::new_id();
    // jsonwebtoken takes the expiry in whole seconds
    let secs = expires_in.map(|s| s.floor() as i64);
    let token = generate(&st.abs_server.tokens.secret(), &key_id, name, secs);
    let now = models::now_ms();
    let key = ApiKey {
        id: Some(key_id),
        name: Some(name.to_string()),
        expires_at: expires_in.map(|s| models::date_text(now + (s * 1000.0) as i64)),
        user_id: Some(user_id.to_string()),
        is_active: Some(crate::abs::settings::truthy(b.get("isActive"))),
        created_by_user_id: me.user.id.clone(),
        created_at: Some(models::date_text(now)),
        updated_at: Some(models::date_text(now)),
        ..ApiKey::default()
    };
    if let Err(e) = models::insert(&st.abs, &key) {
        return failed(e);
    }
    tracing::info!("Made the API key \"{name}\"");
    // a newly made row has only the columns that were set (and the defaults)
    let mut m = key_json(&key);
    for unset in ["description", "lastUsedAt", "permissions"] {
        m.remove(unset);
    }
    let owner_json = json!({ "id": owner.id, "username": owner.username, "type": owner.kind });
    m.insert("user".into(), owner_json);
    let mut out = Map::new();
    out.insert("apiKey".into(), json!(token));
    out.extend(m);
    Json(json!({ "apiKey": out })).into_response()
}

/// `PATCH /api/api-keys/:id`: turns a key on or off (`isActive`) or gives it to another user (`userId`). Only
/// root changes root's keys.
async fn update(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let mut key = match models::find::<ApiKey>(&st.abs, &id) {
        Ok(Some(k)) => k,
        Ok(None) => return send_status(StatusCode::NOT_FOUND),
        Err(e) => return failed(e),
    };
    let owner = match user::by_id(&st.abs, key.user_id.as_deref().unwrap_or_default()) {
        Ok(u) => u,
        Err(e) => return failed(e),
    };
    if owner.as_ref().is_some_and(user::is_root) && !user::is_root(&me.user) {
        tracing::warn!("Only root may change root's keys");
        return send_status(StatusCode::FORBIDDEN);
    }
    let b = body_fields(&headers, &body);
    let mut changed = false;
    if let Some(v) = b.get("userId") {
        let Some(new_owner) = v.as_str() else { return send_status(StatusCode::BAD_REQUEST) };
        let found = match user::by_id(&st.abs, new_owner) {
            Ok(Some(u)) => u,
            Ok(None) => return send_status(StatusCode::BAD_REQUEST),
            Err(e) => return failed(e),
        };
        if user::is_root(&found) && !user::is_root(&me.user) {
            return send_status(StatusCode::FORBIDDEN);
        }
        if key.user_id.as_deref() != Some(new_owner) {
            key.user_id = Some(new_owner.to_string());
            changed = true;
        }
    }
    if let Some(v) = b.get("isActive") {
        let Some(active) = v.as_bool() else { return send_status(StatusCode::BAD_REQUEST) };
        if key.is_active != Some(active) {
            key.is_active = Some(active);
            changed = true;
        }
    }
    if changed {
        key.updated_at = Some(models::date_now());
        if let Err(e) = models::update(&st.abs, &key) {
            return failed(e);
        }
        tracing::info!("Changed the API key \"{}\"", key.name.as_deref().unwrap_or_default());
    }
    // the original sends the whole owner row (password hash included) when nothing changed; Folio always sends
    // the short owner it sends after a change (docs/DECISIONS.md)
    let mut m = key_json(&key);
    match short_user(&st.abs, key.user_id.as_deref()) {
        Ok(u) => {
            m.insert("user".into(), u);
        }
        Err(e) => return failed(e),
    }
    Json(json!({ "apiKey": m })).into_response()
}

/// `DELETE /api/api-keys/:id`: removes a key; it no longer signs in.
async fn delete(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    match models::find::<ApiKey>(&st.abs, &id) {
        Ok(Some(k)) => {
            if let Err(e) = models::delete::<ApiKey>(&st.abs, &id) {
                return failed(e);
            }
            tracing::info!("Removed the API key \"{}\"", k.name.as_deref().unwrap_or_default());
            send_status(StatusCode::OK)
        }
        Ok(None) => send_status(StatusCode::NOT_FOUND),
        Err(e) => failed(e),
    }
}
