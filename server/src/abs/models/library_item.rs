//! A library item (one book or podcast folder or file) and the JSON the original sends for it
//! (`server/models/LibraryItem.js`): `toOldJSON`, `toOldJSONMinified` and `toOldJSONExpanded`, with its files as
//! `LibraryFile` JSON (`server/objects/files/LibraryFile.js`).

use super::book::BookFull;
use super::LibraryItem;
use crate::abs::library::util::{b, ms, ms0, pick, put_some, s, Sorting};
use serde_json::{Map, Value};

/// Image formats (`globals.SupportedImageTypes`).
pub const IMAGE_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// Audio formats (`globals.SupportedAudioTypes`).
pub const AUDIO_TYPES: &[&str] = &[
    "m4b", "mp3", "m4a", "flac", "opus", "ogg", "oga", "mp4", "aac", "wma", "aiff", "aif", "wav", "webm", "webma", "mka", "awb", "caf",
    "mpg", "mpeg",
];
/// Ebook formats (`globals.SupportedEbookTypes`).
pub const EBOOK_TYPES: &[&str] = &["epub", "pdf", "mobi", "azw3", "cbr", "cbz"];
/// Text formats (`globals.TextFileTypes`).
pub const TEXT_TYPES: &[&str] = &["txt", "nfo"];
/// Metadata formats (`globals.MetadataFileTypes`).
pub const METADATA_TYPES: &[&str] = &["opf", "abs", "xml", "json"];

/// What kind of file an extension is: `image`, `audio`, `ebook`, `text`, `metadata` or `unknown`
/// (`LibraryFile.fileType`).
pub fn file_type(ext: Option<&str>) -> &'static str {
    let format = ext.map(|e| e.chars().skip(1).collect::<String>().to_lowercase()).unwrap_or_default();
    let f = format.as_str();
    if IMAGE_TYPES.contains(&f) {
        "image"
    } else if AUDIO_TYPES.contains(&f) {
        "audio"
    } else if EBOOK_TYPES.contains(&f) {
        "ebook"
    } else if TEXT_TYPES.contains(&f) {
        "text"
    } else if METADATA_TYPES.contains(&f) {
        "metadata"
    } else {
        "unknown"
    }
}

/// One stored library file as `LibraryFile.toJSON()`: its metadata's known keys, `isSupplementary` (`null` when
/// never set) and the `fileType` worked out from the extension.
pub fn library_file_json(file: &Value) -> Value {
    let mut m = pick(file, &["ino"]);
    let meta = file.get("metadata").cloned().unwrap_or(Value::Null);
    let meta_json = pick(&meta, &["filename", "ext", "path", "relPath", "size", "mtimeMs", "ctimeMs", "birthtimeMs"]);
    m.insert("metadata".into(), Value::Object(meta_json));
    m.insert("isSupplementary".into(), file.get("isSupplementary").cloned().unwrap_or(Value::Null));
    put_some(&mut m, "addedAt", file.get("addedAt").cloned());
    put_some(&mut m, "updatedAt", file.get("updatedAt").cloned());
    m.insert("fileType".into(), Value::from(file_type(meta.get("ext").and_then(Value::as_str))));
    Value::Object(m)
}

/// The stored files of an item.
pub fn library_files(item: &LibraryItem) -> Vec<Value> {
    match &item.library_files {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// Every file of an item as `LibraryFile` JSON (`getLibraryFilesJson`).
pub fn library_files_json(item: &LibraryItem) -> Value {
    Value::Array(library_files(item).iter().map(library_file_json).collect())
}

/// The keys every item JSON starts with.
fn base(item: &LibraryItem) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("id".into(), s(&item.id));
    m.insert("ino".into(), s(&item.ino));
    let old = item.extra_data.as_ref().and_then(|e| e.get("oldLibraryItemId")).filter(|v| super::book::truthy(v));
    m.insert("oldLibraryItemId".into(), old.cloned().unwrap_or(Value::Null));
    m.insert("libraryId".into(), s(&item.library_id));
    m.insert("folderId".into(), s(&item.library_folder_id));
    m.insert("path".into(), s(&item.path));
    m.insert("relPath".into(), s(&item.rel_path));
    m.insert("isFile".into(), b(item.is_file));
    put_some(&mut m, "mtimeMs", ms(&item.mtime).map(Value::from));
    put_some(&mut m, "ctimeMs", ms(&item.ctime).map(Value::from));
    put_some(&mut m, "birthtimeMs", ms(&item.birthtime).map(Value::from));
    m.insert("addedAt".into(), ms0(&item.created_at));
    m.insert("updatedAt".into(), ms0(&item.updated_at));
    m
}

/// `toOldJSON()` of a book item.
pub fn old_json(item: &LibraryItem, book: &BookFull) -> Value {
    let id = item.id.clone().unwrap_or_default();
    let mut m = base(item);
    put_some(&mut m, "lastScan", ms(&item.last_scan).map(Value::from));
    m.insert("scanVersion".into(), s(&item.last_scan_version));
    m.insert("isMissing".into(), Value::from(item.is_missing.unwrap_or(false)));
    m.insert("isInvalid".into(), Value::from(item.is_invalid.unwrap_or(false)));
    m.insert("mediaType".into(), s(&item.media_type));
    m.insert("media".into(), book.old_json(&id));
    m.insert("libraryFiles".into(), library_files_json(item));
    Value::Object(m)
}

/// `toOldJSONMinified()` of a book item.
pub fn minified(item: &LibraryItem, book: &BookFull, sorting: &Sorting) -> Map<String, Value> {
    let mut m = base(item);
    m.insert("isMissing".into(), Value::from(item.is_missing.unwrap_or(false)));
    m.insert("isInvalid".into(), Value::from(item.is_invalid.unwrap_or(false)));
    m.insert("mediaType".into(), s(&item.media_type));
    m.insert("media".into(), Value::Object(book.minified(sorting)));
    m.insert("numFiles".into(), Value::from(library_files(item).len()));
    m.insert("size".into(), item.size.map_or(Value::Null, Value::from));
    m
}

/// `toOldJSONExpanded()` of a book item.
pub fn expanded(item: &LibraryItem, book: &BookFull, sorting: &Sorting) -> Map<String, Value> {
    let id = item.id.clone().unwrap_or_default();
    let mut m = minified(item, book, sorting);
    put_some(&mut m, "lastScan", ms(&item.last_scan).map(Value::from));
    m.insert("scanVersion".into(), s(&item.last_scan_version));
    m.insert("media".into(), Value::Object(book.expanded(&id, sorting)));
    m.insert("libraryFiles".into(), library_files_json(item));
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_types_from_extensions() {
        assert_eq!(file_type(Some(".M4B")), "audio");
        assert_eq!(file_type(Some(".epub")), "ebook");
        assert_eq!(file_type(Some(".jpg")), "image");
        assert_eq!(file_type(Some(".opf")), "metadata");
        assert_eq!(file_type(Some(".nfo")), "text");
        assert_eq!(file_type(Some(".bin")), "unknown");
        assert_eq!(file_type(None), "unknown");
    }

    #[test]
    fn library_files_keep_only_known_keys() {
        let f = json!({"ino": "1", "metadata": {"filename": "a.mp3", "ext": ".mp3", "size": 3, "extra": 1}, "addedAt": 5});
        let j = library_file_json(&f);
        assert_eq!(j["metadata"], json!({"filename": "a.mp3", "ext": ".mp3", "size": 3}));
        assert_eq!(j["isSupplementary"], Value::Null);
        assert_eq!(j["fileType"], "audio");
        assert!(j.get("updatedAt").is_none());
    }
}
