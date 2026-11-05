//! The port of the original's playback session manager (`server/managers/PlaybackSessionManager.js`): starting a
//! session when a player opens a book, syncing where the listener is and how long they listened, closing it, and
//! taking in the sessions a phone played offline. Each sync also updates the listener's progress, like the
//! original.
//!
//! Open sessions live in memory, one list per server (per data folder). The original closes sessions that have
//! not changed for 36 hours once a day (`closeStaleOpenSessions`); here they are dropped whenever the list is
//! next used, which no client can tell apart.

pub mod device;
pub mod session;

use crate::abs::models::{self, book::BookFull, media_progress, user, Library, LibraryItem, MediaProgress, User};
use crate::AppState;
use device::DeviceInfo;
use kit_server_core::Result;
use serde_json::{json, Map, Value};
use session::Session;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// How long an open session may go without a change before it is dropped: 36 hours.
const STALE_MS: i64 = 36 * 60 * 60 * 1000;

/// The open sessions of each server, by its data folder.
static OPEN: LazyLock<Mutex<HashMap<PathBuf, Vec<Session>>>> = LazyLock::new(Default::default);

/// The ids given to old offline sessions (`play_local_…`) of each server (`oldPlaybackSessionMap`).
static OLD_IDS: LazyLock<Mutex<HashMap<PathBuf, HashMap<String, String>>>> = LazyLock::new(Default::default);

/// Runs `f` on this server's open sessions, after dropping the stale ones.
pub fn with_open<R>(st: &AppState, f: impl FnOnce(&mut Vec<Session>) -> R) -> R {
    let mut all = OPEN.lock().unwrap_or_else(|p| p.into_inner());
    let list = all.entry(st.abs_server.config_path.clone()).or_default();
    let cutoff = models::now_ms() - STALE_MS;
    list.retain(|s| session::js_ms(Some(&s.updated_at)).is_none_or(|u| u >= cutoff));
    f(list)
}

/// A copy of an open session.
pub fn open_session(st: &AppState, id: &str) -> Option<Session> {
    with_open(st, |list| list.iter().find(|s| s.id == id).cloned())
}

/// Puts a changed open session back (when it is still open).
fn put_back(st: &AppState, s: &Session) {
    with_open(st, |list| {
        if let Some(slot) = list.iter_mut().find(|o| o.id == s.id) {
            *slot = s.clone();
        }
    });
}

/// Drops an open session (`removeSession`).
pub fn remove_open(st: &AppState, id: &str) {
    with_open(st, |list| list.retain(|s| s.id != id));
}

/// The user as other users see them, with their open session (`toJSONForPublic(sessions)`).
fn public_user(st: &AppState, u: &User) -> Value {
    let id = u.id.clone().unwrap_or_default();
    let session = with_open(st, |list| list.iter().find(|s| s.user_id.as_deref() == Some(id.as_str())).map(|s| s.to_json_for_client(None)));
    user::to_json_for_public(u, session)
}

/// Tells the admins how this user's listening changed (`user_stream_update`).
fn tell_stream_update(st: &AppState, u: &User) {
    st.sockets.admin_emitter("user_stream_update", &public_user(st, u));
}

/// The settings of the item's library, for when a listener counts as finished (`librarySettings`).
fn library_settings(st: &AppState, item: &LibraryItem) -> Result<Option<Value>> {
    let Some(lib) = models::find::<Library>(&st.abs, item.library_id.as_deref().unwrap_or_default())? else { return Ok(None) };
    Ok(Some(lib.settings.unwrap_or(Value::Null)))
}

/// What a player asked for when it opened a book (`options` of `startSession`).
#[derive(Debug, Default)]
pub struct StartOptions {
    /// Play the files as they are, whatever the player supports.
    pub force_direct_play: bool,
    /// Convert the audio even when the player could play the files.
    pub force_transcode: bool,
    /// The player's name; `unknown` when not given.
    pub media_player: Option<String>,
    /// The audio types the player can play, when it says.
    pub supported_mime_types: Option<Vec<String>>,
}

impl StartOptions {
    /// The options from a request body.
    pub fn from_body(b: &Map<String, Value>) -> StartOptions {
        let truthy = |k: &str| crate::abs::settings::truthy(b.get(k));
        StartOptions {
            force_direct_play: truthy("forceDirectPlay"),
            force_transcode: truthy("forceTranscode"),
            media_player: b.get("mediaPlayer").filter(|v| crate::abs::settings::truthy(Some(v))).map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
            supported_mime_types: b
                .get("supportedMimeTypes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()),
        }
    }
}

/// Whether a player that plays these audio types can play the book's files as they are (`checkCanDirectPlay`).
fn can_direct_play(b: &BookFull, types: &Option<Vec<String>>) -> bool {
    let Some(types) = types else { return false };
    b.included_audio_files().iter().all(|af| af.get("mimeType").and_then(Value::as_str).is_some_and(|m| types.iter().any(|t| t == m)))
}

/// The user's progress for a book or episode (`getMediaProgress`).
fn progress_of(st: &AppState, user_id: &str, media_item_id: &str) -> Result<Option<MediaProgress>> {
    Ok(user::media_progresses(&st.abs, user_id)?.into_iter().find(|mp| mp.media_item_id.as_deref() == Some(media_item_id)))
}

/// Closes a session (`closeSession`): syncs it one last time when the player sent where it stopped, else stores
/// it as it is; then tells the admins and the user's apps, and drops it.
pub fn close(st: &AppState, u: &User, s: &mut Session, sync_data: Option<&Map<String, Value>>) -> Result<()> {
    match sync_data {
        Some(data) => {
            sync(st, u, s, data)?;
        }
        None => save(st, s)?,
    }
    put_back(st, s);
    tell_stream_update(st, u);
    st.sockets.client_emitter(s.user_id.as_deref().unwrap_or_default(), "user_session_closed", &json!(s.id));
    remove_open(st, &s.id);
    Ok(())
}

/// Starts a session for a book (`startSession`): closes the user's open sessions on the same device, starts
/// where the user left off (from the start again when they had finished), and lists the tracks to play.
pub fn start(st: &AppState, u: &User, device: DeviceInfo, item: &LibraryItem, b: &BookFull, options: &StartOptions) -> Result<Session> {
    let user_id = u.id.clone().unwrap_or_default();
    let same_device: Vec<Session> = with_open(st, |list| {
        list.iter().filter(|s| s.user_id.as_deref() == Some(user_id.as_str()) && s.device_id() == device.id).cloned().collect()
    });
    for mut s in same_device {
        tracing::info!(
            "Closing the open session \"{}\" of {} on {}",
            s.id,
            u.username.as_deref().unwrap_or_default(),
            s.device_description()
        );
        close(st, u, &mut s, None)?;
    }
    let direct = options.force_direct_play || (!options.force_transcode && can_direct_play(b, &options.supported_mime_types));
    if !direct {
        // Converting audio on the fly (the original's HLS stream) is the next step of the port; until then the
        // player gets the files as they are.
        tracing::warn!("The player can't play this book's files as they are; it gets them as they are for now");
    }
    let book_id = b.book.id.clone().unwrap_or_default();
    let start_time = match progress_of(st, &user_id, &book_id)? {
        Some(mp) if mp.is_finished.unwrap_or(false) => 0.0,
        Some(mp) => mp.current_time.filter(|t| t.is_finite()).unwrap_or(0.0),
        None => 0.0,
    };
    let now = models::now_ms();
    let (day, weekday) = session::day_of(now);
    let item_id = item.id.clone().unwrap_or_default();
    let s = Session {
        id: models::new_id(),
        user_id: Some(user_id),
        library_id: item.library_id.clone(),
        library_item_id: Some(item_id.clone()),
        book_id: Some(book_id),
        episode_id: None,
        media_type: json!(item.media_type),
        media_metadata: Value::Object(b.metadata_json()),
        chapters: match &b.book.chapters {
            Some(c @ Value::Array(_)) => c.clone(),
            _ => json!([]),
        },
        display_title: json!(b.book.title),
        display_author: json!(b.author_name()),
        cover_path: json!(b.book.cover_path),
        duration: json!(b.book.duration),
        play_method: json!(session::DIRECT_PLAY),
        media_player: json!(options.media_player.clone().unwrap_or_else(|| "unknown".into())),
        device_info: Some(device),
        server_version: json!(crate::abs::ABS_VERSION),
        date: json!(day),
        day_of_week: json!(weekday),
        time_listening: json!(0),
        start_time: json!(start_time),
        current_time: json!(start_time),
        started_at: json!(now),
        updated_at: json!(now),
        last_save: 0,
        audio_tracks: b.tracks(&item_id).as_array().cloned().unwrap_or_default(),
    };
    with_open(st, |list| list.push(s.clone()));
    tell_stream_update(st, u);
    Ok(s)
}

/// Stores a session that has listening time (`saveSession`): the first time as new, then as an update.
pub fn save(st: &AppState, s: &mut Session) -> Result<()> {
    if !crate::abs::settings::truthy(Some(&s.time_listening)) {
        return Ok(());
    }
    if s.last_save != 0 {
        session::update(&st.abs, s)
    } else {
        s.last_save = models::now_ms();
        session::create(&st.abs, s)
    }
}

/// Takes a player's sync (`syncSession`): where the listener is and how long they listened since the last sync.
/// Updates their progress, tells their other apps and stores the session. `false` when the item or its library
/// is gone.
pub fn sync(st: &AppState, u: &User, s: &mut Session, data: &Map<String, Value>) -> Result<bool> {
    let item_id = s.library_item_id.clone().unwrap_or_default();
    let Some((item, _)) = crate::abs::items::expanded_by_id(&st.abs, &item_id)? else {
        tracing::error!("Syncing a session: the library item {item_id} is gone");
        return Ok(false);
    };
    let Some(settings) = library_settings(st, &item)? else {
        tracing::error!("Syncing a session: the library of item {item_id} is gone");
        return Ok(false);
    };
    s.current_time = data.get("currentTime").cloned().unwrap_or(Value::Null);
    s.add_listening_time(data.get("timeListened"));
    let duration = [data.get("duration"), Some(&s.duration)]
        .into_iter()
        .flatten()
        .find(|v| crate::abs::settings::truthy(Some(v)))
        .cloned()
        .unwrap_or(json!(0));
    let mut payload = Map::new();
    payload.insert("libraryItemId".into(), json!(item_id));
    payload.insert("episodeId".into(), s.episode_id.clone().map_or(Value::Null, Value::from));
    payload.insert("duration".into(), duration);
    payload.insert("currentTime".into(), data.get("currentTime").cloned().unwrap_or(Value::Null));
    payload.insert("progress".into(), json!(s.progress()));
    payload.insert("markAsFinishedTimeRemaining".into(), settings.get("markAsFinishedTimeRemaining").cloned().unwrap_or(Value::Null));
    payload.insert("markAsFinishedPercentComplete".into(), settings.get("markAsFinishedPercentComplete").cloned().unwrap_or(Value::Null));
    if let Ok(mp) = crate::abs::progress::create_update(&st.abs, u, &payload)? {
        tell_progress(st, u, &mp, s);
    }
    save(st, s)?;
    put_back(st, s);
    Ok(true)
}

/// Tells the user's apps their progress changed during a session (`user_item_progress_updated`).
fn tell_progress(st: &AppState, u: &User, mp: &MediaProgress, s: &Session) {
    st.sockets.client_emitter(
        u.id.as_deref().unwrap_or_default(),
        "user_item_progress_updated",
        &json!({
            "id": mp.id,
            "sessionId": s.id,
            "deviceDescription": s.device_description(),
            "data": media_progress::old_json(mp),
        }),
    );
}

/// Takes in a session a phone played offline (`syncLocalSession`): stores it (or its newer times), and moves
/// the listener's progress along unless it changed more recently elsewhere. Answers the original's result object.
pub fn sync_local(st: &AppState, u: &User, mut json: Map<String, Value>, device: &DeviceInfo) -> Result<Value> {
    let id_of = |j: &Map<String, Value>| j.get("id").cloned().unwrap_or(Value::Null);
    let title = json.get("displayTitle").cloned().unwrap_or(Value::Null);
    let item_id = json.get("libraryItemId").and_then(Value::as_str).unwrap_or_default().to_string();
    let found = if item_id.is_empty() { None } else { crate::abs::items::expanded_by_id(&st.abs, &item_id)? };
    let Some((item, b)) = found else {
        tracing::error!("Syncing a local session \"{title}\": the book is not here");
        return Ok(json!({"id": id_of(&json), "success": false, "error": "Media item not found"}));
    };
    let Some(settings) = library_settings(st, &item)? else {
        return Ok(json!({"id": id_of(&json), "success": false, "error": "Library not found"}));
    };
    json.insert("userId".into(), json!(u.id));
    json.insert("serverVersion".into(), json!(crate::abs::ABS_VERSION));
    if let Some(old) = json.get("id").and_then(Value::as_str).filter(|i| i.starts_with("play_local_")).map(String::from) {
        let mut all = OLD_IDS.lock().unwrap_or_else(|p| p.into_inner());
        let map = all.entry(st.abs_server.config_path.clone()).or_default();
        let new = map.entry(old).or_insert_with(models::new_id).clone();
        json.insert("id".into(), json!(new));
    }
    let book_id = b.book.id.clone().unwrap_or_default();
    if !crate::abs::settings::truthy(json.get("bookId")) {
        json.insert("bookId".into(), json!(book_id));
    }
    if json.get("libraryId") != Some(&json!(item.library_id)) {
        json.insert("libraryId".into(), json!(item.library_id));
    }
    let id = json.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut s = match session::by_id(&st.abs, &id)? {
        None => {
            let mut s = Session::from_json(&json);
            s.device_info = Some(device.clone());
            let mut meta = match s.media_metadata.take() {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            for (k, v) in b.metadata_json() {
                meta.entry(k).or_insert(v);
            }
            s.media_metadata = Value::Object(meta);
            if s.display_title.as_str().is_none_or(str::is_empty) {
                s.display_title = json!(b.book.title);
            }
            if s.display_author.as_str().is_none_or(str::is_empty) {
                s.display_author = json!(b.author_name());
            }
            s.duration = json!(b.book.duration);
            session::create(&st.abs, &s)?;
            s
        }
        Some(mut s) => {
            s.current_time = json.get("currentTime").cloned().unwrap_or(Value::Null);
            s.time_listening = json.get("timeListening").cloned().unwrap_or(Value::Null);
            s.updated_at = json.get("updatedAt").cloned().unwrap_or(Value::Null);
            let ms = session::js_ms(json.get("updatedAt")).unwrap_or_else(models::now_ms);
            let (day, weekday) = session::day_of(ms);
            s.date = json!(day);
            s.day_of_week = json!(weekday);
            session::update(&st.abs, &s)?;
            s
        }
    };
    let media_item_id = s.episode_id.clone().unwrap_or_else(|| book_id.clone());
    let current = progress_of(st, u.id.as_deref().unwrap_or_default(), &media_item_id)?;
    let newer_here = current.as_ref().is_some_and(|mp| {
        let changed = mp.updated_at.as_deref().and_then(models::date_ms).unwrap_or(0);
        session::js_ms(Some(&s.updated_at)).is_some_and(|at| changed > at)
    });
    let mut synced = false;
    if !newer_here {
        let mut payload = Map::new();
        payload.insert("libraryItemId".into(), json!(item.id));
        payload.insert("episodeId".into(), s.episode_id.clone().map_or(Value::Null, Value::from));
        payload.insert("duration".into(), s.duration.clone());
        payload.insert("currentTime".into(), s.current_time.clone());
        payload.insert("progress".into(), json!(s.progress()));
        payload.insert("lastUpdate".into(), s.updated_at.clone());
        payload
            .insert("markAsFinishedPercentComplete".into(), settings.get("markAsFinishedPercentComplete").cloned().unwrap_or(Value::Null));
        payload.insert("markAsFinishedTimeRemaining".into(), settings.get("markAsFinishedTimeRemaining").cloned().unwrap_or(Value::Null));
        if let Ok(mp) = crate::abs::progress::create_update(&st.abs, u, &payload)? {
            synced = true;
            tell_progress(st, u, &mp, &s);
        }
    }
    s.last_save = 0;
    Ok(json!({"id": s.id, "success": true, "progressSynced": synced}))
}
