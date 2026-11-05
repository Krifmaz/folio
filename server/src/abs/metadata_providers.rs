//! The port of the original's custom metadata providers (`server/controllers/CustomMetadataProviderController.js`):
//! admins add their own metadata servers (name, address, book or podcast, optional sign-in header) and remove
//! them; a library that used a removed one falls back to Google Books (books) or iTunes (podcasts). Answers are
//! the original's (tests/abs_admin.rs).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, user, CustomMetadataProvider};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use axum::{Json, Router};
use kit_server_core::Db;
use serde_json::{json, Map, Value};

/// The routes, under `/api/custom-metadata-providers`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/custom-metadata-providers", get(all).post(create))
        .route("/api/custom-metadata-providers/{id}", delete(remove))
}

/// The original's check for every provider route (`middleware`): admins only.
fn admin_only(me: &models::User) -> Option<Response> {
    if user::is_admin_or_up(me) {
        return None;
    }
    tracing::warn!("\"{}\" tried to use the metadata providers but isn't an admin", me.username.as_deref().unwrap_or_default());
    Some(send_status(StatusCode::FORBIDDEN))
}

/// A provider as Sequelize's `toJSON` sends it (dates as ISO text).
fn provider_json(p: &CustomMetadataProvider) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), json!(p.id));
    m.insert("name".into(), json!(p.name));
    m.insert("mediaType".into(), json!(p.media_type));
    m.insert("url".into(), json!(p.url));
    m.insert("authHeaderValue".into(), json!(p.auth_header_value));
    m.insert("extraData".into(), p.extra_data.clone().unwrap_or(Value::Null));
    m.insert("createdAt".into(), models::date_iso(p.created_at.as_deref()));
    m.insert("updatedAt".into(), models::date_iso(p.updated_at.as_deref()));
    m
}

/// A provider as the apps list it in their provider menus (`toClientJson`), with its `custom-<id>` name.
pub fn client_json(p: &CustomMetadataProvider) -> Value {
    json!({ "id": p.id, "name": p.name, "mediaType": p.media_type, "slug": format!("custom-{}", p.id.as_deref().unwrap_or_default()) })
}

/// `GET /api/custom-metadata-providers`.
async fn all(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    match models::list::<CustomMetadataProvider>(&st.abs, "1 = 1", "", vec![]) {
        Ok(list) => Json(json!({ "providers": list.iter().map(provider_json).collect::<Vec<_>>() })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// A body value as text, the way Sequelize stores it in a text column.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `POST /api/custom-metadata-providers`: adds a provider (`name`, `url`, `mediaType`, `authHeaderValue`).
async fn create(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let b = body_fields(&headers, &body);
    let given = |k: &str| b.get(k).filter(|v| crate::abs::settings::truthy(Some(v)));
    let (Some(name), Some(url), Some(media_type)) = (given("name"), given("url"), given("mediaType")) else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    if !url.as_str().is_some_and(|u| reqwest::Url::parse(u).is_ok()) {
        tracing::error!("The metadata provider address {url} isn't a valid address");
        return send_text(StatusCode::BAD_REQUEST, "Invalid url");
    }
    let now = models::date_now();
    let p = CustomMetadataProvider {
        id: Some(models::new_id()),
        name: Some(text(name)),
        media_type: Some(text(media_type)),
        url: Some(text(url)),
        auth_header_value: given("authHeaderValue").map(text),
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..CustomMetadataProvider::default()
    };
    if let Err(e) = models::insert(&st.abs, &p) {
        return e.into_response();
    }
    st.sockets.emitter("custom_metadata_provider_added", &client_json(&p), None);
    // a newly made row has only the columns that were set
    let mut j = provider_json(&p);
    j.remove("extraData");
    Json(json!({ "provider": j })).into_response()
}

/// `DELETE /api/custom-metadata-providers/:id`: removes a provider; libraries that used it go back to Google
/// Books or iTunes.
async fn remove(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let p = match models::find::<CustomMetadataProvider>(&st.abs, &id) {
        Ok(Some(p)) => p,
        Ok(None) => return send_status(StatusCode::NOT_FOUND),
        Err(e) => return e.into_response(),
    };
    let fallback = if p.media_type.as_deref() == Some("book") { "google" } else { "itunes" };
    let done = models::delete::<CustomMetadataProvider>(&st.abs, &id).and_then(|_| {
        st.abs
            .execute(
                "UPDATE libraries SET provider = ?, updatedAt = ? WHERE provider = ?",
                vec![json!(fallback), json!(models::date_now()), json!(format!("custom-{id}"))],
            )
            .map(|_| ())
    });
    if let Err(e) = done {
        return e.into_response();
    }
    st.sockets.emitter("custom_metadata_provider_removed", &client_json(&p), None);
    send_status(StatusCode::OK)
}
