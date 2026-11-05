//! The port of the original's server settings routes (`server/controllers/MiscController.js`):
//! `PATCH /api/settings`, `PATCH /api/sorting-prefixes`, `POST /api/validate-cron` and `GET /api/tasks`. Admins
//! change the settings; anyone signed in may check a schedule or see the running tasks. Answers are the original's
//! (tests/abs_admin.rs compares them with the recorded ones).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::util::{title_ignore_prefix, Sorting};
use crate::abs::models::user;
use crate::abs::progress::send_text;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/settings", patch(update_settings))
        .route("/api/sorting-prefixes", patch(update_sorting_prefixes))
        .route("/api/validate-cron", post(validate_cron))
        .route("/api/tasks", get(tasks))
}

/// `PATCH /api/settings`: changes the settings an admin may change (others are ignored) and answers them as the
/// apps see them. `allowIframe` can't be turned off while `ALLOW_IFRAME=1`.
async fn update_settings(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to change the server settings but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let fields = body_fields(&headers, &body);
    // JavaScript's `allowIframe == false`
    let iframe_off = fields
        .get("allowIframe")
        .is_some_and(|v| matches!(v, Value::Bool(false)) || v == &json!(0) || v == &json!("") || v == &json!("0") || v == &json!([]));
    if iframe_off && std::env::var("ALLOW_IFRAME").as_deref() == Ok("1") {
        tracing::warn!("Embedding can't be turned off while ALLOW_IFRAME is on");
        return send_text(StatusCode::BAD_REQUEST, "Cannot disable iframe when ALLOW_IFRAME is enabled in environment");
    }
    let filtered: serde_json::Map<String, Value> =
        fields.into_iter().filter(|(k, _)| crate::abs::settings::PATCHABLE.contains(&k.as_str())).collect();
    if filtered.get("allowedOrigins").is_some_and(|o| crate::abs::settings::truthy(Some(o)) && !o.is_array()) {
        return send_text(StatusCode::BAD_REQUEST, "allowedOrigins must be an array");
    }
    let mut settings = st.abs_server.settings();
    if settings.update(&filtered) {
        if let Err(e) = st.abs_server.save_settings(&st.abs, settings.clone()) {
            return e.into_response();
        }
    }
    Json(json!({ "serverSettings": settings.to_json_for_browser() })).into_response()
}

/// `PATCH /api/sorting-prefixes`: the words moved to the end when sorting titles ("The", "A"). Cleans the list
/// (trimmed, lower case, no repeats), stores it and refreshes the sort titles of books, podcasts and series;
/// answers how many rows changed.
async fn update_sorting_prefixes(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to change the sorting words but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let fields = body_fields(&headers, &body);
    let Some(list) = fields.get("sortingPrefixes").and_then(Value::as_array).filter(|l| !l.is_empty()) else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let mut prefixes: Vec<String> = Vec::new();
    for p in list.iter().filter_map(Value::as_str).map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        if !prefixes.contains(&p) {
            prefixes.push(p);
        }
    }
    if prefixes.is_empty() {
        return send_text(StatusCode::BAD_REQUEST, "Invalid sortingPrefixes in request body");
    }
    let mut settings = st.abs_server.settings();
    settings.0.insert("sortingPrefixes".into(), json!(prefixes));
    if let Err(e) = st.abs_server.save_settings(&st.abs, settings.clone()) {
        return e.into_response();
    }
    match refresh_sort_titles(&st.abs, &Sorting::of(&settings)) {
        Ok(rows) => Json(json!({ "rowsUpdated": rows, "serverSettings": settings.to_json_for_browser() })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Sets `titleIgnorePrefix` of books and podcasts and `nameIgnorePrefix` of series where the new words change
/// it; answers how many rows changed.
pub fn refresh_sort_titles(db: &dyn Db, sorting: &Sorting) -> Result<usize> {
    let mut rows = 0;
    for (table, name, sort) in
        [("books", "title", "titleIgnorePrefix"), ("podcasts", "title", "titleIgnorePrefix"), ("series", "name", "nameIgnorePrefix")]
    {
        let all = db.query(&format!("SELECT id, `{name}` AS n, `{sort}` AS s FROM {table}"), vec![])?;
        for r in all {
            let title = r.get("n").and_then(Value::as_str).unwrap_or_default();
            let new = title_ignore_prefix(title, sorting);
            if r.get("s").and_then(Value::as_str) != Some(new.as_str()) {
                db.execute(
                    &format!("UPDATE {table} SET `{sort}` = ? WHERE id = ?"),
                    vec![json!(new), r.get("id").cloned().unwrap_or(Value::Null)],
                )?;
                rows += 1;
            }
        }
    }
    if rows > 0 {
        tracing::info!("The sorting words changed the sort title of {rows} books, podcasts and series");
    }
    Ok(rows)
}

/// `POST /api/validate-cron`: whether a schedule (`expression`) is valid; 200 or 400.
async fn validate_cron(_me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    let Some(expression) = fields.get("expression").filter(|e| crate::abs::settings::truthy(Some(e))) else {
        return send_status(StatusCode::BAD_REQUEST);
    };
    if !expression.as_str().is_some_and(crate::abs::cron::validate) {
        tracing::warn!("The schedule {expression} isn't valid");
        return send_text(StatusCode::BAD_REQUEST, "Invalid cron expression");
    }
    send_status(StatusCode::OK)
}

/// `GET /api/tasks`: the running tasks; with `?include=queue` also the items waiting for their metadata to be
/// embedded.
async fn tasks(State(st): State<AppState>, _me: AbsUser, Query(q): Query<HashMap<String, String>>) -> Response {
    let mut out = json!({ "tasks": st.abs_server.tasks.tasks() });
    if q.get("include").is_some_and(|i| i.split(',').any(|x| x == "queue")) {
        out["queuedTaskData"] = json!({ "embedMetadata": [] });
    }
    Json(out).into_response()
}
