//! One playback session (`server/objects/PlaybackSession.js`, stored as `server/models/PlaybackSession.js`): a
//! stretch of listening to one book or episode on one device, with where it started, where it is now and how
//! long was listened. Open sessions live in memory; a session is stored once it has listening time.

use super::device::{self, DeviceInfo};
use crate::abs::library::util::js_numbers;
use crate::abs::models::{self, date_ms, date_text, media_progress::js_number};
use kit_server_core::{Db, Result, Row};
use serde_json::{json, Map, Value};

/// How a session plays its audio (`PlayMethod`): the files as they are.
pub const DIRECT_PLAY: i64 = 0;
/// How a session plays its audio: converted on the fly into a stream.
pub const TRANSCODE: i64 = 2;

/// A playback session as the original's `PlaybackSession` object holds it. Values the original copies from the
/// client unchecked stay JSON values, so they read back exactly as they were sent.
#[derive(Debug, Clone, Default)]
pub struct Session {
    /// The session's id (a UUID).
    pub id: String,
    /// The listener's user id.
    pub user_id: Option<String>,
    /// The library the item is in.
    pub library_id: Option<String>,
    /// The library item played.
    pub library_item_id: Option<String>,
    /// The book played (for a book).
    pub book_id: Option<String>,
    /// The episode played (for a podcast).
    pub episode_id: Option<String>,
    /// `book` or `podcast`.
    pub media_type: Value,
    /// The book's or podcast's metadata when the session started.
    pub media_metadata: Value,
    /// The chapters played.
    pub chapters: Value,
    /// The title shown in the player.
    pub display_title: Value,
    /// The author shown in the player.
    pub display_author: Value,
    /// The cover's path.
    pub cover_path: Value,
    /// The length played, in seconds.
    pub duration: Value,
    /// [`DIRECT_PLAY`] or [`TRANSCODE`] (or the phone app's own values for local sessions).
    pub play_method: Value,
    /// The player, such as `html5` or `exo-player`.
    pub media_player: Value,
    /// The device listened on.
    pub device_info: Option<DeviceInfo>,
    /// The server version that made the session.
    pub server_version: Value,
    /// The day of the listening, `YYYY-MM-DD`.
    pub date: Value,
    /// The weekday of the listening, such as `Thursday`.
    pub day_of_week: Value,
    /// The seconds listened.
    pub time_listening: Value,
    /// Where in the book the session started, in seconds.
    pub start_time: Value,
    /// Where in the book the listener is now, in seconds.
    pub current_time: Value,
    /// When the session started, in milliseconds.
    pub started_at: Value,
    /// When the session last changed, in milliseconds.
    pub updated_at: Value,
    /// When the session was first stored (0 before that); not stored itself.
    pub last_save: i64,
    /// The tracks the player plays; not stored.
    pub audio_tracks: Vec<Value>,
}

/// JavaScript's `v || fallback` for a JSON value.
fn or(v: Option<&Value>, fallback: Value) -> Value {
    match v {
        Some(v) if crate::abs::settings::truthy(Some(v)) => v.clone(),
        _ => fallback,
    }
}

/// A text field of a JSON object, if it is text.
fn text(m: &Map<String, Value>, k: &str) -> Option<String> {
    m.get(k).and_then(Value::as_str).map(String::from)
}

/// The day and weekday of a time in milliseconds, as the original formats them (`YYYY-MM-DD` and `dddd`, in UTC
/// like the original's image).
pub fn day_of(ms: i64) -> (String, String) {
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    let day = date_text(ms)[..10].to_string();
    (day, DAYS[ms.div_euclid(86_400_000).rem_euclid(7) as usize].to_string())
}

impl Session {
    /// A session from JSON (`construct`): what a phone sends for a session it played offline, or a stored row.
    pub fn from_json(m: &Map<String, Value>) -> Session {
        let mut s = Session {
            id: text(m, "id").unwrap_or_default(),
            user_id: text(m, "userId"),
            library_id: text(m, "libraryId").filter(|v| !v.is_empty() && !v.starts_with("lib_")),
            library_item_id: text(m, "libraryItemId").filter(|v| !v.starts_with("li_") && !v.starts_with("local_")),
            book_id: text(m, "bookId").filter(|v| !v.is_empty()),
            episode_id: text(m, "episodeId").filter(|v| !v.starts_with("ep_") && !v.starts_with("local_")),
            media_type: m.get("mediaType").cloned().unwrap_or(Value::Null),
            duration: m.get("duration").cloned().unwrap_or(Value::Null),
            play_method: m.get("playMethod").cloned().unwrap_or(Value::Null),
            media_player: or(m.get("mediaPlayer"), Value::Null),
            device_info: Some(DeviceInfo::from_json(m.get("deviceInfo").unwrap_or(&Value::Null))),
            server_version: m.get("serverVersion").cloned().unwrap_or(Value::Null),
            chapters: or(m.get("chapters"), json!([])),
            media_metadata: m.get("mediaMetadata").cloned().unwrap_or(Value::Null),
            display_title: or(m.get("displayTitle"), json!("")),
            display_author: or(m.get("displayAuthor"), json!("")),
            cover_path: m.get("coverPath").cloned().unwrap_or(Value::Null),
            date: m.get("date").cloned().unwrap_or(Value::Null),
            day_of_week: m.get("dayOfWeek").cloned().unwrap_or(Value::Null),
            time_listening: or(m.get("timeListening"), Value::Null),
            start_time: or(m.get("startTime"), json!(0)),
            current_time: or(m.get("currentTime"), json!(0)),
            started_at: m.get("startedAt").cloned().unwrap_or(Value::Null),
            updated_at: or(m.get("updatedAt"), m.get("startedAt").cloned().unwrap_or(Value::Null)),
            ..Session::default()
        };
        if !crate::abs::settings::truthy(Some(&s.date)) && crate::abs::settings::truthy(m.get("updatedAt")) {
            if let Some(ms) = js_ms(m.get("updatedAt")) {
                let (day, weekday) = day_of(ms);
                s.date = json!(day);
                s.day_of_week = json!(weekday);
            }
        }
        s
    }

    /// The id of the book or episode played (`mediaItemId` as stored).
    pub fn media_item_id(&self) -> Option<String> {
        self.episode_id.clone().or_else(|| self.book_id.clone())
    }

    /// How far along the listener is, from 0 to 1 (`progress`).
    pub fn progress(&self) -> f64 {
        let duration = js_number(Some(&self.duration)).unwrap_or(0.0);
        if duration == 0.0 {
            return 0.0;
        }
        let current = js_number(Some(&self.current_time)).unwrap_or(f64::NAN);
        (current / duration).clamp(0.0, 1.0)
    }

    /// The device's description (`deviceDescription`).
    pub fn device_description(&self) -> String {
        self.device_info.as_ref().map_or_else(|| "No Device Info".into(), DeviceInfo::description)
    }

    /// The device's id.
    pub fn device_id(&self) -> Option<String> {
        self.device_info.as_ref().and_then(|d| d.id.clone())
    }

    /// The fields every JSON form of a session starts with.
    fn base(&self) -> Map<String, Value> {
        let o = |v: &Option<String>| v.clone().map_or(Value::Null, Value::from);
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("userId".into(), o(&self.user_id));
        m.insert("libraryId".into(), o(&self.library_id));
        m.insert("libraryItemId".into(), o(&self.library_item_id));
        m.insert("bookId".into(), o(&self.book_id));
        m.insert("episodeId".into(), o(&self.episode_id));
        m.insert("mediaType".into(), self.media_type.clone());
        m.insert("mediaMetadata".into(), self.media_metadata.clone());
        m.insert("chapters".into(), self.chapters.clone());
        m.insert("displayTitle".into(), self.display_title.clone());
        m.insert("displayAuthor".into(), self.display_author.clone());
        m.insert("coverPath".into(), self.cover_path.clone());
        m.insert("duration".into(), self.duration.clone());
        m.insert("playMethod".into(), self.play_method.clone());
        m.insert("mediaPlayer".into(), self.media_player.clone());
        m.insert("deviceInfo".into(), self.device_info.as_ref().map_or(Value::Null, DeviceInfo::to_json));
        m.insert("serverVersion".into(), self.server_version.clone());
        m.insert("date".into(), self.date.clone());
        m.insert("dayOfWeek".into(), self.day_of_week.clone());
        m.insert("timeListening".into(), self.time_listening.clone());
        m.insert("startTime".into(), self.start_time.clone());
        m.insert("currentTime".into(), self.current_time.clone());
        m.insert("startedAt".into(), self.started_at.clone());
        m.insert("updatedAt".into(), self.updated_at.clone());
        m
    }

    /// The session as JSON (`toJSON`).
    pub fn to_json(&self) -> Value {
        js_numbers(Value::Object(self.base()))
    }

    /// The session as a player gets it (`toJSONForClient`): with its tracks and the expanded library item.
    pub fn to_json_for_client(&self, library_item: Option<Value>) -> Value {
        let mut m = self.base();
        m.insert("audioTracks".into(), Value::Array(self.audio_tracks.clone()));
        m.insert("libraryItem".into(), library_item.unwrap_or(Value::Null));
        js_numbers(Value::Object(m))
    }

    /// The session object spread into a plain object (`{...session}`), as the session lists send it: also the
    /// fields the original keeps only in memory.
    pub fn to_spread_json(&self) -> Map<String, Value> {
        let mut m = self.base();
        m.insert("lastSave".into(), json!(self.last_save));
        m.insert("audioTracks".into(), Value::Array(self.audio_tracks.clone()));
        m.insert("stream".into(), Value::Null);
        m.insert("shareSessionId".into(), Value::Null);
        m.insert("mediaItemShareId".into(), Value::Null);
        m.insert("coverAspectRatio".into(), Value::Null);
        match js_numbers(Value::Object(m)) {
            Value::Object(m) => m,
            _ => Map::new(),
        }
    }

    /// Adds listened seconds (`addListeningTime`); nothing for a missing or non-number value.
    pub fn add_listening_time(&mut self, listened: Option<&Value>) {
        let Some(secs) = js_number(listened).filter(|n| *n != 0.0) else { return };
        let now = models::now_ms();
        if !crate::abs::settings::truthy(Some(&self.date)) {
            let (day, weekday) = day_of(now);
            self.date = json!(day);
            self.day_of_week = json!(weekday);
        }
        let before = js_number(Some(&self.time_listening)).unwrap_or(0.0);
        self.time_listening = json!(before + secs);
        self.updated_at = json!(now);
    }

    /// The stored row's values, in [`COLUMNS`] order (`getFromOld`).
    fn row_values(&self) -> Vec<Value> {
        let o = |v: &Option<String>| v.clone().map_or(Value::Null, Value::from);
        let date = |v: &Value| js_ms(Some(v)).map_or(Value::Null, |ms| Value::from(date_text(ms)));
        let mut extra = Map::new();
        if let Some(id) = &self.library_item_id {
            extra.insert("libraryItemId".into(), json!(id));
        }
        let number = |v: &Value| match v {
            Value::Null | Value::Number(_) => v.clone(),
            other => js_number(Some(other)).map_or(Value::Null, Value::from),
        };
        let string = |v: &Value| match v {
            Value::Null | Value::String(_) => v.clone(),
            other => Value::from(other.to_string()),
        };
        vec![
            json!(self.id),
            o(&self.media_item_id()),
            json!(if self.episode_id.is_some() { "podcastEpisode" } else { "book" }),
            string(&self.display_title),
            string(&self.display_author),
            number(&self.duration),
            number(&self.play_method),
            string(&self.media_player),
            number(&self.start_time),
            number(&self.current_time),
            if crate::abs::settings::truthy(Some(&self.server_version)) { string(&self.server_version) } else { Value::Null },
            string(&self.cover_path),
            number(&self.time_listening),
            if self.media_metadata.is_null() { Value::Null } else { Value::from(self.media_metadata.to_string()) },
            string(&self.date),
            string(&self.day_of_week),
            Value::from(Value::Object(extra).to_string()),
            date(&self.started_at),
            date(&self.updated_at),
            o(&self.user_id),
            o(&self.device_id()),
            o(&self.library_id),
        ]
    }

    /// The session from its stored row and device (`getOldPlaybackSession`).
    pub fn from_row(row: &Row, device: Option<DeviceInfo>) -> Session {
        let json_col = |k: &str| match row.get(k) {
            Some(Value::String(t)) => serde_json::from_str(t).unwrap_or(Value::Null),
            Some(other) => other.clone(),
            None => Value::Null,
        };
        let episode = row.get("mediaItemType").and_then(Value::as_str) == Some("podcastEpisode");
        let media_item = row.get("mediaItemId").cloned().unwrap_or(Value::Null);
        let ms = |k: &str| row.get(k).and_then(Value::as_str).and_then(date_ms).map_or(Value::Null, Value::from);
        let extra = json_col("extraData");
        let mut m = Map::new();
        m.insert("id".into(), row.get("id").cloned().unwrap_or(Value::Null));
        m.insert("userId".into(), row.get("userId").cloned().unwrap_or(Value::Null));
        m.insert("libraryId".into(), row.get("libraryId").cloned().unwrap_or(Value::Null));
        m.insert("libraryItemId".into(), or(extra.get("libraryItemId"), Value::Null));
        m.insert("bookId".into(), if episode { Value::Null } else { media_item.clone() });
        m.insert("episodeId".into(), if episode { media_item } else { Value::Null });
        m.insert("mediaType".into(), json!(if episode { "podcast" } else { "book" }));
        m.insert("mediaMetadata".into(), json_col("mediaMetadata"));
        for k in
            ["displayTitle", "displayAuthor", "coverPath", "duration", "playMethod", "mediaPlayer", "serverVersion", "date", "dayOfWeek"]
        {
            m.insert(k.into(), row.get(k).cloned().unwrap_or(Value::Null));
        }
        for k in ["timeListening", "startTime", "currentTime"] {
            m.insert(k.into(), row.get(k).cloned().unwrap_or(Value::Null));
        }
        m.insert("startedAt".into(), ms("createdAt"));
        m.insert("updatedAt".into(), ms("updatedAt"));
        let mut s = Session::from_json(&m);
        s.device_info = device;
        s
    }
}

/// A JSON time as milliseconds: a number, or a date text JavaScript's `Date` reads.
pub fn js_ms(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64),
        Value::String(t) => date_ms(t),
        _ => None,
    }
}

/// The stored columns, in the order [`Session::row_values`] gives them.
const COLUMNS: &[&str] = &[
    "id",
    "mediaItemId",
    "mediaItemType",
    "displayTitle",
    "displayAuthor",
    "duration",
    "playMethod",
    "mediaPlayer",
    "startTime",
    "currentTime",
    "serverVersion",
    "coverPath",
    "timeListening",
    "mediaMetadata",
    "date",
    "dayOfWeek",
    "extraData",
    "createdAt",
    "updatedAt",
    "userId",
    "deviceId",
    "libraryId",
];

/// Stores a new session, or replaces the stored one with the same id (`createFromOld`, an upsert).
pub fn create(db: &dyn Db, s: &Session) -> Result<()> {
    let cols = COLUMNS.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ");
    let marks = vec!["?"; COLUMNS.len()].join(", ");
    let set = COLUMNS[1..].iter().map(|c| format!("`{c}` = excluded.`{c}`")).collect::<Vec<_>>().join(", ");
    db.execute(
        &format!("INSERT INTO `playbackSessions` ({cols}) VALUES ({marks}) ON CONFLICT (`id`) DO UPDATE SET {set}"),
        s.row_values(),
    )?;
    Ok(())
}

/// Updates the stored session (`updateFromOld`).
pub fn update(db: &dyn Db, s: &Session) -> Result<()> {
    let mut values = s.row_values();
    let id = values.remove(0);
    values.push(id);
    let set = COLUMNS[1..].iter().map(|c| format!("`{c}` = ?")).collect::<Vec<_>>().join(", ");
    db.execute(&format!("UPDATE `playbackSessions` SET {set} WHERE `id` = ?"), values)?;
    Ok(())
}

/// Removes a stored session (`removeById`).
pub fn remove(db: &dyn Db, id: &str) -> Result<()> {
    db.execute("DELETE FROM `playbackSessions` WHERE `id` = ?", vec![json!(id)])?;
    Ok(())
}

/// The sessions of rows, each with its device.
pub fn from_rows(db: &dyn Db, rows: Vec<Row>) -> Result<Vec<Session>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let device = match row.get("deviceId").and_then(Value::as_str) {
            Some(id) => device::by_id(db, id)?,
            None => None,
        };
        out.push(Session::from_row(&row, device));
    }
    Ok(out)
}

/// A stored session by id (`getById`).
pub fn by_id(db: &dyn Db, id: &str) -> Result<Option<Session>> {
    let rows = db.query("SELECT * FROM `playbackSessions` WHERE `id` = ?", vec![json!(id)])?;
    Ok(from_rows(db, rows)?.into_iter().next())
}

/// A user's stored sessions, all or for one book or episode, newest change first (`getPlaybackSessions` sorted
/// like `getUserListeningSessionsHelper`).
pub fn of_user(db: &dyn Db, user_id: &str, media_item_id: Option<&str>) -> Result<Vec<Session>> {
    let rows = match media_item_id {
        Some(m) => db.query("SELECT * FROM `playbackSessions` WHERE `userId` = ? AND `mediaItemId` = ?", vec![json!(user_id), json!(m)])?,
        None => db.query("SELECT * FROM `playbackSessions` WHERE `userId` = ?", vec![json!(user_id)])?,
    };
    let mut list = from_rows(db, rows)?;
    list.sort_by(|a, b| {
        let (x, y) = (js_ms(Some(&a.updated_at)).unwrap_or(0), js_ms(Some(&b.updated_at)).unwrap_or(0));
        y.cmp(&x)
    });
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_are_named_like_the_original() {
        assert_eq!(day_of(0), ("1970-01-01".to_string(), "Thursday".to_string()));
        assert_eq!(day_of(1_793_901_844_788).1, "Thursday");
    }

    #[test]
    fn listening_time_adds_up_and_ignores_nonsense() {
        let mut s = Session { time_listening: json!(0), ..Session::default() };
        s.add_listening_time(Some(&json!(10)));
        s.add_listening_time(Some(&json!("4.5")));
        s.add_listening_time(Some(&json!("soon")));
        s.add_listening_time(None);
        assert_eq!(js_number(Some(&s.time_listening)), Some(14.5));
        assert!(s.date.is_string());
    }

    #[test]
    fn progress_is_kept_between_zero_and_one() {
        let s = Session { duration: json!(60), current_time: json!(90), ..Session::default() };
        assert_eq!(s.progress(), 1.0);
        let s = Session { duration: json!(0), current_time: json!(30), ..Session::default() };
        assert_eq!(s.progress(), 0.0);
    }
}
