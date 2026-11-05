//! The port of the original's backups (`server/managers/BackupManager.js`, `server/objects/Backup.js`,
//! `server/controllers/BackupController.js`): a backup is a zip named `<YYYY-MM-DDTHHmm>.audiobookshelf` in the
//! backup folder holding a copy of the database, the items' and authors' metadata folders and a `details` file
//! (id, kind, time, version). Admins list, make, download, upload, restore and remove them and move the backup
//! folder; the schedule in the server settings makes them on its own and only the newest `backupsToKeep` stay.
//! Answers are the original's (tests/abs_backups.rs).

use crate::abs::auth::routes::body_fields;
use crate::abs::auth::{send_status, AbsUser};
use crate::abs::file_utils::{extname, resolve_posix, sanitize_filename};
use crate::abs::models::{self, media_progress::js_json, user};
use crate::abs::progress::send_text;
use crate::abs::{cron, notifications, settings};
use crate::AppState;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Multipart, Path as UrlPath, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use kit_server_core::Db;
use serde_json::{json, Map, Value};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tower::ServiceExt;

/// The routes, under `/api/backups`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/backups", get(all).post(create))
        .route("/api/backups/upload", post(upload).layer(DefaultBodyLimit::disable()))
        .route("/api/backups/path", patch(update_path))
        .route("/api/backups/{id}", delete(remove))
        .route("/api/backups/{id}/download", get(download))
        .route("/api/backups/{id}/apply", get(apply))
}

/// One backup (`Backup`).
#[derive(Clone, Debug, PartialEq)]
pub struct Backup {
    /// Its time as `YYYY-MM-DDTHHmm`, also its file name without the extension.
    pub id: String,
    /// `sqlite` for backups of the database (older ones have none); `None` inside leaves it out of the answer.
    pub key: Option<Value>,
    /// The time as people read it, `ddd, MMM D YYYY HH:mm`.
    pub date_pretty: String,
    /// The folder it is in.
    pub backup_dir_path: String,
    /// Its file name.
    pub filename: String,
    /// `backups/<file name>`.
    pub path: String,
    /// Where the file is.
    pub full_path: String,
    /// The version of the server that made it, if it says.
    pub server_version: Option<String>,
    /// Its size in bytes.
    pub file_size: Option<u64>,
    /// When it was made, in milliseconds since 1970 (`None` when the details don't say a number).
    pub created_at: Option<f64>,
}

/// Short English day and month names, as `date-and-time` writes them.
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// A time as `ddd, MMM D YYYY HH:mm` (UTC).
fn date_pretty(ms: Option<f64>) -> String {
    let Some(ms) = ms.filter(|m| m.is_finite()) else { return "Invalid Date".into() };
    let ms = ms as i64;
    let t = models::date_text(ms);
    let [_, minute, hour, day, month, weekday] = cron::time_parts(ms);
    format!("{}, {} {day} {} {hour:02}:{minute:02}", DAYS[weekday as usize], MONTHS[(month - 1) as usize], &t[..4])
}

impl Backup {
    /// A backup read from its `details` lines and where its file is (`construct`).
    pub fn from_details(details: &[&str], full_path: &Path) -> Backup {
        let id = details.first().copied().unwrap_or_default().to_string();
        // `this.key == 1` is true for '1' in older backups
        let key = details.get(1).map(|k| if k.trim() == "1" { Value::Null } else { json!(k) });
        let created_at = details.get(2).and_then(|t| js_number_text(t));
        let server_version = details.get(3).filter(|v| !v.is_empty()).map(|v| v.to_string());
        let dir = full_path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        let filename = full_path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        Backup {
            id,
            key,
            date_pretty: date_pretty(created_at),
            backup_dir_path: dir,
            path: format!("backups/{filename}"),
            filename,
            full_path: full_path.display().to_string(),
            server_version,
            file_size: None,
            created_at,
        }
    }

    /// A new backup made now in a folder (`setData`).
    pub fn new_now(dir: &str) -> Backup {
        let ms = models::now_ms();
        let t = models::date_text(ms);
        let id = format!("{}T{}{}", &t[..10], &t[11..13], &t[14..16]);
        let filename = format!("{id}.audiobookshelf");
        Backup {
            key: Some(json!("sqlite")),
            date_pretty: date_pretty(Some(ms as f64)),
            backup_dir_path: dir.to_string(),
            path: format!("backups/{filename}"),
            full_path: Path::new(dir).join(&filename).display().to_string(),
            filename,
            id,
            server_version: Some(crate::abs::ABS_VERSION.to_string()),
            file_size: None,
            created_at: Some(ms as f64),
        }
    }

    /// The `details` file: id, kind, time and version on their own lines (`detailsString`).
    pub fn details_string(&self) -> String {
        let key = match &self.key {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let created = self.created_at.map(|c| js_json(Some(c)).to_string()).unwrap_or_else(|| "NaN".into());
        [self.id.clone(), key, created, self.server_version.clone().unwrap_or_default()].join("\n")
    }

    /// The backup as the apps get it (`toJSON`).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        if let Some(k) = &self.key {
            m.insert("key".into(), k.clone());
        }
        m.insert("backupDirPath".into(), json!(self.backup_dir_path));
        m.insert("datePretty".into(), json!(self.date_pretty));
        m.insert("fullPath".into(), json!(self.full_path));
        m.insert("path".into(), json!(self.path));
        m.insert("filename".into(), json!(self.filename));
        m.insert("fileSize".into(), json!(self.file_size));
        m.insert("createdAt".into(), js_json(self.created_at));
        m.insert("serverVersion".into(), json!(self.server_version));
        Value::Object(m)
    }
}

/// JavaScript's `Number(text)` for a time stamp: a plain decimal number, `None` (NaN) otherwise.
fn js_number_text(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return Some(0.0);
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// The backups the server knows (`BackupManager.backups`) and the lock that lets one backup run at a time.
#[derive(Default)]
pub struct Manager {
    /// The backups, in the order they were found or made.
    list: Mutex<Vec<Backup>>,
    /// Held while a backup is being made.
    running: tokio::sync::Mutex<()>,
}

/// The largest `details` file read (1 MiB), like the original.
const MAX_DETAILS: u64 = 1024 * 1024;

/// What reading a backup file found.
enum Contents {
    /// The `details` lines and whether the database copy is in it.
    Found(String, bool),
    /// Not a zip the server can read.
    NotZip,
    /// No `details` file.
    NoDetails,
    /// A `details` file over 1 MiB.
    DetailsTooLarge,
}

/// Reads a backup file's `details` file.
fn read_backup(path: &Path) -> Contents {
    let Ok(file) = File::open(path) else { return Contents::NotZip };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return Contents::NotZip };
    let has_db = zip.index_for_name("absdatabase.sqlite").is_some();
    let Ok(mut entry) = zip.by_name("details") else { return Contents::NoDetails };
    if entry.size() > MAX_DETAILS {
        return Contents::DetailsTooLarge;
    }
    let mut data = Vec::new();
    if entry.by_ref().take(MAX_DETAILS).read_to_end(&mut data).is_err() {
        return Contents::NotZip;
    }
    Contents::Found(String::from_utf8_lossy(&data).into_owned(), has_db)
}

/// A file's size in bytes, if it can be read.
fn file_size(path: &str) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

impl Manager {
    /// The backups found in a folder; the folder is made when it is missing (`init`).
    pub fn new(dir: &str) -> Manager {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::error!("The backup folder {dir} can't be made: {e}");
        }
        let m = Manager::default();
        m.load(dir);
        m
    }

    /// Adds the backups in a folder (`loadBackups`): files ending in `.audiobookshelf` with a readable `details`
    /// file.
    pub fn load(&self, dir: &str) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            tracing::error!("The backups in {dir} can't be read");
            return;
        };
        let mut names: Vec<String> = entries.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let mut list = self.lock();
        for name in names.into_iter().filter(|n| n.ends_with(".audiobookshelf")) {
            let full = Path::new(dir).join(&name);
            let details = match read_backup(&full) {
                Contents::Found(d, _) => d,
                Contents::NoDetails => {
                    tracing::error!("The backup {} has no details file, so it was skipped", full.display());
                    continue;
                }
                Contents::DetailsTooLarge => {
                    tracing::error!("The backup {} has a details file that is too large, so it was skipped", full.display());
                    continue;
                }
                Contents::NotZip => {
                    tracing::error!("The backup {} can't be opened", full.display());
                    continue;
                }
            };
            let lines: Vec<&str> = details.split('\n').collect();
            let mut b = Backup::from_details(&lines, &full);
            if b.server_version.is_none() {
                tracing::error!("The backup {} is from before version 2 and can't be used", b.filename);
            } else if !b.key.as_ref().is_some_and(|k| !k.is_null() && k.as_str() != Some("")) {
                tracing::warn!("The backup {} is from before the database change and can't be used", b.filename);
            }
            b.file_size = file_size(&b.full_path);
            if list.iter().any(|x| x.id == b.id) {
                tracing::warn!("A backup with the id {} is already loaded, so {} was skipped", b.id, b.filename);
            } else {
                list.push(b);
            }
        }
        tracing::info!("{} backups found", list.len());
    }

    /// Forgets the backups and loads the ones in a folder (`reload`).
    pub fn reload(&self, dir: &str) {
        self.lock().clear();
        self.load(dir);
    }

    /// The list, locked.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Backup>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The backups as the apps get them.
    pub fn json(&self) -> Vec<Value> {
        self.lock().iter().map(Backup::to_json).collect()
    }

    /// A backup by id.
    pub fn find(&self, id: &str) -> Option<Backup> {
        self.lock().iter().find(|b| b.id == id).cloned()
    }

    /// Adds a backup, or puts it in place of one with the same id.
    fn put(&self, b: Backup) {
        let mut list = self.lock();
        match list.iter().position(|x| x.id == b.id) {
            Some(i) => list[i] = b,
            None => list.push(b),
        }
    }

    /// Deletes a backup's file and forgets it (`removeBackup`).
    pub fn remove(&self, b: &Backup) {
        match std::fs::remove_file(&b.full_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::error!("The backup {} can't be deleted: {e}", b.full_path);
                return;
            }
        }
        self.lock().retain(|x| x.id != b.id);
        tracing::info!("The backup {} was removed", b.id);
    }
}

/// The original's check for every backup route (`middleware`): admins only, and the backup must exist.
fn guard(st: &AppState, me: &models::User, id: Option<&str>) -> Result<Option<Backup>, Box<Response>> {
    if !user::is_admin_or_up(me) {
        tracing::error!("\"{}\" tried to use the backups but isn't an admin", me.username.as_deref().unwrap_or_default());
        return Err(Box::new(send_status(StatusCode::FORBIDDEN)));
    }
    match id {
        Some(id) => st.abs_server.backups.find(id).map(Some).ok_or_else(|| Box::new(send_status(StatusCode::NOT_FOUND))),
        None => Ok(None),
    }
}

/// The backup folder in effect (`backupPath`).
fn backup_path(st: &AppState) -> String {
    st.abs_server.settings().str("backupPath").to_string()
}

/// `{ backups }`, the answer after a change.
fn backups_answer(st: &AppState) -> Response {
    Json(json!({ "backups": st.abs_server.backups.json() })).into_response()
}

/// `GET /api/backups`.
async fn all(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Err(r) = guard(&st, &me.user, None) {
        return *r;
    }
    Json(json!({
        "backups": st.abs_server.backups.json(),
        "backupLocation": backup_path(&st),
        "backupPathEnvSet": std::env::var_os("BACKUP_PATH").is_some_and(|v| !v.is_empty()),
    }))
    .into_response()
}

/// `POST /api/backups`: makes a backup now (`requestCreateBackup`).
async fn create(State(st): State<AppState>, me: AbsUser) -> Response {
    if let Err(r) = guard(&st, &me.user, None) {
        return *r;
    }
    if run(&st).await {
        backups_answer(&st)
    } else {
        send_status(StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// `DELETE /api/backups/:id`.
async fn remove(State(st): State<AppState>, me: AbsUser, UrlPath(id): UrlPath<String>) -> Response {
    let b = match guard(&st, &me.user, Some(&id)) {
        Ok(Some(b)) => b,
        Ok(None) => return send_status(StatusCode::NOT_FOUND),
        Err(r) => return *r,
    };
    st.abs_server.backups.remove(&b);
    backups_answer(&st)
}

/// `GET /api/backups/:id/download`: the backup file as a download.
async fn download(State(st): State<AppState>, me: AbsUser, UrlPath(id): UrlPath<String>, req: Request) -> Response {
    let b = match guard(&st, &me.user, Some(&id)) {
        Ok(Some(b)) => b,
        Ok(None) => return send_status(StatusCode::NOT_FOUND),
        Err(r) => return *r,
    };
    let served = tower_http::services::ServeFile::new(&b.full_path).oneshot(req).await;
    let mut r = match served {
        Ok(r) => r.map(Body::new),
        Err(_) => return send_status(StatusCode::NOT_FOUND),
    };
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename={}", b.filename)) {
        r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=0"));
    r
}

/// `GET /api/backups/:id/apply`: puts the backup's database and metadata in place of the current ones
/// (`requestApplyBackup`).
async fn apply(State(st): State<AppState>, me: AbsUser, UrlPath(id): UrlPath<String>) -> Response {
    let b = match guard(&st, &me.user, Some(&id)) {
        Ok(Some(b)) => b,
        Ok(None) => return send_status(StatusCode::NOT_FOUND),
        Err(r) => return *r,
    };
    tracing::info!("Restoring the backup {}", b.full_path);
    let st2 = st.clone();
    let done = tokio::task::spawn_blocking(move || restore(&st2, &b)).await;
    match done {
        Ok(Ok(())) => {}
        Ok(Err(r)) => return *r,
        Err(_) => return send_status(StatusCode::INTERNAL_SERVER_ERROR),
    }
    let r = send_status(StatusCode::OK);
    st.sockets.emitter("backup_applied", &Value::Null, None);
    r
}

/// The restore's work: the database's tables replaced by the backup's, the metadata folders written out, the
/// settings read again and the cache emptied.
fn restore(st: &AppState, b: &Backup) -> Result<(), Box<Response>> {
    let fail = |text: &str| Box::new(send_text(StatusCode::INTERNAL_SERVER_ERROR, text));
    let file = File::open(&b.full_path).map_err(|_| fail("Failed to extract sqlite db from backup"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| fail("Failed to extract sqlite db from backup"))?;
    if zip.index_for_name("absdatabase.sqlite").is_none() {
        tracing::error!("The backup {} is too old to restore", b.full_path);
        return Err(fail("Invalid backup file. Does not include absdatabase.sqlite. This might be from an older Audiobookshelf server."));
    }
    let server = &st.abs_server;
    let temp = server.config_path.join("absdatabase-temp.sqlite");
    let extracted = zip
        .by_name("absdatabase.sqlite")
        .ok()
        .and_then(|mut entry| File::create(&temp).ok().and_then(|mut out| std::io::copy(&mut entry, &mut out).ok()))
        .is_some();
    if !extracted {
        tracing::error!("The database in the backup can't be read, so nothing was restored");
        let _ = std::fs::remove_file(&temp);
        return Err(fail("Failed to extract sqlite db from backup"));
    }
    let copied = copy_tables(&server.config_path.join("absdatabase.sqlite"), &temp);
    let _ = std::fs::remove_file(&temp);
    if let Err(e) = copied {
        tracing::error!("The database can't be replaced by the backup's: {e}");
        return Err(fail(&format!("Failed to overwrite sqlite db: {e}")));
    }
    tracing::info!("The database was restored from {}", b.full_path);
    for (prefix, dir) in [("metadata-items/", "items"), ("metadata-authors/", "authors")] {
        let dir = server.metadata_path.join(dir);
        let _ = std::fs::create_dir_all(&dir);
        extract_folder(&mut zip, prefix, &dir);
    }
    server.reload_settings(&st.abs);
    notifications::forget(st);
    purge_cache(&server.metadata_path);
    let path = backup_path(st);
    server.backups.reload(&path);
    Ok(())
}

/// Replaces every table of the database file `db` with the same table of another database file: the columns both
/// have are copied, in one transaction on a connection of its own (the original swaps the database file itself).
fn copy_tables(db: &Path, from: &Path) -> Result<(), String> {
    let mut conn = rusqlite::Connection::open(db).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(|e| e.to_string())?;
    conn.execute("ATTACH DATABASE ?1 AS abs_backup", [from.display().to_string()]).map_err(|e| e.to_string())?;
    let names = |conn: &rusqlite::Connection, schema: &str| -> rusqlite::Result<Vec<String>> {
        let mut q = conn.prepare(&format!("SELECT name FROM {schema}.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"))?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    let columns = |conn: &rusqlite::Connection, schema: &str, table: &str| -> rusqlite::Result<Vec<String>> {
        let mut q = conn.prepare("SELECT name FROM pragma_table_info(?1, ?2)")?;
        let rows = q.query_map([table, schema], |r| r.get::<_, String>(0))?;
        rows.collect()
    };
    let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let copy = |conn: &mut rusqlite::Connection| -> rusqlite::Result<()> {
        let mut tables = names(conn, "main")?;
        let theirs = names(conn, "abs_backup")?;
        // the items' titles are kept up to date by triggers on other tables, so they go first and come back last
        tables.sort_by_key(|t| t != "libraryItems");
        let tx = conn.transaction()?;
        for t in &tables {
            tx.execute(&format!("DELETE FROM main.{}", quote(t)), [])?;
        }
        tables.reverse();
        for t in tables.iter().filter(|t| theirs.contains(t)) {
            let mine = columns(&tx, "main", t)?;
            let both: Vec<String> =
                columns(&tx, "abs_backup", t)?.into_iter().filter(|c| mine.contains(c)).map(|c| quote(c.as_str())).collect();
            if both.is_empty() {
                continue;
            }
            let cols = both.join(", ");
            tx.execute(&format!("INSERT INTO main.{q} ({cols}) SELECT {cols} FROM abs_backup.{q}", q = quote(t)), [])?;
        }
        tx.commit()
    };
    let result = copy(&mut conn).map_err(|e| e.to_string());
    let _ = conn.execute("DETACH DATABASE abs_backup", []);
    result
}

/// Writes out the files of a zip under a folder name (`zip.extract('metadata-items/', path)`); names that would
/// leave the folder are skipped.
fn extract_folder(zip: &mut zip::ZipArchive<File>, prefix: &str, dir: &Path) {
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else { continue };
        let Some(name) = entry.enclosed_name() else { continue };
        let Ok(rel) = name.strip_prefix(prefix.trim_end_matches('/')) else { continue };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dir.join(rel);
        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&target);
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let written = File::create(&target).and_then(|mut out| std::io::copy(&mut entry, &mut out));
        if let Err(e) = written {
            tracing::error!("The file {} from the backup can't be written: {e}", target.display());
        }
    }
}

/// Empties the cache of covers, images and items (`CacheManager.purgeAll`).
pub fn purge_cache(metadata_path: &Path) {
    let cache = metadata_path.join("cache");
    if cache.exists() {
        if let Err(e) = std::fs::remove_dir_all(&cache) {
            tracing::error!("The cache folder {} can't be removed: {e}", cache.display());
        }
    }
    for sub in ["covers", "images", "items"] {
        let _ = std::fs::create_dir_all(cache.join(sub));
    }
}

/// Makes a backup (`runBackup`): the database copy, the metadata folders and the details zipped into the backup
/// folder; the oldest goes when there are more than `backupsToKeep`. Sends the "backup done" or "backup failed"
/// notifications. True when the backup was made.
pub async fn run(st: &AppState) -> bool {
    let _one = st.abs_server.backups.running.lock().await;
    tracing::info!("Making a backup");
    let s = st.abs_server.settings();
    let mut b = Backup::new_now(s.str("backupPath"));
    let keep = settings::js_number(s.get("backupsToKeep")).filter(|n| *n != 0.0 && !n.is_nan()).unwrap_or(2.0);
    let max_size = settings::js_number(s.get("maxBackupSize")).filter(|n| *n != 0.0 && !n.is_nan());
    let st2 = st.clone();
    let b2 = b.clone();
    let made = tokio::task::spawn_blocking(move || make(&st2, &b2, max_size)).await.unwrap_or_else(|e| Err(e.to_string()));
    if let Err(e) = made {
        tracing::error!("The backup failed: {e}");
        if st.abs_server.backups.find(&b.id).is_none() {
            let _ = std::fs::remove_file(&b.full_path);
        }
        failed(st, &e).await;
        return false;
    }
    tracing::info!("The backup {} was made", b.id);
    b.file_size = file_size(&b.full_path);
    let backups = &st.abs_server.backups;
    backups.put(b.clone());
    let remove_oldest = backups.lock().len() as f64 > keep;
    if remove_oldest {
        let oldest = {
            let mut list = backups.lock();
            list.sort_by(|x, y| x.created_at.unwrap_or(f64::NAN).total_cmp(&y.created_at.unwrap_or(f64::NAN)));
            list.first().cloned()
        };
        if let Some(old) = oldest {
            tracing::debug!("Removing the old backup {}", old.id);
            backups.remove(&old);
        }
    }
    let count = backups.lock().len();
    if notifications::has_active(st, "onBackupCompleted") {
        let mut data = Map::new();
        data.insert("completionTime".into(), js_json(b.created_at));
        data.insert("backupPath".into(), json!(b.full_path));
        data.insert("backupSize".into(), json!(b.file_size));
        data.insert("backupCount".into(), if count == 0 { json!("Invalid") } else { json!(count) });
        data.insert("removedOldest".into(), if remove_oldest { json!(true) } else { json!("false") });
        let _ = notifications::trigger(st, "onBackupCompleted", data, false).await;
    }
    true
}

/// Sends the "backup failed" notifications (`onBackupFailed`).
async fn failed(st: &AppState, message: &str) {
    if notifications::has_active(st, "onBackupFailed") {
        let mut data = Map::new();
        data.insert("errorMsg".into(), json!(if message.is_empty() { "Backup failed" } else { message }));
        let _ = notifications::trigger(st, "onBackupFailed", data, false).await;
    }
}

/// The backup's work: the database copied (`backupSqliteDb`) and zipped with the metadata folders (`zipBackup`).
fn make(st: &AppState, b: &Backup, max_size_gb: Option<f64>) -> Result<(), String> {
    let server = &st.abs_server;
    let authors = server.metadata_path.join("authors");
    let _ = std::fs::create_dir_all(&authors);
    let copy = server.config_path.join(format!("absdatabase.{}.sqlite", b.id));
    let _ = std::fs::remove_file(&copy);
    st.abs.execute("VACUUM INTO ?", vec![json!(copy.display().to_string())]).map_err(|e| e.detail)?;
    let zipped = zip_backup(&copy, b, &server.metadata_path.join("items"), &authors, max_size_gb);
    let _ = std::fs::remove_file(&copy);
    zipped
}

/// Writes the backup zip; stops with "Backup too large" when more than `max_size_gb` GB were read.
fn zip_backup(db_copy: &Path, b: &Backup, items: &Path, authors: &Path, max_size_gb: Option<f64>) -> Result<(), String> {
    let out = File::create(&b.full_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).compression_level(Some(9));
    let limit = max_size_gb.map(|g| g * 1000.0 * 1000.0 * 1000.0);
    let mut processed = 0f64;
    let mut add = |zip: &mut zip::ZipWriter<File>, path: &Path, name: &str| -> Result<(), String> {
        let mut f = File::open(path).map_err(|e| e.to_string())?;
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        processed += std::io::copy(&mut f, zip).map_err(|e| e.to_string())? as f64;
        if limit.is_some_and(|l| processed > l) {
            tracing::error!("The backup is too large ({processed} bytes read), so it was stopped");
            return Err("Backup too large".into());
        }
        Ok(())
    };
    add(&mut zip, db_copy, "absdatabase.sqlite")?;
    for (dir, name) in [(items, "metadata-items"), (authors, "metadata-authors")] {
        for (file, rel) in walk(dir) {
            add(&mut zip, &file, &format!("{name}/{rel}"))?;
        }
    }
    zip.start_file("details", opts).map_err(|e| e.to_string())?;
    zip.write_all(b.details_string().as_bytes()).map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

/// Every file under a folder with its path inside it (`/`-separated), in name order; nothing for a missing
/// folder.
fn walk(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, rel)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push((e.path(), r)),
                Ok(t) if t.is_file() => out.push((e.path(), r)),
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// `PATCH /api/backups/path`: moves the backup folder (`updatePath`); the folder is made when it is missing.
async fn update_path(State(st): State<AppState>, me: AbsUser, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(r) = guard(&st, &me.user, None) {
        return *r;
    }
    let b = body_fields(&headers, &body);
    let Some(given) = b.get("path").and_then(Value::as_str).filter(|p| !p.trim().is_empty()) else {
        tracing::error!("The new backup folder is missing");
        return send_text(StatusCode::BAD_REQUEST, "Invalid request body. Must include path.");
    };
    let new_path = resolve_posix(given);
    if new_path == backup_path(&st) {
        return send_text(StatusCode::OK, "Backup path unchanged");
    }
    tracing::info!("Moving the backup folder to {new_path}");
    if std::env::var_os("BACKUP_PATH").is_some_and(|v| !v.is_empty()) {
        tracing::warn!("BACKUP_PATH is set, so the backup folder goes back to it when the server restarts");
    }
    if !Path::new(&new_path).exists() {
        if let Err(e) = std::fs::create_dir(&new_path) {
            tracing::error!("The backup folder {new_path} can't be made: {e}");
            return send_text(StatusCode::BAD_REQUEST, &format!("Invalid backup path \"{given}\""));
        }
    }
    let mut s = st.abs_server.settings();
    s.0.insert("backupPath".into(), json!(new_path));
    if let Err(e) = st.abs_server.save_settings(&st.abs, s) {
        return e.into_response();
    }
    st.abs_server.backups.reload(&new_path);
    send_status(StatusCode::OK)
}

/// Express's page for an error thrown in a route.
const EXPRESS_ERROR_PAGE: &str = "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>Internal Server Error</pre>\n</body>\n</html>\n";

/// `POST /api/backups/upload`: adds a backup file sent from the browser (`uploadBackup`).
async fn upload(State(st): State<AppState>, me: AbsUser, multipart: Option<Multipart>) -> Response {
    if let Err(r) = guard(&st, &me.user, None) {
        return *r;
    }
    let mut file: Option<(String, Bytes)> = None;
    let mut any_file = false;
    if let Some(mut mp) = multipart {
        while let Ok(Some(field)) = mp.next_field().await {
            if field.file_name().is_none() {
                continue;
            }
            any_file = true;
            if field.name() == Some("file") && file.is_none() {
                let name = field.file_name().unwrap_or_default().to_string();
                match field.bytes().await {
                    Ok(data) => file = Some((name, data)),
                    Err(_) => return send_status(StatusCode::INTERNAL_SERVER_ERROR),
                }
            }
        }
    }
    if !any_file {
        // the original reads `req.files.file` without files, which Express answers with its error page
        tracing::error!("The upload has no files");
        return send_text(StatusCode::INTERNAL_SERVER_ERROR, EXPRESS_ERROR_PAGE);
    }
    let Some((name, data)) = file else {
        tracing::error!("The uploaded backup is missing");
        return send_status(StatusCode::INTERNAL_SERVER_ERROR);
    };
    if extname(&name) != ".audiobookshelf" {
        tracing::error!("The uploaded file {name} isn't a backup");
        return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Invalid backup file");
    }
    let temp = Path::new(&backup_path(&st)).join(sanitize_filename(&name));
    if let Err(e) = std::fs::write(&temp, &data) {
        tracing::error!("The uploaded backup can't be saved at {}: {e}", temp.display());
        return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Failed to move backup file into backups directory");
    }
    let reject = |code: StatusCode, text: &str| {
        tracing::error!("The uploaded backup was refused: {text}");
        if let Err(e) = std::fs::remove_file(&temp) {
            tracing::error!("The refused backup {} can't be removed: {e}", temp.display());
        }
        send_text(code, text)
    };
    let details = match read_backup(&temp) {
        Contents::NotZip => return reject(StatusCode::BAD_REQUEST, "Failed to read backup file - backup might not be a valid .zip file"),
        Contents::Found(_, false) => {
            return reject(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid backup with no absdatabase.sqlite file - might be a backup created on an old Audiobookshelf server.",
            )
        }
        Contents::NoDetails if !zip_has_db(&temp) => {
            return reject(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid backup with no absdatabase.sqlite file - might be a backup created on an old Audiobookshelf server.",
            )
        }
        Contents::NoDetails => return reject(StatusCode::BAD_REQUEST, "Invalid backup file - missing details entry"),
        Contents::DetailsTooLarge if !zip_has_db(&temp) => {
            return reject(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid backup with no absdatabase.sqlite file - might be a backup created on an old Audiobookshelf server.",
            )
        }
        Contents::DetailsTooLarge => return reject(StatusCode::BAD_REQUEST, "Invalid backup file - details entry too large"),
        Contents::Found(d, true) => d,
    };
    let lines: Vec<&str> = details.split('\n').collect();
    let mut b = Backup::from_details(&lines, &temp);
    if b.server_version.is_none() {
        return reject(StatusCode::INTERNAL_SERVER_ERROR, "Invalid backup. Might be a backup created before version 2.0.0.");
    }
    b.file_size = file_size(&b.full_path);
    if st.abs_server.backups.find(&b.id).is_some() {
        tracing::warn!("A backup with the id {} was already there and was replaced", b.id);
    }
    st.abs_server.backups.put(b);
    backups_answer(&st)
}

/// Whether a zip has the database copy.
fn zip_has_db(path: &Path) -> bool {
    File::open(path).ok().and_then(|f| zip::ZipArchive::new(f).ok()).is_some_and(|z| z.index_for_name("absdatabase.sqlite").is_some())
}

/// Starts the backup schedule (`scheduleCron`): every second the `backupSchedule` setting is checked against the
/// time (UTC), and a backup is made when it matches. Turning the schedule off or changing it takes effect at once
/// (`updateCronSchedule`).
pub fn spawn_schedule(st: AppState) {
    tokio::spawn(async move {
        let mut parsed: Option<(String, Option<Vec<String>>)> = None;
        loop {
            let now = models::now_ms();
            let wait = 1000 - now.rem_euclid(1000);
            tokio::time::sleep(std::time::Duration::from_millis(wait as u64)).await;
            let schedule = st.abs_server.settings().get("backupSchedule").and_then(Value::as_str).unwrap_or_default().to_string();
            if schedule.is_empty() {
                parsed = None;
                continue;
            }
            if parsed.as_ref().is_none_or(|(s, _)| *s != schedule) {
                let fields = cron::validate(&schedule).then(|| cron::convert(&schedule)).flatten();
                if fields.is_none() {
                    tracing::error!("The backup schedule {schedule} isn't valid, so no backups are made on their own");
                } else {
                    tracing::info!("Backups are made on the schedule {schedule}");
                }
                parsed = Some((schedule, fields));
            }
            let Some((_, Some(fields))) = &parsed else { continue };
            if cron::matches(fields, cron::time_parts(models::now_ms())) {
                let st2 = st.clone();
                tokio::spawn(async move {
                    run(&st2).await;
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_are_read_like_the_original() {
        let b =
            Backup::from_details(&["2026-11-05T1800", "sqlite", "1793901600000", "2.37.2"], Path::new("/b/2026-11-05T1800.audiobookshelf"));
        assert_eq!(b.date_pretty, "Thu, Nov 5 2026 18:00");
        assert_eq!(b.path, "backups/2026-11-05T1800.audiobookshelf");
        assert_eq!(b.backup_dir_path, "/b");
        assert_eq!(b.details_string(), "2026-11-05T1800\nsqlite\n1793901600000\n2.37.2");
        let old = Backup::from_details(&["x", "1", "5"], Path::new("/b/x.audiobookshelf"));
        assert_eq!(old.key, Some(Value::Null));
        assert_eq!(old.server_version, None);
        let short = Backup::from_details(&["x"], Path::new("/b/x.audiobookshelf"));
        assert!(short.to_json().get("key").is_none());
        assert_eq!(short.to_json()["createdAt"], Value::Null);
    }

    #[test]
    fn a_backup_over_the_size_limit_is_stopped() {
        let dir = std::env::temp_dir().join(format!("folio-backup-size-{}", std::process::id()));
        let items = dir.join("items/one");
        std::fs::create_dir_all(&items).unwrap();
        std::fs::write(items.join("cover.jpg"), vec![7u8; 4096]).unwrap();
        let db = dir.join("db.sqlite");
        std::fs::write(&db, b"db").unwrap();
        let b = Backup::new_now(&dir.display().to_string());
        let authors = dir.join("authors");
        // 2 KB at most: the cover is too large
        let r = zip_backup(&db, &b, &dir.join("items"), &authors, Some(2e-6));
        assert_eq!(r, Err("Backup too large".to_string()));
        // without a limit the zip holds the database, the item's file and the details
        zip_backup(&db, &b, &dir.join("items"), &authors, None).unwrap();
        let z = zip::ZipArchive::new(File::open(&b.full_path).unwrap()).unwrap();
        let mut names: Vec<&str> = z.file_names().collect();
        names.sort();
        assert_eq!(names, ["absdatabase.sqlite", "details", "metadata-items/one/cover.jpg"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
