//! The port of the original's logs (`server/Logger.js`, `server/managers/LogManager.js`,
//! `server/objects/DailyLog.js`): every message the server writes also goes, as one JSON line, to a file for the
//! day in `<metadata>/logs/daily/<YYYY-MM-DD>.txt`, only the newest `loggerDailyLogsToKeep` days stay, the
//! `logLevel` setting picks the least important level kept, admins read today's messages
//! (`GET /api/logger-data`) and the web app's log page follows new ones over the socket (`set_log_listener`).
//! Messages come from Folio's own `tracing` events through [`Layer`].

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::models::user;
use crate::AppState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};
use socketioxide::extract::SocketRef;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The original's levels (`LogLevel`).
pub mod level {
    /// Step-by-step detail.
    pub const TRACE: i64 = 0;
    /// Detail for finding problems.
    pub const DEBUG: i64 = 1;
    /// What the server did.
    pub const INFO: i64 = 2;
    /// Something odd that the server got past.
    pub const WARN: i64 = 3;
    /// Something that failed.
    pub const ERROR: i64 = 4;
    /// The server stopped.
    pub const FATAL: i64 = 5;
    /// A note shown whatever the level.
    pub const NOTE: i64 = 6;
}

/// A level's name (`getLogLevelString`).
pub fn level_name(l: i64) -> &'static str {
    match l {
        level::TRACE => "TRACE",
        level::DEBUG => "DEBUG",
        level::INFO => "INFO",
        level::WARN => "WARN",
        level::ERROR => "ERROR",
        level::FATAL => "FATAL",
        level::NOTE => "NOTE",
        _ => "UNKNOWN",
    }
}

/// The routes: `GET /api/logger-data`.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/logger-data", get(logger_data))
}

/// One day's log (`DailyLog`).
struct Daily {
    /// `YYYY-MM-DD`.
    id: String,
    /// Where its file is.
    path: PathBuf,
    /// Its messages so far.
    logs: Vec<Value>,
}

impl Daily {
    /// The log of today in a folder.
    fn today(dir: &Path) -> Daily {
        let id = today();
        Daily { path: dir.join(format!("{id}.txt")), id, logs: Vec::new() }
    }

    /// Reads the day's file, dropping lines that aren't messages and saving the file again without them
    /// (`loadLogs`).
    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(&self.path) else { return };
        let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let mut failed = false;
        self.logs = lines
            .iter()
            .filter_map(|l| {
                let v = serde_json::from_str::<Value>(l).ok();
                failed |= l.is_empty() || v.is_none();
                // JavaScript keeps only lines that read as something truthy
                v.filter(|v| !v.is_null() && v != &json!(false) && v != &json!(0) && v != &json!(""))
            })
            .collect();
        if failed {
            let text: String = self.logs.iter().map(|l| format!("{l}\n")).collect();
            if let Err(e) = std::fs::write(&self.path, text) {
                eprintln!("The log {} can't be saved again: {e}", self.path.display());
            }
        }
    }

    /// Adds a message and appends it to the file (`appendLog`).
    fn append(&mut self, log: Value) {
        let line = format!("{log}\n");
        self.logs.push(log);
        let written = std::fs::OpenOptions::new().create(true).append(true).open(&self.path).and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = written {
            eprintln!("The log {} can't be written: {e}", self.path.display());
        }
    }
}

/// Today as `YYYY-MM-DD` (UTC).
fn today() -> String {
    crate::abs::models::date_text(crate::abs::models::now_ms())[..10].to_string()
}

/// What the logs keep.
#[derive(Default)]
struct Inner {
    /// `<metadata>/logs/daily`, once set up.
    dir: Option<PathBuf>,
    /// Today's log, once set up.
    current: Option<Daily>,
    /// Messages from before the logs were set up.
    buffer: Vec<Value>,
    /// The day files there are, oldest first.
    files: Vec<String>,
    /// How many day files stay (`loggerDailyLogsToKeep`, 7 when not set).
    keep: f64,
    /// The least important level written (`logLevel`).
    level: i64,
    /// The sockets following the messages, with the least important level each wants.
    listeners: Vec<(String, i64, SocketRef)>,
}

/// The server's logs (`Logger` with its `LogManager`).
pub struct Logs {
    /// The state, locked.
    inner: Mutex<Inner>,
}

impl Default for Logs {
    fn default() -> Logs {
        Logs { inner: Mutex::new(Inner { keep: 7.0, level: level::INFO, ..Inner::default() }) }
    }
}

/// The logs the [`Layer`] writes to: the most recently started server's.
static ACTIVE: OnceLock<Mutex<Option<Arc<Logs>>>> = OnceLock::new();

/// Makes these the logs every message goes to.
pub fn install(logs: Arc<Logs>) {
    *ACTIVE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner()) = Some(logs);
}

/// The logs messages go to, if a server started.
fn active() -> Option<Arc<Logs>> {
    ACTIVE.get().and_then(|a| a.lock().ok().and_then(|a| a.clone()))
}

impl Logs {
    /// The state, locked.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets the logs up in `<metadata>/logs` (`LogManager.init`): the folders are made, the oldest day files past
    /// `keep` are removed, today's file is read (or started) and the messages from before are written to it.
    pub fn init(&self, metadata_path: &Path) {
        let dir = metadata_path.join("logs").join("daily");
        for d in [dir.clone(), metadata_path.join("logs").join("scans")] {
            if let Err(e) = std::fs::create_dir_all(&d) {
                eprintln!("The log folder {} can't be made: {e}", d.display());
            }
        }
        let mut inner = self.lock();
        let mut files: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        files.retain(|f| crate::abs::file_utils::extname(f) == ".txt");
        files.sort();
        inner.files = files;
        let keep = inner.keep;
        if inner.files.len() as f64 > keep {
            let extra = inner.files.len() - keep.max(0.0) as usize;
            for f in inner.files.clone().into_iter().take(extra) {
                remove_file(&mut inner, &dir, &f);
            }
        }
        let mut current = Daily::today(&dir);
        let name = format!("{}.txt", current.id);
        if inner.files.contains(&name) {
            current.load();
        } else {
            inner.files.push(name);
        }
        for log in std::mem::take(&mut inner.buffer) {
            current.append(log);
        }
        inner.current = Some(current);
        inner.dir = Some(dir);
    }

    /// Takes the settings in effect: `loggerDailyLogsToKeep` and `logLevel` (`setLogLevel`).
    pub fn apply_settings(&self, s: &crate::abs::settings::ServerSettings) {
        let keep = crate::abs::settings::js_number(s.get("loggerDailyLogsToKeep")).filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(7.0);
        let level = s.get("logLevel").and_then(Value::as_i64);
        let changed = {
            let mut inner = self.lock();
            inner.keep = keep;
            match level {
                Some(l) if l != inner.level => {
                    inner.level = l;
                    true
                }
                _ => false,
            }
        };
        if changed {
            tracing::debug!("Set Log Level to {}", level_name(level.unwrap_or(level::INFO)));
        }
    }

    /// The least important level written.
    pub fn level(&self) -> i64 {
        self.lock().level
    }

    /// Writes a message (`#log`): to the sockets following at its level, and to today's file when it is at the
    /// log level or more important.
    pub fn log(&self, lvl: i64, source: &str, message: &str) {
        let mut inner = self.lock();
        if lvl < level::FATAL && lvl < inner.level {
            return;
        }
        let entry = json!({
            "timestamp": timestamp(),
            "source": source,
            "message": message,
            "levelName": level_name(lvl),
            "level": lvl,
        });
        for (_, min, socket) in &inner.listeners {
            if lvl >= level::FATAL || lvl >= *min {
                let _ = socket.emit("log", &entry);
            }
        }
        let Some(dir) = inner.dir.clone() else {
            inner.buffer.push(entry);
            return;
        };
        // a new day starts a new file; the oldest goes when there are too many
        if inner.current.as_ref().is_none_or(|c| c.id != today()) {
            let daily = Daily::today(&dir);
            let name = format!("{}.txt", daily.id);
            inner.current = Some(daily);
            if inner.files.len() as f64 > inner.keep {
                if let Some(oldest) = inner.files.first().cloned() {
                    remove_file(&mut inner, &dir, &oldest);
                }
            }
            if !inner.files.contains(&name) {
                inner.files.push(name);
            }
        }
        if let Some(c) = inner.current.as_mut() {
            c.append(entry);
        }
    }

    /// The newest 5,000 messages of today (`getMostRecentCurrentDailyLogs`), or `""` before the logs are set up.
    pub fn recent(&self) -> Value {
        match &self.lock().current {
            Some(c) => Value::Array(c.logs[c.logs.len().saturating_sub(5000)..].to_vec()),
            None => json!(""),
        }
    }

    /// The day files there are, oldest first.
    pub fn files(&self) -> Vec<String> {
        self.lock().files.clone()
    }

    /// A socket follows the messages from a level up (`addSocketListener`).
    pub fn add_listener(&self, socket: SocketRef, lvl: i64) {
        let id = socket.id.to_string();
        let mut inner = self.lock();
        match inner.listeners.iter().position(|(i, _, _)| *i == id) {
            Some(i) => inner.listeners[i] = (id, lvl, socket),
            None => inner.listeners.push((id, lvl, socket)),
        }
    }

    /// A socket stops following the messages (`removeSocketListener`).
    pub fn remove_listener(&self, socket_id: &str) {
        self.lock().listeners.retain(|(i, _, _)| i != socket_id);
    }
}

/// Deletes a day file and forgets it (`removeLogFile`).
fn remove_file(inner: &mut Inner, dir: &Path, name: &str) {
    let path = dir.join(name);
    if !path.exists() {
        inner.files.retain(|f| f != name);
        return;
    }
    match std::fs::remove_file(&path) {
        Ok(()) => inner.files.retain(|f| f != name),
        Err(e) => eprintln!("The log {} can't be removed: {e}", path.display()),
    }
}

/// Now as `YYYY-MM-DD HH:mm:ss.SSS` (UTC).
fn timestamp() -> String {
    crate::abs::models::date_text(crate::abs::models::now_ms())[..23].to_string()
}

/// `GET /api/logger-data`: today's messages, for admins.
async fn logger_data(axum::extract::State(st): axum::extract::State<AppState>, me: AbsUser) -> Response {
    if !user::is_admin_or_up(&me.user) {
        tracing::error!("\"{}\" tried to read the logs but isn't an admin", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    Json(json!({ "currentDailyLogs": st.abs_server.logs.recent() })).into_response()
}

/// The `tracing` layer that hands Folio's own messages (from the `server` crate) to the logs.
pub struct Layer;

/// Collects an event's message and other fields as text.
#[derive(Default)]
struct Message(String, Map<String, Value>);

impl tracing::field::Visit for Message {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        } else {
            self.1.insert(field.name().to_string(), json!(format!("{value:?}")));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        } else {
            self.1.insert(field.name().to_string(), json!(value));
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Layer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        if !meta.target().starts_with("server") {
            return;
        }
        let Some(logs) = active() else { return };
        let lvl = match *meta.level() {
            tracing::Level::TRACE => level::TRACE,
            tracing::Level::DEBUG => level::DEBUG,
            tracing::Level::INFO => level::INFO,
            tracing::Level::WARN => level::WARN,
            tracing::Level::ERROR => level::ERROR,
        };
        let mut m = Message::default();
        event.record(&mut m);
        let mut text = m.0;
        for (k, v) in m.1 {
            text.push_str(&format!(" {k}={}", v.as_str().unwrap_or_default()));
        }
        let file = meta.file().map(|f| f.rsplit('/').next().unwrap_or(f)).unwrap_or(meta.target());
        let source = match meta.line() {
            Some(line) => format!("{file}:{line}"),
            None => file.to_string(),
        };
        logs.log(lvl, &source, &text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_go_to_the_day_file_from_the_log_level_up() {
        let dir = std::env::temp_dir().join(format!("folio-logs-{}", std::process::id()));
        let daily = dir.join("logs/daily");
        std::fs::create_dir_all(&daily).unwrap();
        for old in ["2020-01-01.txt", "2020-01-02.txt", "2020-01-03.txt", "notes.md"] {
            std::fs::write(daily.join(old), "").unwrap();
        }
        let logs = Logs::default();
        logs.log(level::INFO, "a.rs:1", "before the start");
        let mut s = crate::abs::settings::ServerSettings::defaults("/m");
        s.0.insert("loggerDailyLogsToKeep".into(), json!(2));
        logs.apply_settings(&s);
        logs.init(&dir);
        // like the original, `keep` old days stay and today's comes on top
        assert_eq!(logs.files(), ["2020-01-02.txt".to_string(), "2020-01-03.txt".to_string(), format!("{}.txt", today())]);
        assert!(!daily.join("2020-01-01.txt").exists());
        logs.log(level::DEBUG, "a.rs:2", "too detailed");
        logs.log(level::WARN, "a.rs:3", "kept");
        let recent = logs.recent();
        let messages: Vec<&str> = recent.as_array().unwrap().iter().map(|l| l["message"].as_str().unwrap()).collect();
        assert_eq!(messages, ["before the start", "kept"]);
        assert_eq!(recent[1]["levelName"], "WARN");
        assert_eq!(recent[1]["level"], 3);
        assert_eq!(recent[1]["source"], "a.rs:3");
        let text = std::fs::read_to_string(daily.join(format!("{}.txt", today()))).unwrap();
        assert_eq!(text.lines().count(), 2);

        // a restart reads today's file again, dropping a broken line
        std::fs::write(daily.join(format!("{}.txt", today())), format!("{text}not json\n")).unwrap();
        let again = Logs::default();
        again.init(&dir);
        assert_eq!(again.recent().as_array().unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(daily.join(format!("{}.txt", today()))).unwrap(), text);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_layer_turns_events_into_messages() {
        use tracing_subscriber::layer::SubscriberExt;
        let dir = std::env::temp_dir().join(format!("folio-logs-layer-{}", std::process::id()));
        let logs = Arc::new(Logs::default());
        logs.init(&dir);
        install(logs.clone());
        let subscriber = tracing_subscriber::registry().with(Layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(count = 3, "Something odd");
            tracing::trace!("too detailed");
        });
        let recent = logs.recent();
        let last = recent.as_array().unwrap().last().unwrap().clone();
        assert_eq!(last["message"], "Something odd count=3");
        assert_eq!(last["levelName"], "WARN");
        assert!(last["source"].as_str().unwrap().starts_with("logs.rs:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
