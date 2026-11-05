//! Keeping places when the server can't be reached: a place the player or reader couldn't send waits in this
//! browser (`localStorage`) and goes to the server when the connection is back. Each place carries the time it was
//! made, so the server keeps the newest one, wherever it came from.

use kit_leptos::{api, rt};
use serde_json::{json, Map, Value};
use std::cell::Cell;
use wasm_bindgen::JsCast;

/// What this app calls itself in a place it saves (`device`): the web app.
pub const DEVICE: &str = "web";

/// The `localStorage` key of the places waiting to be sent: `{"<edition id>": {place}}`.
const PENDING_KEY: &str = "folio.progress.pending";

/// Now in Unix seconds, the time a place carries.
pub fn now_s() -> f64 {
    rt::now_ms() as f64 / 1000.0
}

/// `localStorage`, where the browser allows it.
fn storage() -> Option<web_sys::Storage> {
    rt::window()?.local_storage().ok().flatten()
}

/// The places waiting to be sent, by edition id.
fn pending_all() -> Map<String, Value> {
    storage()
        .and_then(|s| s.get_item(PENDING_KEY).ok().flatten())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Writes the waiting places back.
fn keep_pending(all: &Map<String, Value>) {
    if let Some(s) = storage() {
        let _ = if all.is_empty() { s.remove_item(PENDING_KEY) } else { s.set_item(PENDING_KEY, &Value::Object(all.clone()).to_string()) };
    }
}

/// The place waiting to be sent for `edition`, if there is one.
pub fn pending(edition: i64) -> Option<Value> {
    pending_all().get(&edition.to_string()).cloned()
}

/// Puts `place` in the waiting list for `edition` (a newer one replaces an older one).
pub fn wait(edition: i64, place: &Value) {
    let mut all = pending_all();
    let key = edition.to_string();
    if newer(place, all.get(&key)) {
        all.insert(key, place.clone());
        keep_pending(&all);
    }
}

/// Whether `a` was made after `b` (by `updated_at`; anything beats nothing).
pub fn newer(a: &Value, b: Option<&Value>) -> bool {
    b.is_none_or(|b| made_at(a) >= made_at(b))
}

/// Whether a failed call means the server wasn't reached (so trying later may work), not that it said no.
pub fn unreachable(e: &api::ApiError) -> bool {
    e.status == 0 || (502..=504).contains(&e.status)
}

/// Sends the place in `edition` to the server; the place that stands there (this one, or a newer one from another
/// device). When the server can't be reached the place waits here and goes later; `None` then.
pub async fn save_progress(edition: i64, place: Value) -> Option<Value> {
    match api::put(&format!("/progress/{edition}"), place.clone()).await {
        Ok(v) => {
            // sent: an older waiting place for this edition is not needed any more
            let mut all = pending_all();
            if all.get(&edition.to_string()).is_some_and(|p| newer(&place, Some(p))) {
                all.remove(&edition.to_string());
                keep_pending(&all);
            }
            Some(v)
        }
        Err(e) if unreachable(&e) => {
            wait(edition, &place);
            None
        }
        Err(_) => None,
    }
}

/// When a place was made (`updated_at`, Unix seconds; 0 when it doesn't say).
fn made_at(v: &Value) -> f64 {
    v.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0)
}

/// Sends every waiting place; those the server still can't take keep waiting.
pub async fn flush() {
    let all = pending_all();
    let mut done = Vec::new();
    for (k, place) in all {
        let Ok(edition) = k.parse::<i64>() else { continue };
        match api::put(&format!("/progress/{edition}"), place.clone()).await {
            Err(e) if unreachable(&e) => {}
            // taken, or refused for good (the book is gone): either way it stops waiting
            _ => done.push((k, place)),
        }
    }
    if done.is_empty() {
        return;
    }
    // a place saved while this ran is newer than the copy sent, and keeps waiting
    let mut now = pending_all();
    for (k, place) in done {
        if now.get(&k).is_some_and(|cur| made_at(cur) <= made_at(&place)) {
            now.remove(&k);
        }
    }
    keep_pending(&now);
}

/// The bytes of the library file `id` (`/api/files/{id}`), in plain words when they can't be had.
pub async fn file_bytes(id: i64) -> Result<Vec<u8>, String> {
    let w = rt::window().ok_or("This page has no window.")?;
    let resp = wasm_bindgen_futures::JsFuture::from(w.fetch_with_str(&format!("/api/files/{id}")))
        .await
        .map_err(|_| "The book couldn't be fetched: the server can't be reached. Check the connection, then open it again.".to_string())?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "The server's answer can't be read.")?;
    if !resp.ok() {
        let text = match resp.text() {
            Ok(p) => wasm_bindgen_futures::JsFuture::from(p).await.ok().and_then(|t| t.as_string()).unwrap_or_default(),
            Err(_) => String::new(),
        };
        let detail = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("detail").and_then(Value::as_str).map(str::to_string));
        return Err(detail.unwrap_or_else(|| format!("The book couldn't be fetched (the server answered {}). Try again.", resp.status())));
    }
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(|_| "The book can't be read.")?)
        .await
        .map_err(|_| "The book stopped coming in part way. Try again.")?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

thread_local! {
    /// Whether [`watch`] has started.
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Sends the waiting places now and whenever the browser is back online (once per page; later calls only send).
pub fn watch() {
    rt::spawn(flush());
    if WATCHING.with(|w| w.replace(true)) {
        return;
    }
    rt::on_window("online", |_| rt::spawn(flush()));
}

/// A place to send for an audiobook at `position_ms` of a book `total_ms` long.
pub fn audio_place(position_ms: i64, total_ms: i64) -> Value {
    let fraction = if total_ms > 0 { (position_ms as f64 / total_ms as f64).clamp(0.0, 1.0) } else { 0.0 };
    json!({
        "position_ms": position_ms.max(0),
        "fraction": fraction,
        "finished": total_ms > 0 && position_ms >= total_ms - 1000,
        "device": DEVICE,
        "updated_at": now_s(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newer_place_wins() {
        let a = json!({ "updated_at": 2.0 });
        let b = json!({ "updated_at": 1.0 });
        assert!(newer(&a, Some(&b)));
        assert!(!newer(&b, Some(&a)));
        assert!(newer(&b, None));
    }

    #[test]
    fn audio_places_say_how_far() {
        let p = audio_place(30_000, 60_000);
        assert_eq!((p["position_ms"].as_i64(), p["fraction"].as_f64(), p["finished"].as_bool()), (Some(30_000), Some(0.5), Some(false)));
        assert_eq!(audio_place(59_500, 60_000)["finished"], true);
        assert_eq!(audio_place(-5, 0)["fraction"], 0.0);
    }

    #[test]
    fn only_no_answer_waits() {
        let e = |status| api::ApiError { status, message: String::new(), data: Value::Null };
        assert!(unreachable(&e(0)));
        assert!(unreachable(&e(502)));
        assert!(!unreachable(&e(404)));
        assert!(!unreachable(&e(400)));
    }
}
