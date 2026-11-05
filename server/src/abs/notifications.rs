//! The port of the original's notifications (`server/controllers/NotificationController.js`,
//! `managers/NotificationManager.js`, `objects/settings/NotificationSettings.js`, `objects/Notification.js`,
//! `utils/notifications.js`): admins point Folio at an Apprise server and set up notifications for events
//! (a podcast episode downloaded, a backup done or failed, an RSS feed failing, a test). When an event happens,
//! each enabled notification for it is sent to Apprise with its title and body filled in; one that keeps failing
//! is turned off. Events that come while one is being sent wait in a queue. Answers are the original's
//! (tests/abs_notifications.rs).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::{self, media_progress::js_json, user};
use crate::abs::progress::send_text;
use crate::abs::settings::{js_number, load_setting, save_setting, truthy};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// The events notifications can be set up for, with their variables, default texts and test values
/// (`notificationData`, copied from the original).
const DATA: &str = include_str!("notification_data.json");

/// The events data as JSON.
pub fn data() -> &'static Value {
    static PARSED: OnceLock<Value> = OnceLock::new();
    PARSED.get_or_init(|| serde_json::from_str(DATA).unwrap_or_else(|_| json!({ "events": [] })))
}

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/notifications", get(get_all).patch(update).post(create))
        .route("/api/notificationdata", get(get_data))
        .route("/api/notifications/test", get(fire_test))
        .route("/api/notifications/{id}", axum::routing::delete(remove).patch(update_one))
        .route("/api/notifications/{id}/test", get(send_test))
}

/// One notification as stored and sent to the apps (`Notification.toJSON`); text templates the original never
/// set are left out.
fn notification_json(n: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for k in [
        "id",
        "libraryId",
        "eventName",
        "urls",
        "titleTemplate",
        "bodyTemplate",
        "enabled",
        "type",
        "lastFiredAt",
        "lastAttemptFailed",
        "numConsecutiveFailedAttempts",
        "numTimesFired",
        "createdAt",
    ] {
        if let Some(v) = n.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out
}

/// A stored notification read back (`new Notification(stored)`).
fn construct_notification(n: &Value) -> Map<String, Value> {
    let get = |k: &str| n.get(k).cloned();
    let or = |k: &str, d: Value| get(k).filter(|v| truthy(Some(v))).unwrap_or(d);
    let mut m = Map::new();
    m.insert("id".into(), get("id").unwrap_or(Value::Null));
    m.insert("libraryId".into(), or("libraryId", Value::Null));
    if let Some(e) = get("eventName") {
        m.insert("eventName".into(), e);
    }
    m.insert("urls".into(), or("urls", json!([])));
    m.insert("titleTemplate".into(), or("titleTemplate", json!("")));
    m.insert("bodyTemplate".into(), or("bodyTemplate", json!("")));
    m.insert("enabled".into(), json!(truthy(n.get("enabled"))));
    m.insert("type".into(), or("type", json!("info")));
    m.insert("lastFiredAt".into(), or("lastFiredAt", Value::Null));
    m.insert("lastAttemptFailed".into(), json!(truthy(n.get("lastAttemptFailed"))));
    m.insert("numConsecutiveFailedAttempts".into(), or("numConsecutiveFailedAttempts", json!(0)));
    m.insert("numTimesFired".into(), or("numTimesFired", json!(0)));
    if let Some(c) = get("createdAt") {
        m.insert("createdAt".into(), c);
    }
    m
}

/// The notification settings (`NotificationSettings`): the Apprise address, the notifications and the limits.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings(pub Map<String, Value>);

impl Settings {
    /// The defaults (`new NotificationSettings()`).
    pub fn defaults() -> Settings {
        let mut m = Map::new();
        m.insert("id".into(), json!("notification-settings"));
        m.insert("appriseType".into(), json!("api"));
        m.insert("appriseApiUrl".into(), Value::Null);
        m.insert("notifications".into(), json!([]));
        m.insert("maxFailedAttempts".into(), json!(5));
        m.insert("maxNotificationQueue".into(), json!(20));
        m.insert("notificationDelay".into(), json!(1000));
        Settings(m)
    }

    /// The stored settings, or the defaults (`construct`).
    pub fn load(db: &dyn Db) -> Result<Settings> {
        let Some(Value::Object(s)) = load_setting(db, "notification-settings")? else { return Ok(Settings::defaults()) };
        let or = |k: &str, d: Value| s.get(k).cloned().filter(|v| truthy(Some(v))).unwrap_or(d);
        let mut m = Map::new();
        m.insert("id".into(), json!("notification-settings"));
        if let Some(t) = s.get("appriseType") {
            m.insert("appriseType".into(), t.clone());
        }
        m.insert("appriseApiUrl".into(), or("appriseApiUrl", Value::Null));
        let list: Vec<Value> = s
            .get("notifications")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|n| Value::Object(construct_notification(n)))
            .collect();
        m.insert("notifications".into(), Value::Array(list));
        m.insert("maxFailedAttempts".into(), or("maxFailedAttempts", json!(5)));
        m.insert("maxNotificationQueue".into(), or("maxNotificationQueue", json!(20)));
        m.insert("notificationDelay".into(), or("notificationDelay", json!(1000)));
        Ok(Settings(m))
    }

    /// Stores the settings (`Database.updateSetting`).
    pub fn save(&self, db: &dyn Db) -> Result<()> {
        save_setting(db, "notification-settings", &self.to_json())
    }

    /// The settings as the apps see them (`toJSON`).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for k in ["id", "appriseType", "appriseApiUrl", "notifications", "maxFailedAttempts", "maxNotificationQueue", "notificationDelay"] {
            if let Some(v) = self.0.get(k) {
                let v = if k == "notifications" {
                    Value::Array(
                        v.as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_object)
                            .map(|n| Value::Object(notification_json(n)))
                            .collect(),
                    )
                } else {
                    v.clone()
                };
                m.insert(k.into(), v);
            }
        }
        Value::Object(m)
    }

    /// The notifications, editable.
    fn list(&mut self) -> &mut Vec<Value> {
        if !self.0.get("notifications").is_some_and(Value::is_array) {
            self.0.insert("notifications".into(), json!([]));
        }
        match self.0.get_mut("notifications") {
            Some(Value::Array(a)) => a,
            _ => unreachable!("set just above"),
        }
    }

    /// Whether an Apprise address is set (`isUseable`).
    pub fn is_useable(&self) -> bool {
        truthy(self.0.get("appriseApiUrl"))
    }

    /// A number setting.
    fn number(&self, key: &str, default: f64) -> f64 {
        js_number(self.0.get(key)).unwrap_or(default)
    }

    /// One notification by id (`getNotification`).
    pub fn get(&self, id: &str) -> Option<Map<String, Value>> {
        self.0
            .get("notifications")?
            .as_array()?
            .iter()
            .filter_map(Value::as_object)
            .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
            .cloned()
    }

    /// Applies a change of the Apprise address and limits (`update`). Like the original, a limit left out goes
    /// back to its default, and so does the address.
    pub fn update(&mut self, payload: &Map<String, Value>) -> bool {
        let mut changed = false;
        // `payload.appriseApiUrl !== this.appriseApiUrl`: a missing address always counts as a change
        let url = payload.get("appriseApiUrl");
        if url.is_none() || url != self.0.get("appriseApiUrl") {
            let v = url.filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null);
            self.0.insert("appriseApiUrl".into(), v);
            changed = true;
        }
        for (k, d) in [("maxFailedAttempts", 5.0), ("maxNotificationQueue", 20.0)] {
            let sent = payload.get(k);
            let n = match sent {
                None | Some(Value::Null) => d,
                Some(v) => js_number(Some(v)).unwrap_or(d),
            };
            let n = js_json(Some(n));
            if self.0.get(k) != Some(&n) {
                self.0.insert(k.into(), n);
                changed = true;
            }
        }
        changed
    }

    /// Adds a notification (`createNotification`); `None` when the payload can't make one, `Err` where the
    /// original fails (no `urls` at all).
    pub fn create(&mut self, payload: &Map<String, Value>) -> std::result::Result<bool, NoUrls> {
        let urls_len = match payload.get("urls") {
            None | Some(Value::Null) => {
                if !truthy(payload.get("eventName")) {
                    return Ok(false);
                }
                return Err(NoUrls);
            }
            Some(Value::Array(a)) => a.len(),
            Some(Value::String(s)) => s.encode_utf16().count(),
            Some(_) => 0,
        };
        if !truthy(payload.get("eventName")) || urls_len == 0 {
            return Ok(false);
        }
        let mut n = Map::new();
        n.insert("id".into(), json!(models::new_id()));
        n.insert("libraryId".into(), payload.get("libraryId").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        n.insert("eventName".into(), payload["eventName"].clone());
        n.insert("urls".into(), payload["urls"].clone());
        for k in ["titleTemplate", "bodyTemplate"] {
            if let Some(v) = payload.get(k) {
                n.insert(k.into(), v.clone());
            }
        }
        n.insert("enabled".into(), json!(truthy(payload.get("enabled"))));
        n.insert("type".into(), payload.get("type").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        n.insert("lastFiredAt".into(), Value::Null);
        n.insert("lastAttemptFailed".into(), json!(false));
        n.insert("numConsecutiveFailedAttempts".into(), json!(0));
        n.insert("numTimesFired".into(), json!(0));
        n.insert("createdAt".into(), json!(models::now_ms()));
        self.list().push(Value::Object(n));
        Ok(true)
    }

    /// Changes the notification the payload names by its `id` (`updateNotification`, `Notification.update`).
    pub fn update_notification(&mut self, payload: &Map<String, Value>) -> bool {
        let id = payload.get("id").cloned();
        let Some(Value::Object(n)) = self.list().iter_mut().find(|n| n.get("id").is_some() && n.get("id") == id.as_ref()) else {
            tracing::error!("There is no notification {}", id.as_ref().map(js_text).unwrap_or_default());
            return false;
        };
        if !truthy(n.get("enabled")) && truthy(payload.get("enabled")) {
            n.insert("lastFiredAt".into(), Value::Null);
            n.insert("lastAttemptFailed".into(), json!(false));
            n.insert("numConsecutiveFailedAttempts".into(), json!(0));
        }
        let mut changed = false;
        for k in ["libraryId", "eventName", "urls", "titleTemplate", "bodyTemplate", "enabled", "type"] {
            let Some(v) = payload.get(k) else { continue };
            if k == "urls" {
                let joined = |x: Option<&Value>| x.and_then(Value::as_array).map(|a| a.iter().map(js_text).collect::<Vec<_>>().join(","));
                if joined(Some(v)) != joined(n.get("urls")) {
                    n.insert("urls".into(), v.clone());
                    changed = true;
                }
            } else if n.get(k) != Some(v) {
                n.insert(k.into(), v.clone());
                changed = true;
            }
        }
        changed
    }

    /// Removes a notification (`removeNotification`).
    pub fn remove(&mut self, id: &str) -> bool {
        let list = self.list();
        let before = list.len();
        list.retain(|n| n.get("id").and_then(Value::as_str) != Some(id));
        list.len() != before
    }
}

/// A notification sent without any `urls`, where the original fails with a server error.
#[derive(Debug)]
pub struct NoUrls;

/// An event waiting to be sent: its name and values.
type Queued = (String, Map<String, Value>);

/// A value as JavaScript's `String()` writes it in a list.
fn js_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A template with `{{name}}` (or `{{ name }}`) replaced by the event's values; names without a value stay
/// (`replaceVariablesInTemplate`).
pub fn fill_template(template: &str, data: &Map<String, Value>) -> String {
    let mut out = template.to_string();
    let mut i = 0;
    let b = template.as_bytes();
    while i + 1 < b.len() {
        if &b[i..i + 2] != b"{{" {
            i += 1;
            continue;
        }
        let mut j = i + 2;
        if j < b.len() && b[j] == b' ' {
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
        }
        let name = &template[start..j];
        if j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if !name.is_empty() && template.get(j..j + 2) == Some("}}") {
            let whole = &template[i..j + 2];
            if let Some(v) = data.get(name).filter(|v| truthy(Some(v))) {
                out = out.replacen(whole, &js_text(v), 1);
            }
            i = j + 2;
        } else {
            i += 1;
        }
    }
    out
}

/// The body sent to Apprise (`getApprisePayload`).
fn apprise_payload(n: &Map<String, Value>, data: &Map<String, Value>) -> Value {
    let text = |k: &str| n.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    json!({
        "urls": n.get("urls").cloned().unwrap_or(json!([])),
        "title": fill_template(&text("titleTemplate"), data),
        "body": fill_template(&text("bodyTemplate"), data)
    })
}

/// Sends one notification to Apprise (`sendNotification`); true when Apprise accepted it.
async fn send(st: &AppState, url: &str, n: &Map<String, Value>, data: &Map<String, Value>) -> bool {
    let r = st.http.post(url).json(&apprise_payload(n, data)).timeout(std::time::Duration::from_secs(6)).send().await;
    match r {
        Ok(resp) if resp.status().is_success() => true,
        Ok(resp) => {
            tracing::error!("Apprise refused the notification {}: {}", n.get("id").map(js_text).unwrap_or_default(), resp.status());
            false
        }
        Err(e) => {
            tracing::error!("The notification {} couldn't reach Apprise: {e}", n.get("id").map(js_text).unwrap_or_default());
            false
        }
    }
}

/// The queue of events that came while one was being sent (`NotificationManager`).
#[derive(Default)]
pub struct Manager {
    /// Whether an event is being sent, and the events waiting.
    state: Mutex<(bool, VecDeque<Queued>)>,
    /// The settings in effect, read from the database once (`Database.notificationSettings`), like the original
    /// keeps them in memory.
    cache: Mutex<Option<Settings>>,
}

/// The settings in effect (read from the database on first use).
pub fn current(st: &AppState) -> Result<Settings> {
    let mut c = st.abs_server.notifications.cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = c.as_ref() {
        return Ok(s.clone());
    }
    let s = Settings::load(&st.abs)?;
    *c = Some(s.clone());
    Ok(s)
}

/// Whether notifications can be sent and one is turned on for an event (`isUseable` and
/// `getHasActiveNotificationsForEvent`).
pub fn has_active(st: &AppState, event: &str) -> bool {
    let Ok(mut s) = current(st) else { return false };
    s.is_useable() && s.list().iter().any(|n| n.get("eventName").and_then(Value::as_str) == Some(event) && truthy(n.get("enabled")))
}

/// Drops the settings in effect so they are read again from the database (after a backup is restored).
pub fn forget(st: &AppState) {
    *st.abs_server.notifications.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Stores the settings and makes them the ones in effect.
pub fn store(st: &AppState, s: &Settings) -> Result<()> {
    s.save(&st.abs)?;
    *st.abs_server.notifications.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    Ok(())
}

impl Manager {
    /// Starts sending an event, or queues it (`checkTriggerNotification`); true when it may be sent now.
    fn begin(&self, event: &str, data: &Map<String, Value>, max_queue: f64) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.0 {
            if s.1.len() as f64 >= max_queue {
                tracing::warn!("The notification queue is full, so the event {event} was dropped");
            } else {
                s.1.push_back((event.to_string(), data.clone()));
            }
            return false;
        }
        s.0 = true;
        true
    }
}

/// Sends an event's notifications (`triggerNotification`): each enabled one for the event, counting failures and
/// turning one off after too many. With `fail` every send counts as failed (the test button's `?fail=1`).
pub async fn trigger(st: &AppState, event: &str, data: Map<String, Value>, fail: bool) -> Result<()> {
    let mut settings = current(st)?;
    if !settings.is_useable() {
        return Ok(());
    }
    if !st.abs_server.notifications.begin(event, &data, settings.number("maxNotificationQueue", 20.0)) {
        return Ok(());
    }
    let url = settings.0.get("appriseApiUrl").map(js_text).unwrap_or_default();
    let max_failed = settings.number("maxFailedAttempts", 5.0);
    let delay = settings.number("notificationDelay", 1000.0).max(0.0) as u64;
    let mut list = settings.list().clone();
    for n in list.iter_mut().filter_map(Value::as_object_mut) {
        if n.get("eventName").and_then(Value::as_str) != Some(event) || !truthy(n.get("enabled")) {
            continue;
        }
        let ok = if fail { false } else { send(st, &url, n, &data).await };
        // `updateNotificationFired`
        n.insert("lastFiredAt".into(), json!(models::now_ms()));
        n.insert("lastAttemptFailed".into(), json!(!ok));
        let failed = if ok { 0.0 } else { js_number(n.get("numConsecutiveFailedAttempts")).unwrap_or(0.0) + 1.0 };
        n.insert("numConsecutiveFailedAttempts".into(), js_json(Some(failed)));
        let fired = js_number(n.get("numTimesFired")).unwrap_or(0.0) + 1.0;
        n.insert("numTimesFired".into(), js_json(Some(fired)));
        if !ok {
            if failed >= max_failed {
                tracing::error!("The notification {} failed too often and was turned off", n.get("id").map(js_text).unwrap_or_default());
                n.insert("enabled".into(), json!(false));
            } else {
                tracing::error!("The notification {} failed {failed} times in a row", n.get("id").map(js_text).unwrap_or_default());
            }
        }
    }
    *settings.list() = list;
    store(st, &settings)?;
    st.sockets.emitter("notifications_updated", &settings.to_json(), None);
    // `notificationFinished`: after the delay, the next waiting event
    let st2 = st.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        let next = {
            let mut s = st2.abs_server.notifications.state.lock().unwrap_or_else(|e| e.into_inner());
            s.0 = false;
            s.1.pop_front()
        };
        if let Some((event, data)) = next {
            if let Err(e) = trigger_queued(st2, event, data).await {
                tracing::error!("A queued notification couldn't be sent: {}", e.detail);
            }
        }
    });
    Ok(())
}

/// [`trigger`] for an event that waited in the queue, boxed so the queue can call it again.
fn trigger_queued(
    st: AppState,
    event: String,
    data: Map<String, Value>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
    Box::pin(async move { trigger(&st, &event, data, false).await })
}

/// The original's check for every notification route (`middleware`): admins only, and an id that exists.
fn admin_only(me: &models::User) -> Option<Response> {
    (!user::is_admin_or_up(me)).then(|| send_status(StatusCode::FORBIDDEN))
}

/// `GET /api/notifications`: the events data and the settings.
async fn get_all(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    match current(&st) {
        Ok(s) => Json(json!({ "data": data(), "settings": s.to_json() })).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/notificationdata`: the events notifications can be set up for.
async fn get_data(me: AbsUser) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    Json(data().clone()).into_response()
}

/// Runs a settings change and answers with the settings, stored when it changed something.
fn change(st: &AppState, f: impl FnOnce(&mut Settings) -> std::result::Result<bool, NoUrls>) -> Response {
    let mut s = match current(st) {
        Ok(s) => s,
        Err(e) => return e.into_response(),
    };
    match f(&mut s) {
        Ok(true) => {
            if let Err(e) = store(st, &s) {
                return e.into_response();
            }
        }
        Ok(false) => {}
        Err(NoUrls) => return send_status(StatusCode::INTERNAL_SERVER_ERROR),
    }
    Json(s.to_json()).into_response()
}

/// `PATCH /api/notifications`: the Apprise address and limits; answers 200.
async fn update(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let fields = body_fields(&headers, &body);
    let done = (|| -> Result<()> {
        let mut s = current(&st)?;
        if s.update(&fields) {
            store(&st, &s)?;
        }
        Ok(())
    })();
    match done {
        Ok(()) => send_status(StatusCode::OK),
        Err(e) => e.into_response(),
    }
}

/// `POST /api/notifications`: adds a notification; answers the settings.
async fn create(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let fields = body_fields(&headers, &body);
    change(&st, |s| s.create(&fields))
}

/// The notification an address names, after the admin check; `Err` is the answer to send.
fn named(st: &AppState, me: &models::User, id: &str) -> std::result::Result<(Settings, Map<String, Value>), Box<Response>> {
    if let Some(r) = admin_only(me) {
        return Err(Box::new(r));
    }
    let s = current(st).map_err(|e| Box::new(e.into_response()))?;
    let n = s.get(id).ok_or_else(|| Box::new(send_status(StatusCode::NOT_FOUND)))?;
    Ok((s, n))
}

/// `DELETE /api/notifications/:id`: removes a notification; answers the settings.
async fn remove(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    if let Err(r) = named(&st, &me.user, &id) {
        return *r;
    }
    change(&st, |s| Ok(s.remove(&id)))
}

/// `PATCH /api/notifications/:id`: changes the notification the body's `id` names; answers the settings.
async fn update_one(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = named(&st, &me.user, &id) {
        return *r;
    }
    let fields = body_fields(&headers, &body);
    change(&st, |s| Ok(s.update_notification(&fields)))
}

/// `GET /api/notifications/test`: fires the test event (`?fail=1` makes every send count as failed).
async fn fire_test(State(st): State<AppState>, me: AbsUser, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Some(r) = admin_only(&me.user) {
        return r;
    }
    let mut d = Map::new();
    d.insert("version".into(), json!(format!("v{}", crate::abs::ABS_VERSION)));
    match trigger(&st, "onTest", d, q.get("fail").map(String::as_str) == Some("1")).await {
        Ok(()) => send_status(StatusCode::OK),
        Err(e) => e.into_response(),
    }
}

/// `GET /api/notifications/:id/test`: sends one notification with its event's test values.
async fn send_test(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>) -> Response {
    let (s, n) = match named(&st, &me.user, &id) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    if !s.is_useable() {
        return send_text(StatusCode::BAD_REQUEST, "Apprise is not configured");
    }
    let event = n.get("eventName").and_then(Value::as_str).unwrap_or_default();
    let test_data =
        data()["events"].as_array().into_iter().flatten().find(|e| e["name"] == event).and_then(|e| e["testData"].as_object()).cloned();
    let Some(test_data) = test_data else {
        tracing::error!("There is no event called {event}");
        return send_status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let url = s.0.get("appriseApiUrl").map(js_text).unwrap_or_default();
    if send(&st, &url, &n, &test_data).await {
        send_status(StatusCode::OK)
    } else {
        send_status(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_filled_like_the_original() {
        let d = json!({"version": "v2.37.2", "numFailed": 3, "empty": ""});
        let d = d.as_object().unwrap();
        assert_eq!(fill_template("Test {{version}}", d), "Test v2.37.2");
        assert_eq!(fill_template("Body {{ version }} {{missing}}", d), "Body v2.37.2 {{missing}}");
        assert_eq!(fill_template("{{numFailed}} and {{numFailed}}", d), "3 and 3");
        assert_eq!(fill_template("{{empty}} {{ver_sion}} {{  version}}", d), "{{empty}} {{ver_sion}} {{  version}}");
    }

    #[test]
    fn the_events_are_the_originals() {
        let names: Vec<&str> = data()["events"].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect();
        assert_eq!(
            names,
            ["onPodcastEpisodeDownloaded", "onBackupCompleted", "onBackupFailed", "onRSSFeedFailed", "onRSSFeedDisabled", "onTest"]
        );
    }
}
