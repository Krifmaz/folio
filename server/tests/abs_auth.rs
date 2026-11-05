//! The Audiobookshelf port's sign-in (PARITY P2): setup, `/status`, `/login`, `/auth/refresh`, `/logout`, the
//! kinds of token, `/api/authorize`, `/api/me` and unknown addresses, each compared with the answer recorded
//! from the pinned original (tests/parity/fixtures/abs).

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

/// Sends a request with optional JSON body and headers.
async fn send(app: &Router, method: &str, path: &str, body: Option<Value>, headers: &[(&str, &str)]) -> Answer {
    let mut b = Request::builder().method(method).uri(path);
    for (k, v) in headers {
        b = b.header(*k, *v);
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

/// A matcher for one test; the backup folder is Folio's data folder, so it is compared by type only.
fn matcher() -> Matcher {
    let mut m = Matcher::default();
    m.volatile.push("backupPath".into());
    m
}

/// Checks status, content type and body against a recording.
fn like(m: &mut Matcher, a: &Answer, name: &str) {
    let fx = fixture("abs", name);
    assert_eq!(a.content_type, fx.content_type, "{name}: content type");
    assert_like_with(m, a.status, &a.body, &fx);
}

/// The cookies a recording set (other than are express-session's, which Folio doesn't use).
fn recorded_cookies(name: &str) -> Vec<String> {
    let path = parity::dir("abs").join(format!("{name}.json"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|c| !c.starts_with("connect.sid"))
        .map(str::to_string)
        .collect()
}

/// A set-up server with the root user (root / root) and one library, like the recordings had.
async fn set_up(name: &str) -> (Router, server::AppState, std::path::PathBuf) {
    let (app, st, dir) = common::server(name, HashMap::new()).await;
    let now = "2026-11-05 18:00:00.000 +00:00";
    st.abs
        .execute(
            "INSERT INTO libraries (id, name, displayOrder, mediaType, createdAt, updatedAt) VALUES (?, 'Books', 1, 'book', ?, ?)",
            vec![json!("2b3f6c6f-0488-43dc-83b8-7fd501ae4620"), json!(now), json!(now)],
        )
        .unwrap();
    let a = send(&app, "POST", "/init", Some(json!({"newRoot": {"username": "root", "password": "root"}})), &[]).await;
    assert_eq!((a.status, a.body.clone()), (200, json!("OK")));
    (app, st, dir)
}

#[tokio::test]
async fn a_new_server_asks_for_setup_then_answers_like_the_original() {
    let (app, _st, _dir) = common::server("abs-auth-new", HashMap::new()).await;
    let a = send(&app, "GET", "/status", None, &[]).await;
    assert_eq!(a.body["isInit"], false);
    assert!(a.body["ConfigPath"].is_string() && a.body["MetadataPath"].is_string(), "{}", a.body);
    let a = send(&app, "POST", "/init", Some(json!({"newRoot": {"username": "root", "password": "root"}})), &[]).await;
    assert_eq!(a.status, 200);
    let mut m = matcher();
    like(&mut m, &send(&app, "POST", "/init", Some(json!({"newRoot": {"username": "x", "password": "y"}})), &[]).await, "init_again");
    like(&mut m, &send(&app, "GET", "/status", None, &[]).await, "status");
    like(&mut m, &send(&app, "GET", "/ping", None, &[]).await, "ping");
    like(&mut m, &send(&app, "GET", "/healthcheck", None, &[]).await, "healthcheck");
}

#[tokio::test]
async fn sign_in_refresh_and_sign_out_answer_like_the_original() {
    let (app, _st, _dir) = set_up("abs-auth-flow").await;
    let mut m = matcher();

    // the mobile app: tokens in the answer
    let login = send(&app, "POST", "/login", Some(json!({"username": "root", "password": "root"})), &[("x-return-tokens", "true")]).await;
    like(&mut m, &login, "login");
    assert!(login.cookies.iter().all(|c| !c.starts_with("refresh_token")), "{:?}", login.cookies);
    let access = login.body["user"]["accessToken"].as_str().unwrap().to_string();
    let legacy = login.body["user"]["token"].as_str().unwrap().to_string();
    let refresh = login.body["user"]["refreshToken"].as_str().unwrap().to_string();

    // the web app: the refresh token as a cookie; names sign in in any case
    let web = send(&app, "POST", "/login", Some(json!({"username": "ROOT", "password": "root"})), &[]).await;
    like(&mut m, &web, "login_web");
    assert_eq!(web.cookies, recorded_cookies("login_web"));

    for name in ["login_bad_password", "login_unknown_user", "login_missing", "refresh_missing", "refresh_bad"] {
        let fx = fixture("abs", name);
        let headers: Vec<(String, String)> = fx.request["headers"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let body = Some(fx.request["body"].clone()).filter(|b| !b.is_null());
        like(&mut m, &send(&app, fx.request["method"].as_str().unwrap(), fx.request["path"].as_str().unwrap(), body, &h).await, name);
    }
    like(&mut m, &send(&app, "POST", "/auth/refresh", None, &[("x-refresh-token", &access)]).await, "refresh_access_token");

    let r = send(&app, "POST", "/auth/refresh", None, &[("x-refresh-token", &refresh)]).await;
    like(&mut m, &r, "refresh");
    assert_eq!(r.cookies, recorded_cookies("refresh"));
    let refresh2 = r.body["user"]["refreshToken"].as_str().unwrap().to_string();
    assert_ne!(refresh2, refresh);
    let again = send(&app, "POST", "/auth/refresh", None, &[("x-refresh-token", &refresh)]).await;
    like(&mut m, &again, "refresh_grace");
    assert_eq!(again.body["user"]["refreshToken"], json!(refresh2), "the old token gets the current one during the grace period");

    // the kinds of token
    like(&mut m, &send(&app, "GET", &format!("/api/me?token={access}"), None, &[]).await, "me_query_token");
    let bearer = |t: &str| format!("Bearer {t}");
    like(&mut m, &send(&app, "GET", "/api/me", None, &[("authorization", &bearer(&legacy))]).await, "me_legacy_token");
    like(&mut m, &send(&app, "GET", "/api/me", None, &[("authorization", &bearer(&refresh2))]).await, "me_refresh_token");
    like(&mut m, &send(&app, "GET", "/api/me", None, &[("authorization", "Bearer not-a-token")]).await, "me_bad_token");
    like(&mut m, &send(&app, "GET", "/api/me", None, &[("authorization", &bearer(&access))]).await, "me");
    like(&mut m, &send(&app, "POST", "/api/authorize", None, &[("authorization", &bearer(&access))]).await, "authorize");

    // unknown addresses: a token first, then Express's page
    like(&mut m, &send(&app, "GET", "/api/libraries", None, &[]).await, "unauthorized");
    like(&mut m, &send(&app, "GET", "/api/does-not-exist", None, &[("authorization", &bearer(&access))]).await, "not_found");
    like(&mut m, &send(&app, "POST", "/api/does-not-exist", None, &[("authorization", &bearer(&access))]).await, "not_found_post");

    // sign out ends the session
    let out = send(&app, "POST", "/logout", None, &[("x-refresh-token", &refresh2)]).await;
    like(&mut m, &out, "logout");
    let cleared: Vec<String> = recorded_cookies("logout");
    assert_eq!(out.cookies, cleared);
    like(&mut m, &send(&app, "POST", "/auth/refresh", None, &[("x-refresh-token", &refresh2)]).await, "refresh_after_logout");
}

#[tokio::test]
async fn too_many_sign_in_attempts_are_refused() {
    let (app, _st, _dir) = set_up("abs-auth-limit").await;
    for _ in 0..40 {
        let a = send(&app, "POST", "/login", Some(json!({"username": "nobody", "password": "x"})), &[]).await;
        assert_eq!(a.status, 401);
    }
    let a = send(&app, "POST", "/login", Some(json!({"username": "root", "password": "root"})), &[]).await;
    assert_eq!((a.status, a.body), (429, json!({"error": "Too many authentication requests"})));
    let other =
        send(&app, "POST", "/login", Some(json!({"username": "root", "password": "root"})), &[("x-forwarded-for", "10.1.2.3")]).await;
    assert_eq!(other.status, 200, "another address isn't limited");
}

#[tokio::test]
async fn a_passwordless_root_signs_in_with_an_empty_password_only() {
    let (app, _st, _dir) = common::server("abs-auth-nopass", HashMap::new()).await;
    send(&app, "POST", "/init", Some(json!({"newRoot": {"username": "admin", "password": ""}})), &[]).await;
    let a = send(&app, "POST", "/login", Some(json!({"username": "admin", "password": ""})), &[]).await;
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.body["user"]["username"], "admin");
    let a = send(&app, "POST", "/login", Some(json!({"username": "admin", "password": "guess"})), &[]).await;
    assert_eq!(a.status, 401);
}
