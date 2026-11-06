//! An item's ebook files (`LibraryItemController.getEBookFile` and `updateEbookFileStatus`, PARITY A0053 and
//! A0054): the file the reader opens, and switching which ebook file is the book's main one.

use crate::abs::auth::{send_status, AbsUser};
use crate::abs::library::util::Sorting;
use crate::abs::models::{self, library_item, user};
use crate::abs::progress::send_text;
use crate::AppState;
use axum::extract::{Path, Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::Router;
use serde_json::{Map, Value};

/// The ebook routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/items/{id}/ebook", get(main_file))
        .route("/api/items/{id}/ebook/{file}", get(one_file))
        .route("/api/items/{id}/ebook/{file}/status", patch(switch))
}

/// The item's library files (`libraryFiles`).
fn library_files(item: &models::LibraryItem) -> Vec<Value> {
    item.library_files.as_ref().and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The library file with this inode number (`getLibraryFileWithIno`).
fn file_with_ino(item: &models::LibraryItem, ino: &str) -> Option<Value> {
    library_files(item).into_iter().find(|f| f.get("ino").and_then(Value::as_str) == Some(ino))
}

/// Whether a library file is an ebook (`isEBookFile`).
fn is_ebook(f: &Value) -> bool {
    f.get("fileType").and_then(Value::as_str) == Some("ebook")
}

/// `GET /api/items/:id/ebook`: the book's main ebook file.
async fn main_file(State(st): State<AppState>, me: AbsUser, Path(id): Path<String>, req: Request) -> Response {
    send_ebook(&st, &me, &id, None, req).await
}

/// `GET /api/items/:id/ebook/:fileid`: one of the item's ebook files.
async fn one_file(State(st): State<AppState>, me: AbsUser, Path((id, file)): Path<(String, String)>, req: Request) -> Response {
    send_ebook(&st, &me, &id, Some(&file), req).await
}

/// Sends an ebook file with the type the original sends for its extension.
async fn send_ebook(st: &AppState, me: &AbsUser, id: &str, ino: Option<&str>, req: Request) -> Response {
    let (item, b) = match super::item_for(st, &me.user, id, &Method::GET) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let file = match ino {
        Some(ino) => {
            let Some(f) = file_with_ino(&item, ino) else {
                tracing::error!("Item {id} has no file {ino}");
                return send_status(StatusCode::NOT_FOUND);
            };
            if !is_ebook(&f) {
                tracing::error!("The file {ino} of item {id} isn't an ebook");
                return send_text(StatusCode::BAD_REQUEST, "Invalid ebook file id");
            }
            f
        }
        None => match b.book.ebook_file.clone().filter(Value::is_object) {
            Some(f) => f,
            None => {
                tracing::error!("Item {id} has no ebook file");
                return send_status(StatusCode::NOT_FOUND);
            }
        },
    };
    let path = file.pointer("/metadata/path").and_then(Value::as_str).unwrap_or_default().to_string();
    tracing::info!("{} opened the ebook {path} of item {id}", me.user.username.as_deref().unwrap_or_default());
    if path.is_empty() || tokio::fs::metadata(&path).await.is_err() {
        tracing::error!("The ebook file {path} of item {id} is missing");
        return send_text(StatusCode::NOT_FOUND, "File not found");
    }
    use tower::ServiceExt;
    let mut r = match tower_http::services::ServeFile::new(&path).oneshot(req).await {
        Ok(r) => r.map(axum::body::Body::new),
        Err(e) => {
            tracing::error!("The ebook file {path} couldn't be read: {e}");
            return send_text(StatusCode::INTERNAL_SERVER_ERROR, "Download failed");
        }
    };
    if let Some(t) = ebook_type(&path) {
        r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(t));
    }
    r.headers_mut().insert(header::CONTENT_ENCODING, HeaderValue::from_static("identity"));
    r
}

/// The type Express sends for an ebook's extension.
fn ebook_type(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path).extension()?.to_str()?.to_lowercase();
    Some(match ext.as_str() {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw3" => "application/vnd.amazon.mobi8-ebook",
        "cbz" => "application/vnd.comicbook+zip",
        "cbr" => "application/vnd.comicbook-rar",
        _ => return None,
    })
}

/// `PATCH /api/items/:id/ebook/:fileid/status` (A0054): makes a supplementary ebook file the book's main one, or
/// the main one supplementary (the book then has no main ebook).
async fn switch(State(st): State<AppState>, me: AbsUser, Path((id, ino)): Path<(String, String)>) -> Response {
    let (mut item, b) = match super::item_for(&st, &me.user, &id, &Method::GET) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let Some(file) = file_with_ino(&item, &ino) else {
        tracing::error!("Item {id} has no file {ino}");
        return send_status(StatusCode::NOT_FOUND);
    };
    if !user::can(&me.user, "update") {
        tracing::warn!("{} tried to change an item without permission", me.user.username.as_deref().unwrap_or_default());
        return send_status(StatusCode::FORBIDDEN);
    }
    if !is_ebook(&file) {
        tracing::error!("The file {ino} of item {id} isn't an ebook");
        return send_text(StatusCode::BAD_REQUEST, "Invalid ebook file id");
    }
    let mut book = b.book.clone();
    let primary = if file.get("isSupplementary").and_then(Value::as_bool).unwrap_or(false) {
        tracing::info!("Making the ebook file {ino} of item {id} the main one");
        let mut m: Map<String, Value> = file.as_object().cloned().unwrap_or_default();
        m.remove("isSupplementary");
        m.remove("fileType");
        let ext = file.pointer("/metadata/ext").and_then(Value::as_str).unwrap_or_default();
        m.insert("ebookFormat".into(), Value::from(ext.strip_prefix('.').unwrap_or(ext).to_lowercase()));
        Some(Value::Object(m))
    } else {
        tracing::info!("Making the ebook file {ino} of item {id} supplementary");
        None
    };
    let primary_ino = primary.as_ref().and_then(|p| p.get("ino")).and_then(Value::as_str).map(String::from);
    book.ebook_file = primary;
    book.updated_at = Some(models::date_now());
    let files: Vec<Value> = library_files(&item)
        .into_iter()
        .map(|mut f| {
            if is_ebook(&f) {
                let other = f.get("ino").and_then(Value::as_str).map(String::from) != primary_ino;
                f["isSupplementary"] = Value::Bool(other);
            }
            f
        })
        .collect();
    item.library_files = Some(Value::Array(files));
    let audio = book.audio_files.as_ref().and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    item.is_missing = Some(!(audio || book.ebook_file.is_some()));
    item.updated_at = Some(models::date_now());
    let saved = models::update(&st.abs, &book).and_then(|_| models::update(&st.abs, &item));
    if let Err(e) = saved {
        return e.into_response();
    }
    // The item as it is now, for the apps of everyone who may see it.
    match super::expanded_by_id(&st.abs, &id) {
        Ok(Some((item, b))) => {
            let json = Value::Object(library_item::expanded(&item, &b, &Sorting::of(&st.abs_server.settings())));
            st.sockets.library_item_emitter("item_updated", &json, &|u| super::can_access(u, &item, &b));
        }
        Ok(None) => {}
        Err(e) => return e.into_response(),
    }
    send_status(StatusCode::OK)
}
