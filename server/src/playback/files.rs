//! An edition's files in the library (the `edition_files` table the import writes, docs/TRACKS.md) and sending
//! them to players and readers: whole or in parts (HTTP Range, so a player can jump anywhere in a long book), with
//! an ETag so a device that has a file already doesn't fetch it again.

use crate::AppState;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use kit_server_core::{err, p, Db, Result, Row, RowExt, SqliteDb};
use serde::Serialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use tower::ServiceExt;

/// One file of an edition, as the API lists it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileInfo {
    /// the file's id: `GET /api/files/{id}` sends it
    pub id: i64,
    /// the file name shown to people
    pub name: String,
    /// `audio`, `epub`, `pdf`, `azw3` or `cover`
    pub kind: String,
    /// the file's type, e.g. `audio/mp4`
    pub mime: String,
    /// bytes
    pub size: i64,
    /// the play order of an edition's audio files, from 0
    pub position: i64,
    /// an audio file's length in milliseconds (0 otherwise)
    pub duration_ms: i64,
    /// an audio file's chapters `[{title, start_ms, end_ms}]`, from the start of this file
    pub chapters: Value,
}

/// A file row as a [`FileInfo`].
fn info_of(r: &Row) -> FileInfo {
    FileInfo {
        id: r.i("id"),
        name: r.s("name"),
        kind: r.s("kind"),
        mime: r.s("mime"),
        size: r.i("size"),
        position: r.i("position"),
        duration_ms: r.i("duration_ms"),
        chapters: serde_json::from_str(&r.s("chapters")).unwrap_or(Value::Array(vec![])),
    }
}

/// The edition's files, audio in play order first.
pub fn edition_files(db: &SqliteDb, edition_id: i64) -> Result<Vec<FileInfo>> {
    let rows = db.query("SELECT * FROM edition_files WHERE edition_id=? ORDER BY kind <> 'audio', position, name, id", p![edition_id])?;
    Ok(rows.iter().map(info_of).collect())
}

/// Where the file `rel` (a path under the library folder) is on disk; `None` when it names a place outside the
/// library folder (an absolute path or `..`).
pub fn under_library(library: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.as_os_str().is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(library.join(rel))
}

/// The ETag of a file of `size` bytes last changed at `modified` (seconds), quoted as HTTP wants it.
pub fn etag(id: i64, size: u64, modified: u64) -> String {
    format!("\"{id}-{size:x}-{modified:x}\"")
}

/// `GET /api/files/{id}`: the file's bytes, whole or the part the Range header asks for, with its type and ETag.
pub async fn send(st: &AppState, id: i64, req: Request<Body>) -> Result<Response> {
    let row =
        st.db.one("SELECT * FROM edition_files WHERE id=?", p![id])?.ok_or_else(|| err(404, "That file isn't in the library any more."))?;
    let library = PathBuf::from(st.settings.str("library_dir"));
    let path = under_library(&library, &row.s("path"))
        .ok_or_else(|| err(500, "This file's place in the library is not a folder inside it, so it isn't sent. Import the book again."))?;
    let meta = tokio::fs::metadata(&path).await.map_err(|_| {
        err(404, format!("The file {} is missing from the library folder. Import the book again, or check the folder.", row.s("name")))
    })?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
    let tag = etag(id, meta.len(), modified);
    let same =
        req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).is_some_and(|v| v.split(',').any(|t| t.trim() == tag));
    if same {
        let mut r = StatusCode::NOT_MODIFIED.into_response();
        r.headers_mut().insert(header::ETAG, HeaderValue::from_str(&tag).unwrap_or(HeaderValue::from_static("\"\"")));
        return Ok(r);
    }
    let mut r = tower_http::services::ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|e| err(500, format!("The file can't be read: {e}")))?
        .map(Body::new);
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&tag) {
        h.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(&row.s("mime")) {
        if !row.s("mime").is_empty() {
            h.insert(header::CONTENT_TYPE, v);
        }
    }
    // the bytes go as they are: compressing them would cost time and break jumping (Range)
    h.insert(header::CONTENT_ENCODING, HeaderValue::from_static("identity"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    let name: String = row.s("name").chars().filter(|c| c.is_ascii_graphic() && *c != '"' && *c != '\\' || *c == ' ').collect();
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{name}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_inside_the_library_are_sent() {
        let lib = Path::new("/library");
        assert_eq!(under_library(lib, "Andy Weir/Book (2021)/Book.m4b"), Some(PathBuf::from("/library/Andy Weir/Book (2021)/Book.m4b")));
        assert_eq!(under_library(lib, "../etc/passwd"), None);
        assert_eq!(under_library(lib, "a/../../b"), None);
        assert_eq!(under_library(lib, "/etc/passwd"), None);
        assert_eq!(under_library(lib, ""), None);
    }

    #[test]
    fn etags_change_with_the_file() {
        assert_ne!(etag(1, 10, 5), etag(1, 11, 5));
        assert_ne!(etag(1, 10, 5), etag(1, 10, 6));
        assert!(etag(1, 10, 5).starts_with('"'));
    }
}
