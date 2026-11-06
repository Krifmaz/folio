//! The port of the original's upload (`MiscController.handleUpload`, `POST /api/upload`): users who may upload
//! send audio or e-book files from the browser, and they are put in a library folder under
//! `author/series/title` (or `title` in a podcast library), each part made safe for file systems. Answers are the
//! original's (tests/abs_upload.rs).

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::file_utils::sanitize_filename;
use crate::abs::models::{self, user, Library, LibraryFolder};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// The route; uploads may be of any size, like the original's.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/upload", post(upload).layer(DefaultBodyLimit::disable()))
}

/// A file received, kept in the temporary folder until it is put in place.
struct Received {
    /// The file's name as the browser sent it.
    name: String,
    /// Where it waits (`<metadata>/tmp`, like the original's `tempFileDir`).
    temp: PathBuf,
}

/// Removes the waiting files of a refused upload.
fn forget(files: &[Received]) {
    for f in files {
        if let Err(e) = std::fs::remove_file(&f.temp) {
            tracing::error!("The uploaded file {} can't be removed from {}: {e}", f.name, f.temp.display());
        }
    }
}

/// Reads the form: its text fields (a field sent twice becomes a list, like the original's form reader) and its
/// files, saved to the temporary folder as they arrive.
async fn read_form(mut mp: Multipart, tmp: &Path) -> std::result::Result<(Map<String, Value>, Vec<Received>), (Vec<Received>, String)> {
    let mut fields = Map::new();
    let mut files: Vec<Received> = Vec::new();
    loop {
        let mut field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => return Ok((fields, files)),
            Err(e) => return Err((files, e.to_string())),
        };
        let field_name = field.name().unwrap_or_default().to_string();
        if let Some(name) = field.file_name().map(String::from) {
            if let Err(e) = std::fs::create_dir_all(tmp) {
                return Err((files, format!("The folder {} can't be made: {e}", tmp.display())));
            }
            let temp = tmp.join(format!("tmp-{}", crate::abs::models::new_id()));
            let mut out = match tokio::fs::File::create(&temp).await {
                Ok(f) => f,
                Err(e) => return Err((files, format!("{} can't be written: {e}", temp.display()))),
            };
            files.push(Received { name, temp });
            loop {
                match field.chunk().await {
                    Ok(Some(chunk)) => {
                        if let Err(e) = out.write_all(&chunk).await {
                            return Err((files, e.to_string()));
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return Err((files, e.to_string())),
                }
            }
            if let Err(e) = out.flush().await {
                return Err((files, e.to_string()));
            }
        } else {
            let text = match field.text().await {
                Ok(t) => t,
                Err(e) => return Err((files, e.to_string())),
            };
            match fields.get_mut(&field_name) {
                Some(Value::Array(list)) => list.push(json!(text)),
                Some(v) => *v = json!([v.clone(), text]),
                None => {
                    fields.insert(field_name, json!(text));
                }
            }
        }
    }
}

/// A text field, when it holds non-empty text (the original's `typeof … === 'string'` and truthy checks).
fn text<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    fields.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `POST /api/upload`: puts the uploaded files in `<folder>/<author>/<series>/<title>` of a library (only
/// `<title>` in a podcast library). Needs the upload permission and access to the library.
async fn upload(State(st): State<AppState>, me: AbsUser, multipart: Option<Multipart>) -> Response {
    if !user::can(&me.user, "upload") {
        tracing::warn!("User \"{}\" attempted to upload without permission", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    let tmp = st.abs_server.metadata_path.join("tmp");
    let (fields, files) = match multipart {
        Some(mp) => match read_form(mp, &tmp).await {
            Ok(read) => read,
            Err((files, e)) => {
                tracing::error!("The upload can't be read: {e}");
                forget(&files);
                return send_status(StatusCode::BAD_REQUEST);
            }
        },
        None => (Map::new(), Vec::new()),
    };
    if files.is_empty() {
        tracing::error!("Invalid request, no files");
        return send_status(StatusCode::BAD_REQUEST);
    }
    let refuse = |files: &[Received], r: Response| {
        forget(files);
        r
    };
    let (Some(library_id), Some(folder_id), Some(title)) = (text(&fields, "library"), text(&fields, "folder"), text(&fields, "title"))
    else {
        return refuse(&files, send_text(StatusCode::BAD_REQUEST, "Invalid request body"));
    };
    let series = text(&fields, "series");
    let author = text(&fields, "author");
    let library = match models::find::<Library>(&st.abs, library_id) {
        Ok(Some(l)) => l,
        Ok(None) => return refuse(&files, send_text(StatusCode::NOT_FOUND, "Library not found")),
        Err(e) => return refuse(&files, e.into_response()),
    };
    if !user::can_access_library(&me.user, library_id) {
        tracing::error!(
            "User \"{}\" attempting to upload to library \"{library_id}\" without access",
            me.user.username.as_deref().unwrap_or_default()
        );
        return refuse(&files, send_status(StatusCode::FORBIDDEN));
    }
    let folder = match models::list::<LibraryFolder>(&st.abs, "libraryId = ? AND id = ?", "", vec![json!(library_id), json!(folder_id)]) {
        Ok(mut found) if !found.is_empty() => found.remove(0),
        Ok(_) => return refuse(&files, send_text(StatusCode::NOT_FOUND, "Folder not found")),
        Err(e) => return refuse(&files, e.into_response()),
    };
    let parts: Vec<&str> = if library.media_type.as_deref() == Some("podcast") {
        vec![title]
    } else {
        [author, series, Some(title)].into_iter().flatten().collect()
    };
    let mut dir = PathBuf::from(folder.path.unwrap_or_default());
    for part in parts {
        dir.push(sanitize_filename(part));
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!("Failed to create upload directory {}: {e}", dir.display());
        return refuse(&files, send_status(StatusCode::INTERNAL_SERVER_ERROR));
    }
    tracing::info!("Uploading {} files to {}", files.len(), dir.display());
    for f in &files {
        let to = dir.join(sanitize_filename(&f.name));
        if let Err(e) = put(&f.temp, &to) {
            tracing::error!("Failed to move file {}: {e}", to.display());
            forget(std::slice::from_ref(f));
        }
    }
    send_status(StatusCode::OK)
}

/// Moves a file into place, copying it when the two folders are on different disks.
fn put(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}
