//! Reading the library for the outside apps: every edition that has files in the library folder, as an
//! [`Item`], and who is asking ([`Who`]): the web app's session, an app token, the API key, or a username and
//! password sent the HTTP Basic way (how e-readers sign in to OPDS).
//!
//! Every account sees the whole library for now (docs/TRACKS.md); when per-user libraries come, the filter goes
//! into [`items`] and every protocol follows.

use super::model::{File, FileChapter, Item};
use crate::metadata::EditionMeta;
use crate::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use kit_server_core::{p, Db, Result, Row, RowExt};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// How long a username and password that were right are remembered, so each feed page doesn't wait for bcrypt.
const REMEMBER_SECS: f64 = 600.0;

/// The account a request comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    /// the account's `users.id`
    pub id: i64,
    /// the account's username
    pub username: String,
}

/// The answer when nobody is signed in: 401 with `WWW-Authenticate: Basic`, so a reader asks for the username
/// and password.
#[derive(Debug)]
pub struct SignIn(pub &'static str);

impl IntoResponse for SignIn {
    fn into_response(self) -> Response {
        let mut r = (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "detail": self.0 }))).into_response();
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"Folio\", charset=\"UTF-8\""));
        r
    }
}

impl FromRequestParts<AppState> for Who {
    type Rejection = SignIn;

    async fn from_request_parts(parts: &mut Parts, st: &AppState) -> std::result::Result<Who, SignIn> {
        if let Some((user, pass)) = basic(&parts.headers) {
            return password_user(st, &user, &pass).await.ok_or(SignIn("The username or password is wrong. Check them and try again."));
        }
        match crate::auth::signed_in(st, parts) {
            Ok(a) => Ok(Who { id: a.id, username: a.username }),
            Err(_) => Err(SignIn("Sign in with your Folio username and password.")),
        }
    }
}

/// The username and password of an `Authorization: Basic` header.
pub fn basic(headers: &axum::http::HeaderMap) -> Option<(String, String)> {
    use base64::Engine;
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, rest) = v.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let raw = base64::engine::general_purpose::STANDARD.decode(rest.trim()).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (u, p) = s.split_once(':')?;
    Some((u.to_string(), p.to_string()))
}

/// The usernames and passwords found right lately: key (SHA-256 of both) → (account, until when).
fn remembered() -> &'static Mutex<HashMap<String, (Who, f64)>> {
    static R: OnceLock<Mutex<HashMap<String, (Who, f64)>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// The account a username and password sign in as; `None` when they are wrong, the account is disabled, or there
/// were 10 wrong tries for this username in 5 minutes.
pub async fn password_user(st: &AppState, username: &str, password: &str) -> Option<Who> {
    let key = hex::encode(Sha256::digest(format!("{}\u{0}{password}", username.trim().to_lowercase())));
    let now = super::now();
    if let Some((who, until)) = remembered().lock().ok()?.get(&key).cloned() {
        if until > now && enabled(st, who.id) {
            return Some(who);
        }
    }
    let limit = format!("compat-login:{}", username.trim().to_lowercase());
    if st.limits.limited(&limit, 10, 300.0, false) {
        return None;
    }
    let row = st.db.one("SELECT * FROM users WHERE username=? COLLATE NOCASE AND disabled=0", p![username.trim()]).ok()??;
    let (pw, hash) = (password.to_string(), row.s("password_hash"));
    let ok = tokio::task::spawn_blocking(move || crate::auth::verify_password(&pw, &hash)).await.unwrap_or(false);
    if !ok {
        st.limits.record(&limit);
        return None;
    }
    let who = Who { id: row.i("id"), username: row.s("username") };
    if let Ok(mut r) = remembered().lock() {
        r.retain(|_, (_, until)| *until > now);
        r.insert(key, (who.clone(), now + REMEMBER_SECS));
    }
    Some(who)
}

/// Whether account `id` is still there and not disabled.
pub fn enabled(st: &AppState, id: i64) -> bool {
    matches!(st.db.one("SELECT id FROM users WHERE id=? AND disabled=0", p![id]), Ok(Some(_)))
}

/// The library folder (setting `library_dir`).
pub fn library_dir(st: &AppState) -> PathBuf {
    PathBuf::from(st.settings.str("library_dir"))
}

/// A row of `edition_files` as a [`File`], its path under the library folder `root`.
fn file_of(root: &std::path::Path, r: &Row) -> File {
    let chapters: Vec<FileChapter> = serde_json::from_str(&r.s("chapters")).unwrap_or_default();
    File {
        id: r.i("id"),
        path: root.join(r.s("path")),
        name: r.s("name"),
        kind: r.s("kind"),
        mime: r.s("mime"),
        size: r.i("size").max(0) as u64,
        duration: r.i("duration_ms").max(0) as f64 / 1000.0,
        chapters,
        added_at: r.f("created_at"),
    }
}

/// The items for the rows of `editions` and the files of those editions (ordered by edition, then position).
fn assemble(root: &std::path::Path, editions: Vec<Row>, files: Vec<Row>) -> Vec<Item> {
    let mut by_edition: HashMap<i64, Vec<Row>> = HashMap::new();
    for f in files {
        by_edition.entry(f.i("edition_id")).or_default().push(f);
    }
    editions
        .into_iter()
        .filter_map(|e| {
            let rows = by_edition.remove(&e.i("id"))?;
            let meta: EditionMeta = serde_json::from_str(&e.s("meta")).ok()?;
            let mut item = Item { id: e.i("id"), work_id: e.i("work_id"), meta, updated_at: e.f("refreshed_at"), ..Item::default() };
            for r in &rows {
                let f = file_of(root, r);
                if f.kind == "cover" {
                    item.cover = Some((f.path, f.mime));
                } else {
                    item.files.push(f);
                }
            }
            if item.files.is_empty() {
                return None;
            }
            item.added_at = item.files.iter().map(|f| f.added_at).fold(f64::MAX, f64::min);
            item.updated_at = item.updated_at.max(item.files.iter().map(|f| f.added_at).fold(0.0, f64::max));
            Some(item)
        })
        .collect()
}

/// Every item `who` can see: each edition with at least one file in the library.
pub fn items(st: &AppState, _who: &Who) -> Result<Vec<Item>> {
    let editions = st.db.query("SELECT * FROM editions WHERE id IN (SELECT edition_id FROM edition_files) ORDER BY id", p![])?;
    let files = st.db.query("SELECT * FROM edition_files ORDER BY edition_id, position, id", p![])?;
    Ok(assemble(&library_dir(st), editions, files))
}

/// The item `id`, when `who` can see it (404 otherwise).
pub fn item(st: &AppState, _who: &Who, id: i64) -> Result<Item> {
    let editions = st.db.query("SELECT * FROM editions WHERE id=?", p![id])?;
    let files = st.db.query("SELECT * FROM edition_files WHERE edition_id=? ORDER BY position, id", p![id])?;
    assemble(&library_dir(st), editions, files)
        .pop()
        .ok_or_else(|| kit_server_core::err(404, "That book isn't in the library, or it has no files yet."))
}
