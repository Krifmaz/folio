//! Other apps' protocols, so the apps people already use work with Folio: an OPDS catalogue for e-readers
//! ([`opds`]), a KOReader sync server ([`koreader`]), an Audiobookshelf-compatible API ([`abs`]) and Kobo sync
//! ([`kobo`]). Each protocol reads the library through [`shelf`] as [`model::Item`]s. See docs/COMPAT.md for
//! how to point each app at Folio.

pub mod abs;
pub mod epub;
pub mod kobo;
pub mod koreader;
pub mod model;
pub mod opds;
pub mod shelf;
pub mod xml;

use crate::AppState;
use axum::extract::Request;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Router;
use kit_server_core::{err, Migrations, Result, SqliteDb};
use std::path::Path;
use tower_http::services::ServeFile;

/// The compatibility track's tables, step by step (a released step is never changed).
pub const MIGRATIONS: Migrations = Migrations {
    component: "folio-compat",
    steps: &[
        // 1: KOReader: each account's sync password (SHA-256 of KOReader's MD5 of it), KOReader's own places by
        // its document name, and the MD5s KOReader names the library's ebook files by
        "CREATE TABLE compat_koreader_keys (
            user_id INTEGER PRIMARY KEY,
            key_hash TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE TABLE compat_koreader_progress (
            user_id INTEGER NOT NULL,
            document TEXT NOT NULL,
            progress TEXT NOT NULL DEFAULT '',
            percentage REAL NOT NULL DEFAULT 0,
            device TEXT NOT NULL DEFAULT '',
            device_id TEXT NOT NULL DEFAULT '',
            updated_at REAL NOT NULL,
            PRIMARY KEY (user_id, document)
        );
        CREATE TABLE compat_file_digests (
            file_id INTEGER PRIMARY KEY,
            size INTEGER NOT NULL,
            partial_md5 TEXT NOT NULL,
            name_md5 TEXT NOT NULL
        );
        CREATE INDEX compat_file_digests_partial ON compat_file_digests (partial_md5);",
        // 2: Audiobookshelf playback sessions: who plays what on which app, where it is and how long was heard
        "CREATE TABLE compat_abs_sessions (
            id TEXT PRIMARY KEY,
            user_id INTEGER NOT NULL,
            edition_id INTEGER NOT NULL,
            device TEXT NOT NULL DEFAULT '',
            started_at REAL NOT NULL,
            updated_at REAL NOT NULL,
            position REAL NOT NULL DEFAULT 0,
            listened REAL NOT NULL DEFAULT 0,
            closed INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX compat_abs_sessions_user ON compat_abs_sessions (user_id, started_at);",
        // 3: Kobo sync: each account's sync address token (kept to show it again, and as SHA-256 to look it up),
        // and the books removed on the Kobo, which aren't synced to it again
        "CREATE TABLE compat_kobo_tokens (
            token_key TEXT PRIMARY KEY,
            token TEXT NOT NULL,
            user_id INTEGER NOT NULL UNIQUE,
            created_at REAL NOT NULL
        );
        CREATE TABLE compat_kobo_removed (
            user_id INTEGER NOT NULL,
            edition_id INTEGER NOT NULL,
            removed_at REAL NOT NULL,
            PRIMARY KEY (user_id, edition_id)
        );",
    ],
};

/// Makes this part's tables (the steps not run yet).
pub fn migrate(db: &SqliteDb) -> anyhow::Result<()> {
    db.migrate(&MIGRATIONS).map_err(|e| anyhow::anyhow!("The tables for other apps can't be made: {}", e.detail))
}

/// This part's routes.
pub fn router() -> Router<AppState> {
    Router::new().merge(opds::router()).merge(koreader::router()).merge(abs::router()).merge(kobo::router())
}

/// Now, in seconds since 1970.
pub fn now() -> f64 {
    kit_server_core::db::now()
}

/// The file at `path` as the answer to `req`: its bytes with `mime`, ranges (206) and ETag handled; with
/// `download`, a `Content-Disposition` that names the file.
pub async fn send_file(path: &Path, mime: &str, download: Option<&str>, req: Request) -> Result<Response> {
    if !path.is_file() {
        return Err(err(404, "The book's file is missing from the library folder. Run the import again, or check the folder."));
    }
    let mime = HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    let mut r =
        ServeFile::new(path).try_call(req).await.map_err(|e| err(500, format!("The book's file can't be read: {e}")))?.into_response();
    if r.status().is_success() {
        r.headers_mut().insert(header::CONTENT_TYPE, mime);
    }
    if let Some(name) = download {
        let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() || c == ' ' { c } else { '_' }).collect();
        let ascii = ascii.replace('"', "'");
        let v = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", opds::urlencode(name));
        if let Ok(v) = HeaderValue::from_str(&v) {
            r.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(r)
}

/// HTML made into plain text: tags dropped, the common entities read, runs of space made one.
pub fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => {
                tag = true;
                out.push(' ');
            }
            '>' if tag => tag = false,
            c if !tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    #[test]
    fn html_becomes_plain_text() {
        assert_eq!(super::plain("<p>One &amp; <b>two</b></p>\n<p>three</p>"), "One & two three");
    }
}
