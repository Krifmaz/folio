//! Listening and reading without a connection.
//!
//! - **Downloads.** A book's files and what the player or reader needs to open it (the edition, its file list) are
//!   kept in this browser's Cache Storage (`folio-media`), so it opens and plays with no connection. The list of
//!   downloaded books is in `localStorage`.
//! - **Places.** A place the player or reader couldn't send waits in `localStorage` and goes to the server when the
//!   connection is back. Each place carries the time it was made, so the server keeps the newest one, wherever it
//!   came from.

use kit_leptos::{api, rt};
use leptos::prelude::*;
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

/// The Cache Storage cache the downloads are kept in.
const CACHE: &str = "folio-media";

/// The `localStorage` key of the downloaded books: `[{edition_id, title, format, files: [ids], bytes, at}]`.
const BOOKS_KEY: &str = "folio.offline.books";

/// The address a library file is fetched from.
pub fn file_url(id: i64) -> String {
    format!("/api/files/{id}")
}

/// The books downloaded to this browser.
pub fn downloaded() -> Vec<Value> {
    storage()
        .and_then(|s| s.get_item(BOOKS_KEY).ok().flatten())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// Whether the edition is downloaded to this browser.
pub fn is_downloaded(edition: i64) -> bool {
    downloaded().iter().any(|b| b.get("edition_id").and_then(Value::as_i64) == Some(edition))
}

/// Writes the list of downloaded books.
fn keep_downloaded(list: &[Value]) {
    if let Some(s) = storage() {
        let _ = s.set_item(BOOKS_KEY, &Value::Array(list.to_vec()).to_string());
    }
}

/// The downloads' cache, when the browser has Cache Storage (it needs HTTPS or localhost).
async fn cache() -> Option<web_sys::Cache> {
    let caches = rt::window()?.caches().ok()?;
    let c = wasm_bindgen_futures::JsFuture::from(caches.open(CACHE)).await.ok()?;
    c.dyn_into().ok()
}

/// The kept answer for `url`, if there is one.
async fn cached(url: &str) -> Option<web_sys::Response> {
    let c = cache().await?;
    let r = wasm_bindgen_futures::JsFuture::from(c.match_with_str(url)).await.ok()?;
    if r.is_undefined() || r.is_null() {
        return None;
    }
    r.dyn_into().ok()
}

/// An answer to keep: `body` with the type `mime`.
fn response(body: &wasm_bindgen::JsValue, mime: &str) -> Option<web_sys::Response> {
    let headers = web_sys::Headers::new().ok()?;
    headers.set("content-type", mime).ok()?;
    let init = web_sys::ResponseInit::new();
    init.set_headers(&headers);
    if let Some(b) = body.dyn_ref::<web_sys::Blob>() {
        web_sys::Response::new_with_opt_blob_and_init(Some(b), &init).ok()
    } else {
        web_sys::Response::new_with_opt_str_and_init(body.as_string().as_deref(), &init).ok()
    }
}

/// Keeps `resp` for `url`; plain words when the browser has no room.
async fn put(c: &web_sys::Cache, url: &str, resp: &web_sys::Response) -> Result<(), String> {
    wasm_bindgen_futures::JsFuture::from(c.put_with_str(url, resp))
        .await
        .map(|_| ())
        .map_err(|_| "This browser has no room left for the book. Remove another download, or free some space, then try again.".to_string())
}

/// `GET /api<path>` as JSON; when the server can't be reached, the copy kept with a downloaded book. A copy is
/// kept fresh whenever the server answers for a book that is downloaded.
pub async fn json(path: &str) -> Result<Value, String> {
    let url = format!("/api{path}");
    match api::get(path).await {
        Ok(v) => {
            if cached(&url).await.is_some() {
                if let (Some(c), Some(r)) = (cache().await, response(&v.to_string().into(), "application/json")) {
                    let _ = put(&c, &url, &r).await;
                }
            }
            Ok(v)
        }
        Err(e) if unreachable(&e) => {
            let Some(r) = cached(&url).await else { return Err(e.message) };
            let text =
                wasm_bindgen_futures::JsFuture::from(r.text().map_err(|_| e.message.clone())?).await.map_err(|_| e.message.clone())?;
            serde_json::from_str(&text.as_string().unwrap_or_default()).map_err(|_| e.message)
        }
        Err(e) => Err(e.message),
    }
}

/// A `blob:` address for the downloaded copy of file `id`, `None` when it isn't downloaded.
pub async fn local_url(id: i64) -> Option<String> {
    let r = cached(&file_url(id)).await?;
    let blob = wasm_bindgen_futures::JsFuture::from(r.blob().ok()?).await.ok()?;
    web_sys::Url::create_object_url_with_blob(blob.dyn_ref()?).ok()
}

/// The bytes of the library file `id`: the downloaded copy, else from the server; in plain words when they can't be
/// had.
pub async fn file_bytes(id: i64) -> Result<Vec<u8>, String> {
    let resp = match cached(&file_url(id)).await {
        Some(r) => r,
        None => fetch_file(id).await?,
    };
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().map_err(|_| "The book can't be read.")?)
        .await
        .map_err(|_| "The book stopped coming in part way. Try again.")?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Asks the server for file `id`; its answer when it is the file, the server's words otherwise.
async fn fetch_file(id: i64) -> Result<web_sys::Response, String> {
    let w = rt::window().ok_or("This page has no window.")?;
    let resp = wasm_bindgen_futures::JsFuture::from(w.fetch_with_str(&file_url(id)))
        .await
        .map_err(|_| "The book couldn't be fetched: the server can't be reached. Check the connection, then try again.".to_string())?;
    let resp: web_sys::Response = resp.dyn_into().map_err(|_| "The server's answer can't be read.")?;
    if resp.ok() {
        return Ok(resp);
    }
    let text = match resp.text() {
        Ok(p) => wasm_bindgen_futures::JsFuture::from(p).await.ok().and_then(|t| t.as_string()).unwrap_or_default(),
        Err(_) => String::new(),
    };
    let detail = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("detail").and_then(Value::as_str).map(str::to_string));
    Err(detail.unwrap_or_else(|| format!("The book couldn't be fetched (the server answered {}). Try again.", resp.status())))
}

/// Reads all of `resp`'s body, telling `got` how many bytes came so far.
async fn read_all(resp: web_sys::Response, got: &dyn Fn(f64)) -> Result<web_sys::Blob, String> {
    let lost = "The download stopped part way. Check the connection, then try again.";
    let mime = resp.headers().get("content-type").ok().flatten().unwrap_or_default();
    let body = resp.body().ok_or(lost)?;
    let reader: web_sys::ReadableStreamDefaultReader = body.get_reader().unchecked_into();
    let parts = js_sys::Array::new();
    let mut n = 0.0;
    loop {
        let step = wasm_bindgen_futures::JsFuture::from(reader.read()).await.map_err(|_| lost)?;
        if js_sys::Reflect::get(&step, &"done".into()).ok().and_then(|d| d.as_bool()).unwrap_or(true) {
            break;
        }
        let chunk = js_sys::Reflect::get(&step, &"value".into()).map_err(|_| lost)?;
        n += js_sys::Uint8Array::new(&chunk).length() as f64;
        parts.push(&chunk);
        got(n);
    }
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(&mime);
    web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(|_| lost.to_string())
}

/// The files of a book worth keeping for reading or listening offline: audio, EPUB and PDF.
pub fn files_to_keep(files: &[Value]) -> Vec<Value> {
    files.iter().filter(|f| matches!(f.get("kind").and_then(Value::as_str), Some("audio" | "epub" | "pdf"))).cloned().collect()
}

/// Downloads the edition to this browser: its files and what opening it needs. `progress` hears how much is in,
/// 0–1. Plain words when it can't.
pub async fn download(edition: &Value, files: &[Value], progress: impl Fn(f64)) -> Result<(), String> {
    let id = edition.get("id").and_then(Value::as_i64).unwrap_or(0);
    let c = cache().await.ok_or("This browser can't keep books for offline use here. It needs the app at an https:// address.")?;
    let keep = files_to_keep(files);
    let total: f64 = keep.iter().filter_map(|f| f.get("size").and_then(Value::as_f64)).sum::<f64>().max(1.0);
    // ask the browser not to clear the downloads when space runs low (it may say no; they are kept anyway)
    if let Some(w) = rt::window() {
        let _ = w.navigator().storage().persist();
    }
    let mut done = 0.0;
    for f in &keep {
        let fid = f.get("id").and_then(Value::as_i64).unwrap_or(0);
        let resp = fetch_file(fid).await?;
        let before = done;
        let blob = read_all(resp, &|n| progress(((before + n) / total).min(1.0))).await?;
        done += blob.size();
        let mime = f.get("mime").and_then(Value::as_str).unwrap_or("application/octet-stream");
        let r = response(&blob.into(), mime).ok_or("The book can't be kept.")?;
        put(&c, &file_url(fid), &r).await?;
    }
    let marks = api::get(&format!("/editions/{id}/bookmarks")).await.unwrap_or_else(|_| json!({ "bookmarks": [] }));
    for (path, v) in [
        (format!("/api/library/editions/{id}"), edition.clone()),
        (format!("/api/library/editions/{id}/files"), json!({ "files": files })),
        (format!("/api/editions/{id}/bookmarks"), marks),
        (format!("/api/progress/{id}"), api::get(&format!("/progress/{id}")).await.unwrap_or(Value::Null)),
    ] {
        if v.is_null() {
            continue;
        }
        let r = response(&v.to_string().into(), "application/json").ok_or("The book can't be kept.")?;
        put(&c, &path, &r).await?;
    }
    let mut list = downloaded();
    list.retain(|b| b.get("edition_id").and_then(Value::as_i64) != Some(id));
    list.push(json!({
        "edition_id": id,
        "title": edition.get("title").cloned().unwrap_or(Value::Null),
        "format": edition.get("format").cloned().unwrap_or(Value::Null),
        "files": keep.iter().filter_map(|f| f.get("id").and_then(Value::as_i64)).collect::<Vec<_>>(),
        "bytes": done,
        "at": now_s(),
    }));
    keep_downloaded(&list);
    progress(1.0);
    Ok(())
}

/// Removes the edition's download from this browser.
pub async fn remove(edition: i64) {
    let list = downloaded();
    if let Some(c) = cache().await {
        let files = list
            .iter()
            .find(|b| b.get("edition_id").and_then(Value::as_i64) == Some(edition))
            .and_then(|b| b.get("files").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        let mut urls: Vec<String> = files.iter().filter_map(Value::as_i64).map(file_url).collect();
        urls.push(format!("/api/library/editions/{edition}"));
        urls.push(format!("/api/library/editions/{edition}/files"));
        urls.push(format!("/api/editions/{edition}/bookmarks"));
        urls.push(format!("/api/progress/{edition}"));
        for u in urls {
            let _ = wasm_bindgen_futures::JsFuture::from(c.delete_with_str(&u)).await;
        }
    }
    keep_downloaded(&list.into_iter().filter(|b| b.get("edition_id").and_then(Value::as_i64) != Some(edition)).collect::<Vec<_>>());
}

/// A size in bytes as people say it: `"312 MB"`, `"1.4 GB"`.
pub fn size_words(bytes: f64) -> String {
    let mb = bytes / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 1.0 {
        format!("{} MB", mb.round() as i64)
    } else {
        format!("{} KB", (bytes / 1000.0).round().max(1.0) as i64)
    }
}

/// The Download button of a book: downloads it for offline use with how far it is, or says it is downloaded and
/// offers Remove (after asking).
#[component]
pub fn DownloadButton(edition: Value, files: Vec<Value>) -> impl IntoView {
    use kit_leptos::toasts::toast;
    let id = edition.get("id").and_then(Value::as_i64).unwrap_or(0);
    let size: f64 = files_to_keep(&files).iter().filter_map(|f| f.get("size").and_then(Value::as_f64)).sum();
    let (state, set_state) = signal(if is_downloaded(id) { -1.0 } else { -2.0 });
    let edition = StoredValue::new(edition);
    let files = StoredValue::new(files);
    let start = move |_| {
        set_state.set(0.0);
        rt::spawn(async move {
            let (e, f) = (edition.get_value(), files.get_value());
            match download(&e, &f, move |p| set_state.set(p)).await {
                Ok(()) => {
                    set_state.set(-1.0);
                    toast("Downloaded. This book now opens and plays without a connection.", "ok");
                }
                Err(err) => {
                    set_state.set(-2.0);
                    remove(id).await;
                    toast(&err, "err");
                }
            }
        });
    };
    let drop_it = move |_| {
        rt::spawn(async move {
            let q = kit_rules::dialogs::Confirm::new("Remove the download?")
                .text("This browser forgets its copy of the book. It still plays from the server when you're online.")
                .ok("Remove")
                .danger(true);
            if kit_leptos::modal::confirm(q).await {
                remove(id).await;
                set_state.set(-2.0);
                toast("The download was removed.", "ok");
            }
        });
    };
    move || {
        let s = state.get();
        if s == -1.0 {
            view! { <button class="btn sm" id="download-btn" title="Remove the copy kept in this browser" on:click=drop_it>"Downloaded · Remove"</button> }.into_any()
        } else if s >= 0.0 {
            view! { <button class="btn sm" id="download-btn" disabled=true>{format!("Downloading {}%", (s * 100.0).floor() as i64)}</button> }.into_any()
        } else {
            view! { <button class="btn sm" id="download-btn" title="Keep this book in this browser to play or read it offline" on:click=start>{format!("Download ({})", size_words(size))}</button> }.into_any()
        }
    }
}

thread_local! {
    /// Whether [`watch`] has started.
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    /// The latest place of each open book, kept outside the page's reactive state so it can still be sent while the
    /// page is being taken down.
    static LATEST: std::cell::RefCell<std::collections::HashMap<i64, Value>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Keeps `place` as the latest place in `edition` (sent by [`send_latest`] when the book is closed).
pub fn remember(edition: i64, place: &Value) {
    LATEST.with(|l| l.borrow_mut().insert(edition, place.clone()));
}

/// Sends the latest place in `edition`, if one was kept (the book is being closed).
pub fn send_latest(edition: i64) {
    if let Some(place) = LATEST.with(|l| l.borrow_mut().remove(&edition)) {
        rt::spawn(async move {
            let _ = save_progress(edition, place).await;
        });
    }
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
    fn sizes_read_plainly_and_only_book_files_are_kept() {
        assert_eq!(size_words(312_400_000.0), "312 MB");
        assert_eq!(size_words(1_420_000_000.0), "1.4 GB");
        assert_eq!(size_words(4_000.0), "4 KB");
        let files = vec![json!({ "id": 1, "kind": "audio" }), json!({ "id": 2, "kind": "cover" }), json!({ "id": 3, "kind": "epub" })];
        assert_eq!(files_to_keep(&files).len(), 2);
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
