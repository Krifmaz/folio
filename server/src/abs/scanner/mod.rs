//! The port of the original's library scanner (`server/scanner/`): a scan walks each library folder, groups the
//! files into items, compares them with the stored items, rescans the ones that changed, marks the missing ones
//! and adds the new ones (`LibraryScanner.scan`). Books are ported; podcast folders come with P5's podcasts.
//!
//! `POST /api/libraries/:id/scan` (A0022) starts a scan.

pub mod audio;
pub mod book;
pub mod covers;
pub mod files;
pub mod item_data;
pub mod meta;
pub mod probe;
pub mod scandir;
pub mod xml;

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::util::Sorting;
use crate::abs::models::{self, Author, Library, LibraryItem};
use crate::abs::{settings::ServerSettings, socket::Sockets, ABS_VERSION};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use item_data::ItemScanData;
use kit_server_core::{Db, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// The scanner's routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/libraries/{id}/scan", post(scan_route))
}

/// What a scan works with: the database, the settings at its start, where metadata goes and the sockets to tell.
pub struct Scan<'a> {
    /// The Audiobookshelf database.
    pub db: &'a dyn Db,
    /// The server settings.
    pub settings: ServerSettings,
    /// How titles are sorted (`sortingIgnorePrefix`, `sortingPrefixes`).
    pub sorting: Sorting,
    /// The original's `MetadataPath`.
    pub metadata_path: PathBuf,
    /// The connected apps, told about added and changed items.
    pub sockets: Option<Sockets>,
}

impl Scan<'_> {
    /// Tells every connected app about an event (`SocketAuthority.emitter`).
    pub fn emit(&self, event: &str, data: &impl Serialize) {
        if let Some(s) = &self.sockets {
            s.emitter(event, data, None);
        }
    }
}

/// One line of a scan's log (`LibraryScan.addLog`).
#[derive(Clone, Debug, Serialize)]
pub struct LogLine {
    /// When, as an ISO time.
    pub timestamp: String,
    /// What happened.
    pub message: String,
    /// "DEBUG", "INFO", "WARN" or "ERROR".
    #[serde(rename = "levelName")]
    pub level_name: &'static str,
    /// The level's number.
    pub level: u8,
}

/// A scan's log and what it found (`LibraryScan`, `ScanLogger`).
#[derive(Debug, Default)]
pub struct ScanLog {
    /// The log lines.
    pub lines: Vec<LogLine>,
    /// Items added.
    pub added: u32,
    /// Items updated.
    pub updated: u32,
    /// Items missing.
    pub missing: u32,
    /// Authors whose number of books changed.
    pub authors_num_books_changed: HashSet<String>,
    /// Authors taken off a book (removed at the end if no book has them).
    pub authors_removed_from_books: Vec<String>,
    /// Series taken off a book.
    pub series_removed_from_books: Vec<String>,
}

impl ScanLog {
    /// Adds a line.
    fn add(&mut self, level: u8, level_name: &'static str, message: String) {
        tracing::debug!(target: "abs::scanner", "{message}");
        self.lines.push(LogLine { timestamp: iso_now(), message, level_name, level });
    }
    /// A detail.
    pub fn debug(&mut self, m: String) {
        self.add(1, "DEBUG", m);
    }
    /// Something that happened.
    pub fn info(&mut self, m: String) {
        self.add(2, "INFO", m);
    }
    /// Something odd.
    pub fn warn(&mut self, m: String) {
        self.add(3, "WARN", m);
    }
    /// Something that failed.
    pub fn error(&mut self, m: String) {
        self.add(4, "ERROR", m);
    }
    /// "2 added, 1 updated" (`resultStats`).
    pub fn stats(&self) -> String {
        format!("{} added | {} updated | {} missing", self.added, self.updated, self.missing)
    }
}

/// Now as an ISO time ("2026-11-05T19:40:50.481Z").
fn iso_now() -> String {
    models::date_text(models::now_ms()).replacen(' ', "T", 1).replace(" +00:00", "Z")
}

/// An author as the apps get it (`Author.toOldJSON`).
pub fn author_json(a: &Author) -> Value {
    json!({
        "id": a.id, "asin": a.asin, "name": a.name, "description": a.description, "imagePath": a.image_path,
        "libraryId": a.library_id,
        "addedAt": a.created_at.as_deref().and_then(models::date_ms), "updatedAt": a.updated_at.as_deref().and_then(models::date_ms),
    })
}

/// The libraries being scanned, and the ones asked to stop (`librariesScanning`, `cancelLibraryScan`).
static SCANNING: LazyLock<Mutex<(HashSet<String>, HashSet<String>)>> = LazyLock::new(|| Mutex::new((HashSet::new(), HashSet::new())));

/// Whether a library is being scanned (`isLibraryScanning`).
pub fn is_scanning(library_id: &str) -> bool {
    SCANNING.lock().map(|s| s.0.contains(library_id)).unwrap_or(false)
}

/// Asks a running scan to stop (`setCancelLibraryScan`).
pub fn cancel(library_id: &str) {
    if let Ok(mut s) = SCANNING.lock() {
        if s.0.contains(library_id) {
            s.1.insert(library_id.to_string());
        }
    }
}

/// Whether the library's scan was asked to stop.
fn cancelled(library_id: &str) -> bool {
    SCANNING.lock().map(|s| s.1.contains(library_id)).unwrap_or(false)
}

/// A task as the apps see it (`Task.toJSON`).
fn task_json(
    id: &str,
    lib: &Library,
    started: i64,
    finished: Option<i64>,
    failed: bool,
    results: Option<Value>,
    description: Option<&str>,
) -> Value {
    let name = lib.name.clone().unwrap_or_default();
    let mut data = json!({"libraryId": lib.id, "libraryName": name, "libraryMediaType": lib.media_type});
    if let Some(r) = results {
        data["scanResults"] = r;
    }
    json!({
        "id": id, "action": "library-scan", "data": data,
        "title": format!("Scanning \"{name}\" library"), "titleKey": "MessageTaskScanningLibrary", "titleSubs": [name],
        "description": description, "descriptionKey": description.map(|_| "MessageTaskCanceledByUser"), "descriptionSubs": null,
        "error": if failed { json!("Failed") } else { Value::Null }, "errorKey": if failed { json!("MessageTaskFailed") } else { Value::Null }, "errorSubs": null,
        "showSuccess": true, "isFailed": failed, "isFinished": finished.is_some(), "startedAt": started, "finishedAt": finished,
    })
}

/// Scans a library (`LibraryScanner.scan`): does nothing when it is being scanned already or has no folders. A
/// change of the metadata order since the last scan forces every book to be read again.
pub fn scan_library(scan: &Scan, library_id: &str, mut force: bool) -> Result<()> {
    let db = scan.db;
    let Some(mut lib) = models::find::<Library>(db, library_id)? else { return Ok(()) };
    let folders = crate::abs::library::folders(db, library_id)?;
    {
        let Ok(mut s) = SCANNING.lock() else { return Ok(()) };
        if s.0.contains(library_id) {
            tracing::warn!("Already scanning {library_id}");
            return Ok(());
        }
        if folders.is_empty() {
            return Ok(());
        }
        s.0.insert(library_id.to_string());
    }
    let settings =
        lib.settings.clone().unwrap_or_else(|| crate::abs::library::default_settings(lib.media_type.as_deref().unwrap_or("book")));
    let order: Vec<String> = book::precedence(&settings);
    let is_book = lib.media_type.as_deref() != Some("podcast");
    let last: Vec<String> = lib
        .extra_data
        .as_ref()
        .and_then(|e| e.get("lastScanMetadataPrecedence"))
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    if is_book && order.join(",") != last.join(",") {
        force = true;
    }
    let task_id = models::new_id();
    let started = models::now_ms();
    scan.emit("task_started", &task_json(&task_id, &lib, started, None, false, None, None));
    let mut log = ScanLog::default();
    let result = scan_items(scan, &lib, &folders, &settings, force, &mut log);
    let finished = models::now_ms();
    let results =
        json!({"added": log.added, "updated": log.updated, "missing": log.missing, "elapsed": finished - started, "text": log.stats()});
    let task = match &result {
        Ok(false) => {
            lib.last_scan = Some(models::date_text(finished));
            lib.last_scan_version = Some(ABS_VERSION.into());
            if is_book {
                let mut extra = lib.extra_data.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
                extra["lastScanMetadataPrecedence"] = json!(order);
                lib.extra_data = Some(extra);
            }
            lib.updated_at = Some(models::date_now());
            models::update(db, &lib)?;
            task_json(&task_id, &lib, started, Some(finished), false, Some(results), None)
        }
        Ok(true) => task_json(&task_id, &lib, started, Some(finished), false, Some(results), Some("Task canceled by user")),
        Err(e) => {
            log.error(format!("Library scan failed: {}", e.detail));
            task_json(&task_id, &lib, started, Some(finished), true, Some(results), None)
        }
    };
    if let Ok(mut s) = SCANNING.lock() {
        s.0.remove(library_id);
        s.1.remove(library_id);
    }
    scan.emit("task_finished", &task);
    save_log(scan, &lib, &task_id, started, finished, &log);
    result.map(|_| ())
}

/// Writes a scan's log to `<metadata>/logs/scans/<date>_<id>.txt` (`LibraryScan.saveLog`).
fn save_log(scan: &Scan, lib: &Library, id: &str, started: i64, finished: i64, log: &ScanLog) {
    let dir = scan.metadata_path.join("logs").join("scans");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let date = models::date_text(finished)[..10].to_string();
    let head = json!({
        "id": id, "type": "scan", "library": crate::abs::library::library_json(lib, &[]), "startedAt": started, "finishedAt": finished,
        "elapsed": finished - started, "resultsAdded": log.added, "resultsUpdated": log.updated, "resultsMissing": log.missing,
    });
    let mut lines = vec![head.to_string()];
    lines.extend(log.lines.iter().map(|l| serde_json::to_string(l).unwrap_or_default()));
    let _ = std::fs::write(dir.join(format!("{date}_{id}.txt")), lines.join("\n"));
}

/// The scan's work (`scanLibrary`); true when it was asked to stop.
fn scan_items(
    scan: &Scan,
    lib: &Library,
    folders: &[models::LibraryFolder],
    settings: &Value,
    force: bool,
    log: &mut ScanLog,
) -> Result<bool> {
    let db = scan.db;
    let library_id = lib.id.clone().unwrap_or_default();
    let media_type = lib.media_type.clone().unwrap_or_else(|| "book".into());
    // The filter data is loaded first: it is where authors and series are looked up by name.
    crate::abs::library::filter_data(db, &media_type, &library_id)?;
    let mut found: Vec<ItemScanData> = Vec::new();
    for folder in folders {
        let items = scan_folder(scan, lib, folder, settings);
        log.info(format!("{} item data found in folder \"{}\"", items.len(), folder.path.clone().unwrap_or_default()));
        found.extend(items);
    }
    if cancelled(&library_id) {
        return Ok(true);
    }
    let existing: Vec<LibraryItem> = models::list(db, "`libraryId` = ?", "", vec![json!(library_id)])?;
    let mut missing_ids: Vec<String> = Vec::new();
    let mut updated: Vec<String> = Vec::new();
    for mut item in existing {
        let path = item.path.clone().unwrap_or_default();
        let mut pos = found.iter().position(|d| d.path == path);
        if pos.is_none() {
            pos = found.iter().position(|d| item_ino_match(d, &item));
            if let Some(p) = pos {
                log.info(format!(
                    "Library item with path \"{path}\" was not found, but library item inode was found at path \"{}\"",
                    found[p].path
                ));
            }
        }
        match pos {
            None => {
                if media_type == "podcast" && files::exists(&path) {
                    log.info(format!("Library item \"{}\" folder exists but has no episodes", item.rel_path.clone().unwrap_or_default()));
                } else {
                    log.warn(format!("Library Item \"{path}\" is missing"));
                    log.missing += 1;
                    if item.is_missing != Some(true) {
                        let id = item.id.clone().unwrap_or_default();
                        item.is_missing = Some(true);
                        item.updated_at = Some(models::date_now());
                        models::update(db, &item)?;
                        missing_ids.push(id.clone());
                        updated.push(id);
                    }
                }
            }
            Some(p) => {
                let mut data = found.remove(p);
                let changed = data.check_library_item(db, &mut item, log)?;
                if changed || force {
                    if force || data.has_file_changes() || data.has_path_change {
                        let was_updated = rescan_item(scan, &mut item, &data, settings, log)?;
                        if !force || was_updated {
                            log.updated += 1;
                            updated.push(item.id.clone().unwrap_or_default());
                        }
                    } else {
                        log.updated += 1;
                        updated.push(item.id.clone().unwrap_or_default());
                    }
                }
            }
        }
        if updated.len() == 10 {
            emit_items(scan, "items_updated", &std::mem::take(&mut updated))?;
        }
        if cancelled(&library_id) {
            return Ok(true);
        }
    }
    if !updated.is_empty() {
        emit_items(scan, "items_updated", &updated)?;
    }
    if !missing_ids.is_empty() {
        let now = models::date_now();
        for id in &missing_ids {
            db.execute(
                "UPDATE libraryItems SET isMissing = 1, lastScan = ?, lastScanVersion = ?, updatedAt = ? WHERE id = ?",
                vec![json!(now), json!(ABS_VERSION), json!(now), json!(id)],
            )?;
        }
    }
    let mut added: Vec<String> = Vec::new();
    for data in &found {
        if media_type != "book" {
            log.warn(format!("Podcast folder \"{}\" is not scanned yet", data.rel_path));
            continue;
        }
        if let Some(item) = book::new_book_item(scan, data, settings, log)? {
            log.info(format!("Created new library item \"{}\" with id \"{}\"", data.rel_path, item.id.clone().unwrap_or_default()));
            log.added += 1;
            added.push(item.id.clone().unwrap_or_default());
        }
        if added.len() == 10 {
            emit_items(scan, "items_added", &std::mem::take(&mut added))?;
        }
        if cancelled(&library_id) {
            return Ok(true);
        }
    }
    if !added.is_empty() {
        emit_items(scan, "items_added", &added)?;
    }
    book::finalize_authors_and_series(scan, &library_id, log)?;
    log.info(format!("Scan completed. {}", log.stats()));
    Ok(false)
}

/// Rescans an item's media (`rescanLibraryItemMedia`).
fn rescan_item(scan: &Scan, item: &mut LibraryItem, data: &ItemScanData, settings: &Value, log: &mut ScanLog) -> Result<bool> {
    if item.media_type.as_deref() == Some("book") {
        book::rescan_book_item(scan, item, data, settings, log)
    } else {
        Ok(false)
    }
}

/// Whether a found item is the stored one moved: the same inode, or a single file that is one of the other's
/// files (`ItemToItemInoMatch`, `ItemToFileInoMatch`).
fn item_ino_match(found: &ItemScanData, item: &LibraryItem) -> bool {
    let ino = item.ino.clone().unwrap_or_default();
    if found.ino == ino {
        return true;
    }
    if found.is_file
        && item
            .library_files
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|l| l.iter().any(|lf| files::file_ino(lf).as_deref() == Some(found.ino.as_str())))
    {
        return true;
    }
    item.is_file == Some(true) && found.library_files.iter().any(|lf| files::file_ino(lf).as_deref() == Some(ino.as_str()))
}

/// Sends items to the apps that may see them (`libraryItemsEmitter`).
fn emit_items(scan: &Scan, event: &str, ids: &[String]) -> Result<()> {
    let Some(sockets) = &scan.sockets else { return Ok(()) };
    let mut items = Vec::new();
    for id in ids {
        if let Some((item, b)) = crate::abs::items::expanded_by_id(scan.db, id)? {
            let j = Value::Object(crate::abs::models::library_item::expanded(&item, &b, &scan.sorting));
            items.push(((item, b), j));
        }
    }
    sockets.library_items_emitter(event, &items, &|u, (item, b)| crate::abs::items::can_access(u, item, b));
    Ok(())
}

/// The items in one library folder (`scanFolder`), in the order the folder walk meets them.
pub fn scan_folder(scan: &Scan, lib: &Library, folder: &models::LibraryFolder, settings: &Value) -> Vec<ItemScanData> {
    let folder_path = folder.path.clone().unwrap_or_default();
    if !files::exists(&folder_path) {
        tracing::error!("Invalid folder path does not exist \"{folder_path}\"");
        return vec![];
    }
    let media_type = lib.media_type.clone().unwrap_or_else(|| "book".into());
    let audiobooks_only = crate::abs::settings::truthy(settings.get("audiobooksOnly"));
    let list = files::recurse_files(&folder_path, None);
    let groups = scandir::group_items(&media_type, &list, audiobooks_only, false);
    let parse_subtitle = crate::abs::settings::truthy(scan.settings.get("scannerParseSubtitle"));
    let now = models::now_ms();
    let mut out = Vec::new();
    for (key, group) in groups {
        let (path, rel_path, meta, file_list, is_file) = match group {
            scandir::Group::Single(name) => {
                let title = files::basename(&name, Some(&files::extname(&name)));
                let path = files::join(&[&folder_path, &name]);
                let meta = scandir::FolderMetadata { title, title_only: true, ..Default::default() };
                (path.clone(), key.clone(), meta, vec![files::library_file(&path, &name, now)], true)
            }
            scandir::Group::Folder(list) => {
                let (path, rel, meta) = scandir::data_from_media_dir(&media_type, &folder_path, &key, parse_subtitle);
                let lfs = list.iter().map(|f| files::library_file(&files::join(&[&path, f]), f, now)).collect();
                (path, rel, meta, lfs, false)
            }
        };
        let Some(st) = files::stamps(&path).filter(|s| !s.ino.is_empty()) else {
            tracing::warn!("Library item folder \"{path}\" has no inode value");
            continue;
        };
        out.push(ItemScanData {
            library_folder_id: folder.id.clone().unwrap_or_default(),
            library_id: folder.library_id.clone().unwrap_or_default(),
            media_type: media_type.clone(),
            ino: st.ino,
            mtime_ms: st.mtime_ms,
            ctime_ms: st.ctime_ms,
            birthtime_ms: st.birthtime_ms,
            path,
            rel_path,
            is_file,
            media_metadata: meta,
            library_files: file_list,
            ..Default::default()
        });
    }
    out
}

/// Runs a scan for a library in the background, with the state's database, settings and sockets.
pub fn spawn_scan(st: &AppState, library_id: &str, force: bool) -> tokio::task::JoinHandle<()> {
    let st = st.clone();
    let id = library_id.to_string();
    tokio::task::spawn_blocking(move || {
        let settings = st.abs_server.settings();
        let scan = Scan {
            db: &st.abs,
            sorting: Sorting::of(&settings),
            settings,
            metadata_path: st.abs_server.metadata_path.clone(),
            sockets: Some(st.sockets.clone()),
        };
        if let Err(e) = scan_library(&scan, &id, force) {
            tracing::error!("The library scan failed: {}", e.detail);
        }
        if let Err(e) = crate::abs::library::reset_issues(&st.abs, &id) {
            tracing::error!("The library's issue count can't be updated: {}", e.detail);
        }
    })
}

/// `POST /api/libraries/:id/scan` (A0022, `LibraryController.scan`): admins only; answers "OK" at once and scans
/// in the background, every book again with `force=1`.
async fn scan_route(
    State(st): State<AppState>,
    AbsUser { user, .. }: AbsUser,
    Path(id): Path<String>,
    Query(mut q): Query<HashMap<String, String>>,
) -> Response {
    let lib = match crate::abs::library::library_for(&st, &user, &id, &mut q) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    if !crate::abs::models::user::is_admin_or_up(&user) {
        return send_status(StatusCode::FORBIDDEN);
    }
    let force = q.get("force").map(String::as_str) == Some("1");
    spawn_scan(&st, lib.id.as_deref().unwrap_or_default(), force);
    send_status(StatusCode::OK)
}
