//! What a scan found for one library item, and how it compares with the stored item, like the original's
//! `LibraryItemScanData`.

use super::files::{self, ext_in, AUDIO_TYPES, EBOOK_TYPES, IMAGE_TYPES};
use super::meta::{BookMeta, SeriesRef};
use super::scandir::FolderMetadata;
use super::ScanLog;
use crate::abs::models::{self, LibraryItem};
use crate::abs::ABS_VERSION;
use kit_server_core::{Db, Result};
use serde_json::{json, Value};

/// One library item as found on disk.
#[derive(Clone, Debug, Default)]
pub struct ItemScanData {
    /// The library folder it is in.
    pub library_folder_id: String,
    /// Its library.
    pub library_id: String,
    /// "book" or "podcast".
    pub media_type: String,
    /// The inode of its folder (or file).
    pub ino: String,
    /// The folder's modified time.
    pub mtime_ms: i64,
    /// The folder's changed time.
    pub ctime_ms: i64,
    /// The folder's made time.
    pub birthtime_ms: i64,
    /// The whole path.
    pub path: String,
    /// The path below the library folder.
    pub rel_path: String,
    /// Whether the item is a single file at the top of the folder.
    pub is_file: bool,
    /// What the folder names say.
    pub media_metadata: FolderMetadata,
    /// The files, as stored library files.
    pub library_files: Vec<Value>,
    /// Whether the stored item had to change (set by [`ItemScanData::check_library_item`]).
    pub has_changes: bool,
    /// Whether the item's path changed.
    pub has_path_change: bool,
    /// Stored files no longer there.
    pub files_removed: Vec<Value>,
    /// Files new since the last scan.
    pub files_added: Vec<Value>,
    /// Files that changed: (before, after).
    pub files_modified: Vec<(Value, Value)>,
}

/// Whether a stored file has one of `types` as extension.
fn is(lf: &Value, types: &[&str]) -> bool {
    ext_in(&files::file_ext(lf), types)
}

impl ItemScanData {
    /// The audio files found.
    pub fn audio_files(&self) -> Vec<Value> {
        self.library_files.iter().filter(|f| is(f, AUDIO_TYPES)).cloned().collect()
    }
    /// The pictures found.
    pub fn image_files(&self) -> Vec<Value> {
        self.library_files.iter().filter(|f| is(f, IMAGE_TYPES)).cloned().collect()
    }
    /// The ebooks found.
    pub fn ebook_files(&self) -> Vec<Value> {
        self.library_files.iter().filter(|f| is(f, EBOOK_TYPES)).cloned().collect()
    }
    /// Audio files that changed: (before, after).
    pub fn audio_modified(&self) -> Vec<(Value, Value)> {
        self.files_modified.iter().filter(|(old, _)| is(old, AUDIO_TYPES)).cloned().collect()
    }
    /// Audio files no longer there.
    pub fn audio_removed(&self) -> Vec<Value> {
        self.files_removed.iter().filter(|f| is(f, AUDIO_TYPES)).cloned().collect()
    }
    /// Audio files new since the last scan.
    pub fn audio_added(&self) -> Vec<Value> {
        self.files_added.iter().filter(|f| is(f, AUDIO_TYPES)).cloned().collect()
    }
    /// Pictures that changed.
    pub fn image_modified(&self) -> Vec<(Value, Value)> {
        self.files_modified.iter().filter(|(old, _)| is(old, IMAGE_TYPES)).cloned().collect()
    }
    /// Pictures no longer there.
    pub fn image_removed(&self) -> Vec<Value> {
        self.files_removed.iter().filter(|f| is(f, IMAGE_TYPES)).cloned().collect()
    }
    /// Ebooks that changed.
    pub fn ebook_modified(&self) -> Vec<(Value, Value)> {
        self.files_modified.iter().filter(|(old, _)| is(old, EBOOK_TYPES)).cloned().collect()
    }
    /// Whether any audio file was added, removed or changed.
    pub fn has_audio_changes(&self) -> bool {
        !self.audio_removed().is_empty() || !self.audio_added().is_empty() || !self.audio_modified().is_empty()
    }
    /// Whether any file was added, removed or changed.
    pub fn has_file_changes(&self) -> bool {
        !self.files_removed.is_empty() || !self.files_modified.is_empty() || !self.files_added.is_empty()
    }
    /// The file with this name.
    fn named(&self, name: &str) -> Option<Value> {
        self.library_files.iter().find(|f| f["metadata"]["filename"] == name).cloned()
    }
    /// `desc.txt`.
    pub fn desc_txt(&self) -> Option<Value> {
        self.named("desc.txt")
    }
    /// `reader.txt`.
    pub fn reader_txt(&self) -> Option<Value> {
        self.named("reader.txt")
    }
    /// `metadata.json`.
    pub fn metadata_json(&self) -> Option<Value> {
        self.named("metadata.json")
    }
    /// The first `.opf` file.
    pub fn opf_file(&self) -> Option<Value> {
        self.library_files.iter().find(|f| files::file_ext(f).to_lowercase() == ".opf").cloned()
    }
    /// The first `.nfo` file.
    pub fn nfo_file(&self) -> Option<Value> {
        self.library_files.iter().find(|f| files::file_ext(f).to_lowercase() == ".nfo").cloned()
    }

    /// The size of all files together.
    pub fn size(&self) -> i64 {
        self.library_files.iter().map(|f| f["metadata"]["size"].as_i64().unwrap_or(0)).sum()
    }

    /// Applies what the folder names say to a book's details (`setBookMetadataFromFilenames`).
    pub fn apply_folder_names(&self, book: &mut BookMeta) {
        let m = &self.media_metadata;
        let texts = [
            ("title", Some(m.title.clone())),
            ("subtitle", m.subtitle.clone()),
            ("publishedYear", m.published_year.clone()),
            ("asin", m.asin.clone()),
        ];
        for (key, value) in texts {
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                if m.title_only && key != "title" {
                    continue;
                }
                book.set(key, Value::String(v));
            }
        }
        if !m.authors.is_empty() {
            book.authors = m.authors.clone();
        }
        if !m.narrators.is_empty() {
            book.narrators = m.narrators.clone();
        }
        if let Some(name) = m.series_name.as_ref().filter(|n| !n.is_empty()) {
            book.series = vec![SeriesRef { name: name.clone(), sequence: m.series_sequence.clone().filter(|s| !s.is_empty()) }];
        }
    }

    /// Whether a stored audio file is gone (`checkAudioFileRemoved`).
    pub fn audio_file_removed(&self, af: &Value) -> bool {
        let removed = self.audio_removed();
        if removed.is_empty() {
            return false;
        }
        removed.iter().any(|r| files::file_path(r) == files::file_path(af))
            || removed.iter().any(|r| files::file_ino(r) == files::file_ino(af))
    }

    /// Whether the book's ebook is gone (`checkEbookFileRemoved`).
    pub fn ebook_file_removed(&self, ebook: &Value) -> bool {
        let ebooks = self.ebook_files();
        if ebooks.is_empty() {
            return true;
        }
        if ebooks.iter().any(|e| files::file_path(e) == files::file_path(ebook)) {
            return false;
        }
        !ebooks.iter().any(|e| files::file_ino(e) == files::file_ino(ebook))
    }

    /// Compares the stored item with what was found and saves the differences (`checkLibraryItemData`): path,
    /// inode, times, missing flag, and files added, removed and changed. True when the item changed.
    pub fn check_library_item(&mut self, db: &dyn Db, item: &mut LibraryItem, log: &mut ScanLog) -> Result<bool> {
        self.has_changes = false;
        self.has_path_change = false;
        let rel = item.rel_path.clone().unwrap_or_default();
        let mut set = |name: &str, old: &mut Option<String>, new: &str, path: bool, me: &mut ItemScanData| {
            if old.as_deref() != Some(new) {
                log.debug(format!(
                    "Library item \"{rel}\" key \"{name}\" changed from \"{}\" to \"{new}\"",
                    old.clone().unwrap_or_default()
                ));
                *old = Some(new.to_string());
                me.has_changes = true;
                if path {
                    me.has_path_change = true;
                }
            }
        };
        let folder_id = self.library_folder_id.clone();
        let ino = self.ino.clone();
        let path = self.path.clone();
        let rel_path = self.rel_path.clone();
        set("libraryFolderId", &mut item.library_folder_id, &folder_id, false, self);
        set("ino", &mut item.ino, &ino, false, self);
        set("path", &mut item.path, &path, true, self);
        set("relPath", &mut item.rel_path, &rel_path, true, self);
        if item.is_file != Some(self.is_file) {
            item.is_file = Some(self.is_file);
            self.has_changes = true;
        }
        for (name, stored, found) in [
            ("mtime", &mut item.mtime, self.mtime_ms),
            ("birthtime", &mut item.birthtime, self.birthtime_ms),
            ("ctime", &mut item.ctime, self.ctime_ms),
        ] {
            if stored.as_deref().and_then(models::date_ms) != Some(found) {
                log.debug(format!("Library item \"{rel}\" key \"{name}\" changed to \"{found}\""));
                *stored = Some(models::date_text(found));
                self.has_changes = true;
            }
        }
        if item.is_missing == Some(true) {
            log.debug(format!("Library item \"{rel}\" was missing but now found"));
            item.is_missing = Some(false);
            self.has_changes = true;
        }
        self.files_removed.clear();
        self.files_modified.clear();
        let mut added: Vec<usize> = (0..self.library_files.len()).collect();
        let mut stored: Vec<Value> = item.library_files.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
        let mut kept: Vec<Value> = Vec::new();
        for mut existing in stored.drain(..) {
            let mut matching = self.library_files.iter().position(|lf| files::file_path(lf) == files::file_path(&existing));
            if matching.is_none() {
                matching = self.library_files.iter().position(|lf| files::file_ino(lf) == files::file_ino(&existing));
                if let Some(m) = matching {
                    log.info(format!(
                        "Library file with path \"{}\" not found, but found file with matching inode value at path \"{}\"",
                        files::file_path(&existing),
                        files::file_path(&self.library_files[m])
                    ));
                }
            }
            match matching {
                None => {
                    log.info(format!("Library file \"{}\" was removed from library item \"{rel}\"", files::file_path(&existing)));
                    self.files_removed.push(existing);
                    self.has_changes = true;
                }
                Some(m) => {
                    added.retain(|i| *i != m);
                    let before = existing.clone();
                    if compare_update_file(&mut existing, &self.library_files[m]) {
                        self.files_modified.push((before, existing.clone()));
                        self.has_changes = true;
                    }
                    kept.push(existing);
                }
            }
        }
        self.files_added.clear();
        if !added.is_empty() {
            self.has_changes = true;
            for i in added {
                let mut lf = self.library_files[i].clone();
                log.info(format!("New library file found with path \"{}\" for library item \"{rel}\"", files::file_path(&lf)));
                if lf["fileType"] == "ebook" {
                    lf["isSupplementary"] = json!(true);
                }
                kept.push(lf.clone());
                self.files_added.push(lf);
            }
        }
        if self.has_changes {
            item.size = Some(kept.iter().map(|lf| lf["metadata"]["size"].as_i64().unwrap_or(0)).sum());
            item.library_files = Some(Value::Array(kept));
            let now = models::now_ms();
            item.last_scan = Some(models::date_text(now));
            item.last_scan_version = Some(ABS_VERSION.into());
            item.updated_at = Some(models::date_text(now));
            models::update(db, item)?;
            return Ok(true);
        }
        Ok(false)
    }
}

/// Updates a stored file from the file found (`compareUpdateLibraryFile`); true when anything changed.
fn compare_update_file(existing: &mut Value, scanned: &Value) -> bool {
    let mut changed = false;
    if existing["ino"] != scanned["ino"] {
        existing["ino"] = scanned["ino"].clone();
        changed = true;
    }
    let keys: Vec<String> = existing["metadata"].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for key in keys {
        let new = scanned["metadata"].get(&key).cloned().unwrap_or(Value::Null);
        if existing["metadata"][&key] != new {
            existing["metadata"][&key] = new;
            changed = true;
        }
    }
    if changed {
        existing["updatedAt"] = json!(models::now_ms());
    }
    changed
}
