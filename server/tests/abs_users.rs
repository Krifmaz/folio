//! The Audiobookshelf port's user management (PARITY P2b): users, API keys, the sign-in settings and the
//! password change. Replays the recorded requests (tests/parity/abs/requests.json) in the recorded order and
//! compares every answer with the pinned original's (tests/parity/fixtures/abs).

<<<<<<< HEAD
mod abs_replay;
mod common;
mod parity;

use abs_replay::{recorded_cookies, send, set_up};
use kit_server_core::Db;
use parity::{assert_like_with, fixture};
use serde_json::{json, Value};
=======
mod common;
mod parity;

use axum::body::Body;
use axum::http::{header, Request};
use axum::Router;
use kit_server_core::Db;
use parity::{assert_like_with, fixture, Matcher};
use serde_json::{json, Value};
use std::collections::HashMap;
use tower::ServiceExt;

/// One answer: status, content type (without parameters), the cookies it set (`name; attributes`, without
/// values and dates, like the recordings) and the body (JSON, else text).
struct Answer {
    status: u16,
    content_type: String,
    cookies: Vec<String>,
    body: Value,
}

/// Sends a request with an optional JSON body and headers.
async fn send(app: &Router, method: &str, path: &str, body: Option<Value>, headers: &[(String, String)]) -> Answer {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let req = match body {
        Some(v) => b.header(header::CONTENT_TYPE, "application/json").body(Body::from(v.to_string())).unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let r = app.clone().oneshot(req).await.unwrap();
    let status = r.status().as_u16();
    let content_type = r
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .to_string();
    let mut cookies: Vec<String> = r
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|c| {
            let (name, rest) = c.split_once('=').unwrap_or((c, ""));
            let attrs = rest.split(';').skip(1).map(str::trim).filter(|a| !a.to_lowercase().starts_with("expires="));
            std::iter::once(name.trim()).chain(attrs).collect::<Vec<_>>().join("; ")
        })
        .collect();
    cookies.sort();
    let bytes = axum::body::to_bytes(r.into_body(), 1 << 24).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    Answer { status, content_type, cookies, body }
}

/// The recorded requests by name.
fn requests() -> HashMap<String, Value> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/abs/requests.json");
    let list: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    list.into_iter().map(|r| (r["name"].as_str().unwrap().to_string(), r)).collect()
}

/// A value with every `{name}` replaced from the saved values.
fn fill(v: &Value, ctx: &HashMap<String, String>) -> Value {
    match v {
        Value::String(s) => {
            let mut s = s.clone();
            for (k, val) in ctx {
                s = s.replace(&format!("{{{k}}}"), val);
            }
            Value::String(s)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| fill(x, ctx)).collect()),
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), fill(x, ctx))).collect()),
        other => other.clone(),
    }
}

/// A value at a dotted path (`users.0.id`).
fn at<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(v, |v, k| match k.parse::<usize>() {
        Ok(i) => &v[i],
        Err(_) => &v[k],
    })
}

/// Replays the recorded requests: `{token}` and `{library}` are known, the rest come from earlier answers.
struct Replay {
    /// The app.
    app: Router,
    /// Saved values for `{name}` placeholders.
    ctx: HashMap<String, String>,
    /// The recorded requests by name.
    requests: HashMap<String, Value>,
    /// The id matcher, kept across answers.
    m: Matcher,
}

impl Replay {
    /// Sends the request recorded as `name` and returns Folio's answer, without comparing it.
    async fn send(&mut self, name: &str) -> Answer {
        let r = self.requests.get(name).unwrap_or_else(|| panic!("no recorded request {name}")).clone();
        let r = fill(&r, &self.ctx);
        let method = r["method"].as_str().unwrap_or("GET").to_string();
        let path = r["path"].as_str().unwrap().to_string();
        let mut headers: Vec<(String, String)> = Vec::new();
        if r["auth"] != json!(false) {
            headers.push(("Authorization".into(), format!("Bearer {}", self.ctx["token"])));
        }
        if let Some(h) = r["headers"].as_object() {
            headers.extend(h.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())));
        }
        let body = r.get("body").filter(|b| !b.is_null()).cloned();
        let a = send(&self.app, &method, &path, body, &headers).await;
        if let Some(save) = r["save"].as_object() {
            for (k, p) in save {
                if let Some(v) = at(&a.body, p.as_str().unwrap()).as_str() {
                    self.ctx.insert(k.clone(), v.to_string());
                }
            }
        }
        a
    }

    /// Sends the request recorded as `name` and checks the answer against the recording.
    async fn like(&mut self, name: &str) -> Answer {
        let a = self.send(name).await;
        let fx = fixture("abs", name);
        assert_eq!(a.content_type, fx.content_type, "{name}: content type ({})", a.body);
        assert_like_with(&mut self.m, a.status, &a.body, &fx);
        a
    }
}

/// The cookies a recording set.
fn recorded_cookies(name: &str) -> Vec<String> {
    let path = parity::dir("abs").join(format!("{name}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["cookies"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// A set-up server like the recording's: root (root / root) with one bookmark, and one library.
async fn set_up(name: &str) -> (Replay, server::AppState, std::path::PathBuf) {
    let (app, st, dir) = common::server(name, HashMap::new()).await;
    let now = "2026-11-05 18:00:00.000 +00:00";
    let library = "2b3f6c6f-0488-43dc-83b8-7fd501ae4620";
    st.abs
        .execute(
            "INSERT INTO libraries (id, name, displayOrder, mediaType, createdAt, updatedAt) VALUES (?, 'Books', 1, 'book', ?, ?)",
            vec![json!(library), json!(now), json!(now)],
        )
        .unwrap();
    let init = send(&app, "POST", "/init", Some(json!({"newRoot": {"username": "root", "password": "root"}})), &[]).await;
    assert_eq!(init.status, 200);
    let mark =
        json!([{ "libraryItemId": "fd58fdb6-2109-425a-8262-8b8229399fab", "time": 12, "title": "A mark", "createdAt": 1793912499182_i64 }]);
    st.abs.execute("UPDATE users SET bookmarks = ? WHERE type = 'root'", vec![json!(mark.to_string())]).unwrap();
    let login = send(&app, "POST", "/login", Some(json!({"username": "root", "password": "root"})), &[]).await;
    let token = login.body["user"]["token"].as_str().unwrap().to_string();
    let mut m = Matcher::default();
    m.volatile.extend(["backupPath", "expiresAt", "apiKey"].map(String::from));
    let ctx = HashMap::from([("token".to_string(), token), ("library".to_string(), library.to_string())]);
    (Replay { app, ctx, requests: requests(), m }, st, dir)
}
>>>>>>> origin/main

#[tokio::test]
async fn users_are_added_read_changed_and_removed_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-users").await;
    for name in [
        "users_create",
        "users_create_taken",
        "users_create_bad_type",
        "users_create_missing",
        "users_list",
        "user_one",
        "user_missing",
        "user_update",
        "user_update_bad_key",
        "user_update_bad_type",
    ] {
        r.like(name).await;
    }
    // the recording's root had listening progress; Folio's test root has none
    let a = r.send("user_update_root_type").await;
    let mut fx = fixture("abs", "user_update_root_type");
    fx.body["user"]["mediaProgress"] = json!([]);
    assert_like_with(&mut r.m, a.status, &a.body, &fx);
    for name in [
        "user_unlink",
        "reader_login",
        "reader_users_list",
        "reader_user_self",
        "reader_user_other",
        "reader_api_keys",
        "reader_auth_settings",
    ] {
        r.like(name).await;
    }
    // the new user's library list and tags are stored in the permissions, like the original
    let row = st.abs.one("SELECT permissions FROM users WHERE username = 'reader'", vec![]).unwrap().unwrap();
    let p: Value = serde_json::from_str(row["permissions"].as_str().unwrap()).unwrap();
    assert_eq!(p["librariesAccessible"], json!([r.ctx["library"]]));
    assert_eq!(p["itemTagsSelected"], json!(["kids"]));

    for name in ["users_delete_self", "users_delete", "users_delete_missing", "users_list_after"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn a_password_change_signs_out_elsewhere_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-password").await;
    r.send("users_create").await;
    r.send("users_list").await;
    r.send("reader_login").await;
    let old_refresh = r.ctx["reader_refresh"].clone();
    for name in ["me_password_invalid", "me_password_wrong", "me_password_empty"] {
        r.like(name).await;
    }
    let a = r.like("me_password").await;
    assert_eq!(a.cookies, recorded_cookies("me_password"));
    assert_ne!(a.body["user"]["refreshToken"].as_str().unwrap(), old_refresh, "the session's tokens are new");
    r.like("reader_login_old_password").await;
    let a = send(&r.app, "POST", "/login", Some(json!({"username": "reader", "password": "pw2"})), &[]).await;
    assert_eq!(a.status, 200, "the new password signs in");
}

#[tokio::test]
async fn api_keys_sign_in_until_turned_off_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-api-keys").await;
    r.send("users_create").await;
    r.send("users_list").await;
    r.send("user_update").await;
    for name in [
        "api_keys_create",
        "api_keys_create_root",
        "api_keys_create_bad_name",
        "api_keys_create_bad_expiry",
        "api_keys_create_no_user",
        "api_keys_list",
        "api_key_signs_in",
        "api_key_inactive_signs_in",
        "api_keys_update",
        "api_keys_update_bad",
        "api_keys_update_missing",
        "api_key_turned_off_signs_in",
        "api_keys_delete",
        "api_keys_delete_missing",
    ] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn sign_in_settings_are_checked_and_saved_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-auth-settings").await;
    let before = r.send("auth_settings_after").await;
    assert_eq!(before.status, 200);
    for name in [
        "auth_settings_patch",
        "auth_settings_after",
        "status_with_message",
        "auth_settings_patch_none",
        "auth_settings_patch_bad",
        "auth_settings_reset",
    ] {
        r.like(name).await;
    }
    // the reset is stored
    let stored = server::abs::settings::load_setting(&st.abs, "server-settings").unwrap().unwrap();
    assert_eq!(stored["authLoginCustomMessage"], Value::Null);
    assert_eq!(stored["authOpenIDMobileRedirectURIs"], json!(["audiobookshelf://oauth"]));
}
