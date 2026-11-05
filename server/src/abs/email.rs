//! The port of the original's email settings (`server/controllers/EmailController.js` and
//! `objects/settings/EmailSettings.js`): the mail server Folio sends e-books through, and the e-reader devices
//! (Kindle and others by address) with who may send to each, and the sending itself (`managers/EmailManager.js`):
//! a test mail and e-books to devices. Answers are the original's (tests/abs_admin.rs and tests/abs_email.rs).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, user};
use crate::abs::progress::send_text;
use crate::abs::settings::{device_allowed, js_number, load_setting, save_setting, truthy};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use lettre::address::AddressError;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::{json, Map, Value};
use std::time::Duration;

/// The routes, under `/api/emails`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/emails/settings", get(settings).patch(update_settings))
        .route("/api/emails/test", post(send_test))
        .route("/api/emails/ereader-devices", post(update_devices))
        .route("/api/emails/send-ebook-to-device", post(send_ebook))
}

/// The settings' keys besides `id`, in the original's order (`toJSON`).
const KEYS: &[&str] = &["host", "port", "secure", "rejectUnauthorized", "user", "pass", "testAddress", "fromAddress", "ereaderDevices"];

/// Who may send to a device (`availabilityOption`).
const AVAILABILITY: &[&str] = &["adminOrUp", "userOrUp", "guestOrUp", "specificUsers"];

/// The email settings as stored, or the defaults (`new EmailSettings(stored)`).
pub fn load(db: &dyn Db) -> Result<Map<String, Value>> {
    let stored = load_setting(db, "email-settings")?.and_then(|v| v.as_object().cloned());
    let mut m = Map::new();
    m.insert("id".into(), json!("email-settings"));
    match stored {
        None => {
            for (k, v) in [
                ("host", Value::Null),
                ("port", json!(465)),
                ("secure", json!(true)),
                ("rejectUnauthorized", json!(true)),
                ("user", Value::Null),
                ("pass", Value::Null),
                ("testAddress", Value::Null),
                ("fromAddress", Value::Null),
                ("ereaderDevices", json!([])),
            ] {
                m.insert(k.into(), v);
            }
        }
        Some(s) => {
            // `construct`: a missing value stays missing (left out of the JSON), like the original's undefined
            for k in ["host", "port", "user", "pass", "testAddress", "fromAddress"] {
                if let Some(v) = s.get(k) {
                    m.insert(k.into(), v.clone());
                }
            }
            m.insert("secure".into(), json!(truthy(s.get("secure"))));
            let reject = s.get("rejectUnauthorized").is_none_or(|v| truthy(Some(v)));
            m.insert("rejectUnauthorized".into(), json!(reject));
            m.insert("ereaderDevices".into(), s.get("ereaderDevices").filter(|d| d.is_array()).cloned().unwrap_or_else(|| json!([])));
        }
    }
    Ok(ordered(&m))
}

/// The settings in the original's key order.
fn ordered(m: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), json!("email-settings"));
    for k in KEYS {
        if let Some(v) = m.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    out
}

/// JavaScript's `isNullOrNaN`.
fn null_or_nan(v: &Value) -> bool {
    v.is_null() || js_number(Some(v)).is_none()
}

/// A number the way the API writes it: whole numbers without a fraction.
fn number(n: f64) -> Value {
    crate::abs::models::media_progress::js_json(Some(n))
}

/// The devices a change keeps (`update`): those with a name and an address; an unknown or impossible
/// availability becomes "admins", and only "specific users" keeps a user list.
fn clean_devices(devices: &[Value]) -> Vec<Value> {
    devices
        .iter()
        .filter_map(|d| {
            let mut d = d.as_object()?.clone();
            if !truthy(d.get("name")) || !truthy(d.get("email")) {
                tracing::error!("An e-reader device without a name or address was left out");
                return None;
            }
            let option = d.get("availabilityOption").and_then(Value::as_str).filter(|o| AVAILABILITY.contains(o));
            if option.is_none() {
                d.insert("availabilityOption".into(), json!("adminOrUp"));
            }
            let has_users = d.get("users").and_then(Value::as_array).is_some_and(|u| !u.is_empty());
            if d.get("availabilityOption") == Some(&json!("specificUsers")) && !has_users {
                d.insert("availabilityOption".into(), json!("adminOrUp"));
            }
            if d.get("availabilityOption") != Some(&json!("specificUsers")) && has_users {
                d.insert("users".into(), json!([]));
            }
            Some(Value::Object(d))
        })
        .collect()
}

/// Applies a change (`EmailSettings.update`): the port as a number (465 when not one), `secure` and
/// `rejectUnauthorized` as true or false, the devices cleaned; empty text is stored as nothing. Answers whether
/// anything changed.
pub fn update(current: &mut Map<String, Value>, payload: &Map<String, Value>) -> bool {
    let mut p = payload.clone();
    if let Some(port) = p.get("port").cloned() {
        let n = if null_or_nan(&port) { 465.0 } else { js_number(Some(&port)).unwrap_or(465.0) };
        p.insert("port".into(), number(n));
    }
    for k in ["secure", "rejectUnauthorized"] {
        if let Some(v) = p.get(k).cloned() {
            p.insert(k.into(), json!(truthy(Some(&v))));
        }
    }
    if p.get("ereaderDevices").is_some_and(|d| !d.is_array()) {
        p.remove("ereaderDevices");
    }
    if let Some(Value::Array(devices)) = p.get("ereaderDevices").cloned() {
        if !devices.is_empty() {
            p.insert("ereaderDevices".into(), Value::Array(clean_devices(&devices)));
        }
    }
    let mut changed = false;
    for k in KEYS {
        let Some(new) = p.get(*k) else { continue };
        if current.get(*k) != Some(new) {
            // `copyValue`: empty text is stored as nothing
            let v = if new == &json!("") { Value::Null } else { new.clone() };
            current.insert((*k).to_string(), v);
            changed = true;
        }
    }
    *current = ordered(current);
    changed
}

/// The original's check for the settings routes (`adminMiddleware`): others get 404.
fn admin_only(me: &models::User) -> Option<Response> {
    (!user::is_admin_or_up(me)).then(|| send_status(StatusCode::NOT_FOUND))
}

/// `GET /api/emails/settings` (admins only).
async fn settings(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    match load(&st.abs) {
        Ok(s) => Json(json!({ "settings": s })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `PATCH /api/emails/settings` (admins only): changes and stores the settings.
async fn update_settings(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let fields = body_fields(&headers, &body);
    let done = (|| -> Result<Map<String, Value>> {
        let mut s = load(&st.abs)?;
        if update(&mut s, &fields) {
            save_setting(&st.abs, "email-settings", &Value::Object(s.clone()))?;
        }
        Ok(s)
    })();
    match done {
        Ok(s) => Json(json!({ "settings": s })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /api/emails/ereader-devices` (admins only): replaces the e-reader devices; every device needs a name
/// and an address. Admins' apps are told the new list.
async fn update_devices(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let fields = body_fields(&headers, &body);
    let Some(devices) = fields.get("ereaderDevices").and_then(Value::as_array) else {
        return send_text(StatusCode::BAD_REQUEST, "Invalid payload. ereaderDevices array required");
    };
    if devices.iter().any(|d| !truthy(d.get("name")) || !truthy(d.get("email"))) {
        return send_text(StatusCode::BAD_REQUEST, "Invalid payload. ereaderDevices array items must have name and email");
    }
    let done = (|| -> Result<Value> {
        let mut s = load(&st.abs)?;
        let mut change = Map::new();
        change.insert("ereaderDevices".into(), Value::Array(devices.clone()));
        if update(&mut s, &change) {
            save_setting(&st.abs, "email-settings", &Value::Object(s.clone()))?;
            st.sockets.admin_emitter("ereader-devices-updated", &json!({ "ereaderDevices": s["ereaderDevices"] }));
        }
        Ok(s["ereaderDevices"].clone())
    })();
    match done {
        Ok(d) => Json(json!({ "ereaderDevices": d })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /api/emails/send-ebook-to-device`: sends an item's e-book to a device by name (`deviceName`,
/// `libraryItemId`), after the original's checks: the device exists and the user may use it, the item exists,
/// the user may see it and it has an e-book. The e-book goes as an attachment (`EmailManager.sendEBookToDevice`).
async fn send_ebook(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    let fields = body_fields(&headers, &body);
    let devices = match load(&st.abs) {
        Ok(s) => s.get("ereaderDevices").and_then(Value::as_array).cloned().unwrap_or_default(),
        Err(e) => return e.into_response(),
    };
    let Some(device) = devices.iter().find(|d| d.get("name").is_some() && d.get("name") == fields.get("deviceName")) else {
        return send_text(StatusCode::NOT_FOUND, "Ereader device not found");
    };
    if !device_allowed(device, &me.user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let item_id = fields.get("libraryItemId").and_then(Value::as_str).unwrap_or_default();
    let row = match st.abs.one(
        "SELECT li.libraryId AS libraryId, b.explicit AS explicit, b.tags AS tags, b.ebookFile AS ebookFile \
         FROM libraryItems li LEFT JOIN books b ON b.id = li.mediaId WHERE li.id = ?",
        vec![json!(item_id)],
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return send_text(StatusCode::NOT_FOUND, "Library item not found"),
        Err(e) => return e.into_response(),
    };
    let library = row.get("libraryId").and_then(Value::as_str).unwrap_or_default();
    let explicit = row.get("explicit").and_then(Value::as_i64).unwrap_or(0) != 0;
    let tags: Vec<String> = row.get("tags").and_then(Value::as_str).and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    if !user::can_access_item(&me.user, library, explicit, &tags) {
        return send_status(StatusCode::FORBIDDEN);
    }
    if row.get("ebookFile").is_none_or(|f| f.is_null() || f == &json!("null")) {
        return send_text(StatusCode::NOT_FOUND, "Ebook file not found");
    }
    let ebook: Value = row.get("ebookFile").and_then(Value::as_str).and_then(|f| serde_json::from_str(f).ok()).unwrap_or_default();
    let file_name = ebook["metadata"]["filename"].as_str().unwrap_or_default().to_string();
    let path = ebook["metadata"]["path"].as_str().unwrap_or_default().to_string();
    let to = device.get("email").and_then(Value::as_str).unwrap_or_default().to_string();
    let device_name = device.get("name").and_then(Value::as_str).unwrap_or_default();
    tracing::info!("Sending ebook \"{file_name}\" to device \"{device_name}\"/\"{to}\"");
    let s = match load(&st.abs) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let Some(transport) = verified(&s).await else {
        return send_text(StatusCode::BAD_REQUEST, "Failed to verify SMTP connection configuration");
    };
    let sent = async {
        let bytes = tokio::fs::read(&path).await.map_err(|e| format!("The e-book file {path} can't be read: {e}"))?;
        let kind =
            ContentType::parse(mime_guess::from_path(&file_name).first_or_octet_stream().essence_str()).map_err(|e| e.to_string())?;
        let body = MultiPart::mixed()
            .singlepart(SinglePart::html(String::from("<div dir=\"auto\"></div>")))
            .singlepart(Attachment::new(file_name.clone()).body(bytes, kind));
        let mail = message(&s, &to, "Here is your Ebook!")?.multipart(body).map_err(|e| e.to_string())?;
        transport.send(mail).await.map_err(|e| e.to_string())
    };
    match sent.await {
        Ok(_) => {
            tracing::info!("Ebook sent to device successfully");
            send_status(StatusCode::OK)
        }
        Err(e) => {
            tracing::error!("Failed to send ebook to device: {e}");
            send_text(StatusCode::BAD_REQUEST, if e.is_empty() { "Failed to send ebook to device" } else { &e })
        }
    }
}

/// `POST /api/emails/test` (admins only): checks the mail server and sends a test mail to the test address, or
/// to the sender's address when there is none (`EmailManager.sendTest`).
async fn send_test(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    tracing::info!("Sending test email");
    let s = match load(&st.abs) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    let Some(transport) = verified(&s).await else {
        return send_text(StatusCode::BAD_REQUEST, "Failed to verify SMTP connection configuration");
    };
    let to = [text(&s, "testAddress"), text(&s, "fromAddress")].into_iter().flatten().next().unwrap_or_default();
    let sent = async {
        let mail = message(&s, &to, "Test email from Audiobookshelf")?.body(String::from("Success!")).map_err(|e| e.to_string())?;
        transport.send(mail).await.map_err(|e| e.to_string())
    };
    match sent.await {
        Ok(_) => {
            tracing::info!("Test email sent successfully");
            send_status(StatusCode::OK)
        }
        Err(e) => {
            tracing::error!("Failed to send test email: {e}");
            send_text(StatusCode::BAD_REQUEST, if e.is_empty() { "Failed to send test email" } else { &e })
        }
    }
}

/// A setting that holds text, when it isn't empty (JavaScript's truthy strings).
fn text(s: &Map<String, Value>, key: &str) -> Option<String> {
    s.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from)
}

/// How the mail server is reached (`getTransportObject` with nodemailer's defaults): the host (`localhost` when
/// none is set), the port (465 with `secure`, otherwise 587, when none is set), TLS from the start only on port
/// 465 and otherwise when the server offers it, the sign-in when a user is set, and certificates that may be
/// self-signed when `rejectUnauthorized` is off.
pub fn transport(s: &Map<String, Value>) -> std::result::Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = text(s, "host").unwrap_or_else(|| "localhost".into());
    let set_port = s.get("port").and_then(Value::as_f64).filter(|p| *p != 0.0);
    let secure = truthy(s.get("secure")) && set_port == Some(465.0);
    let port = set_port.map(|p| p as u16).unwrap_or(if secure { 465 } else { 587 });
    let strict = s.get("rejectUnauthorized").is_none_or(|v| truthy(Some(v)));
    let params = TlsParameters::builder(host.clone())
        .dangerous_accept_invalid_certs(!strict)
        .dangerous_accept_invalid_hostnames(!strict)
        .build()
        .map_err(|e| e.to_string())?;
    let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
        .port(port)
        .tls(if secure { Tls::Wrapper(params) } else { Tls::Opportunistic(params) })
        .timeout(Some(Duration::from_secs(120)));
    if let (Some(user), Some(pass)) = (text(s, "user"), s.get("pass")) {
        b = b.credentials(Credentials::new(user, pass.as_str().unwrap_or_default().to_string()));
    }
    Ok(b.build())
}

/// The transport, once the mail server answered and accepted the sign-in (nodemailer's `verify`); `None`, with
/// the reason in the log, when it didn't.
async fn verified(s: &Map<String, Value>) -> Option<AsyncSmtpTransport<Tokio1Executor>> {
    let checked = match transport(s) {
        Ok(t) => t.test_connection().await.map_err(|e| e.to_string()).and_then(|ok| if ok { Ok(t) } else { Err("no answer".into()) }),
        Err(e) => Err(e),
    };
    checked.map_err(|e| tracing::error!("Failed to verify SMTP connection config: {e}")).ok()
}

/// A mail from the sender's address (`fromAddress`) to `to` with the subject, ready for its body.
fn message(s: &Map<String, Value>, to: &str, subject: &str) -> std::result::Result<MessageBuilder, String> {
    let from: Mailbox = text(s, "fromAddress").unwrap_or_default().parse().map_err(|e: AddressError| e.to_string())?;
    let to: Mailbox = to.parse().map_err(|e: AddressError| e.to_string())?;
    Ok(Message::builder().from(from).to(to).subject(subject))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_cleaned_like_the_original() {
        let out = clean_devices(&[
            json!({"name": "A", "email": "a@x"}),
            json!({"name": "B", "email": "b@x", "availabilityOption": "specificUsers", "users": []}),
            json!({"name": "C", "email": "c@x", "availabilityOption": "guestOrUp", "users": ["u"]}),
            json!({"name": "D"}),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], json!({"name": "A", "email": "a@x", "availabilityOption": "adminOrUp"}));
        assert_eq!(out[1]["availabilityOption"], "adminOrUp");
        assert_eq!(out[2]["users"], json!([]));
    }

    #[test]
    fn the_port_is_a_number() {
        let mut s = Map::new();
        assert!(update(&mut s, json!({"port": "587"}).as_object().unwrap()));
        assert_eq!(s["port"], json!(587));
        assert!(update(&mut s, json!({"port": "abc"}).as_object().unwrap()));
        assert_eq!(s["port"], json!(465));
        assert!(!update(&mut s, json!({"port": 465}).as_object().unwrap()));
    }
}
