//! A KOReader sync server ("Progress sync" in KOReader), so KOReader keeps its place in step with Folio's web
//! reader, the Android app and other KOReader devices. Written from the public description of KOReader's
//! sync protocol (koreader/koreader-sync-server's API, version 1).
//!
//! In KOReader: *Tools → Progress sync → Custom sync server* `https://<folio>/koreader`, then *Login* with the
//! Folio username and the KOReader sync password set in Folio (KOReader only ever sends an MD5 of a password,
//! which Folio's own password can't be checked against).
//!
//! ```text
//! PUT    /koreader/key                {password}: (signed in to Folio) set the KOReader sync password
//! GET    /koreader/key                {set}: whether one is set
//! DELETE /koreader/key                forget it (KOReader can't sync until a new one is set)
//! POST   /koreader/users/create       KOReader's Register: refused, with where to set the password instead
//! GET    /koreader/users/auth         x-auth-user, x-auth-key → {authorized: "OK"} or 401
//! PUT    /koreader/syncs/progress     {document, progress, percentage, device, device_id} → {document, timestamp}
//! GET    /koreader/syncs/progress/{document}  the newest place: {document, progress, percentage, device, device_id, timestamp}, or {}
//! GET    /koreader/healthcheck        {state: "OK"}
//! ```
//!
//! KOReader names a book by an MD5 of samples of its file (or of its file name). Folio works those out for each
//! ebook file in the library, so a place KOReader sends is also the book's place in Folio, and a place another
//! app sends is answered to KOReader as the start of the same chapter (`/body/DocFragment[n]/body`).

use super::shelf;
use crate::playback::store as playback;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SignedIn};
use md5::{Digest as _, Md5};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::io::{Read, Seek, SeekFrom};

/// The routes KOReader and Folio's web app use.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/koreader/key", get(key_state).put(set_key).delete(forget_key))
        .route("/koreader/users/create", post(register))
        .route("/koreader/users/auth", get(authorize))
        .route("/koreader/syncs/progress", put(push))
        .route("/koreader/syncs/progress/{document}", get(pull))
        .route("/koreader/healthcheck", get(|| async { Json(json!({ "state": "OK" })) }))
}

/// A KOReader answer that isn't a success: the status, KOReader's own error code and a plain sentence (as
/// `message` for KOReader and `detail` for everything else).
fn refuse(status: u16, code: u32, sentence: &str) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    (status, Json(json!({ "code": code, "message": sentence, "detail": sentence }))).into_response()
}

/// How a KOReader key is kept: SHA-256 of the MD5 KOReader sends, in hex.
fn key_hash(md5_hex: &str) -> String {
    hex::encode(Sha256::digest(md5_hex.trim().to_lowercase()))
}

/// `PUT /koreader/key` body.
#[derive(Debug, Deserialize)]
pub struct NewKey {
    /// the KOReader sync password, as the person will type it in KOReader
    pub password: String,
}

/// `GET /koreader/key`.
async fn key_state(State(st): State<AppState>, SignedIn(me): SignedIn) -> Result<Json<Value>> {
    let set = st.db.one("SELECT user_id FROM compat_koreader_keys WHERE user_id=?", p![me.id])?.is_some();
    Ok(Json(json!({ "set": set, "username": me.username })))
}

/// `PUT /koreader/key`: set the KOReader sync password (at least 8 characters).
async fn set_key(State(st): State<AppState>, SignedIn(me): SignedIn, Json(b): Json<NewKey>) -> Result<Json<Value>> {
    if b.password.chars().count() < 8 {
        return Err(err(400, "Use at least 8 characters for the KOReader sync password."));
    }
    let md5 = hex::encode(Md5::digest(b.password.as_bytes()));
    st.db.execute(
        "INSERT INTO compat_koreader_keys (user_id, key_hash, created_at) VALUES (?,?,?)
         ON CONFLICT (user_id) DO UPDATE SET key_hash=excluded.key_hash, created_at=excluded.created_at",
        p![me.id, key_hash(&md5), super::now()],
    )?;
    Ok(Json(json!({ "set": true, "username": me.username })))
}

/// `DELETE /koreader/key`.
async fn forget_key(State(st): State<AppState>, SignedIn(me): SignedIn) -> Result<Json<Value>> {
    st.db.execute("DELETE FROM compat_koreader_keys WHERE user_id=?", p![me.id])?;
    Ok(Json(json!({ "set": false, "username": me.username })))
}

/// `POST /koreader/users/create`: KOReader's Register button. Accounts come from Folio, so this says where to
/// set the password instead.
async fn register() -> Response {
    refuse(
        402,
        2002,
        "Folio makes accounts itself. Sign in to Folio, set a KOReader sync password under Other apps, then use Login here with your Folio username.",
    )
}

/// The account KOReader's `x-auth-user` and `x-auth-key` headers sign in as; `None` when they are wrong or there
/// were 10 wrong tries for that username in 5 minutes.
fn koreader_user(st: &AppState, headers: &HeaderMap) -> Option<i64> {
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let (user, key) = (h("x-auth-user")?, h("x-auth-key")?);
    let limit = format!("koreader-login:{}", user.trim().to_lowercase());
    if st.limits.limited(&limit, 10, 300.0, false) {
        return None;
    }
    let row = st
        .db
        .one(
            "SELECT u.id, k.key_hash FROM users u JOIN compat_koreader_keys k ON k.user_id=u.id WHERE u.username=? COLLATE NOCASE AND u.disabled=0",
            p![user.trim()],
        )
        .ok()??;
    let want = row.s("key_hash");
    let got = key_hash(&key);
    if want.len() == got.len() && want.bytes().zip(got.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0 {
        Some(row.i("id"))
    } else {
        st.limits.record(&limit);
        None
    }
}

/// The answer when KOReader's sign-in is wrong.
fn unauthorized() -> Response {
    refuse(
        401,
        2001,
        "The username or KOReader sync password is wrong. Check them in KOReader, or set the password again in Folio under Other apps.",
    )
}

/// `GET /koreader/users/auth`.
async fn authorize(State(st): State<AppState>, headers: HeaderMap) -> Response {
    match koreader_user(&st, &headers) {
        Some(_) => Json(json!({ "authorized": "OK" })).into_response(),
        None => unauthorized(),
    }
}

/// `PUT /koreader/syncs/progress` body (KOReader's names).
#[derive(Debug, Deserialize, Default)]
pub struct Place {
    /// KOReader's name for the book: an MD5 of samples of the file, or of the file name
    #[serde(default)]
    pub document: String,
    /// where: an XPointer (`/body/DocFragment[12]/body/p[3]/text().0`) for EPUBs, a page number for PDFs
    #[serde(default)]
    pub progress: Value,
    /// how far, 0 to 1
    #[serde(default)]
    pub percentage: f64,
    /// the device's name
    #[serde(default)]
    pub device: String,
    /// the device's id
    #[serde(default)]
    pub device_id: String,
}

/// Whether `s` is a KOReader document name: 32 hex digits.
fn is_document(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `progress` as text (KOReader sends a string; a page number may come as a number).
fn progress_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.chars().take(2000).collect(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `PUT /koreader/syncs/progress`: keep KOReader's place, and when the book is in the library, make it the
/// book's place in Folio too.
async fn push(State(st): State<AppState>, headers: HeaderMap, Json(b): Json<Place>) -> Result<Response> {
    let Some(user) = koreader_user(&st, &headers) else { return Ok(unauthorized()) };
    let document = b.document.trim().to_lowercase();
    if !is_document(&document) {
        return Ok(refuse(403, 2003, "KOReader didn't say which book this is. Update KOReader and try again."));
    }
    if !b.percentage.is_finite() || !(0.0..=1.0).contains(&b.percentage) {
        return Ok(refuse(403, 2003, "KOReader sent a place outside the book. Turn a page and sync again."));
    }
    let progress = progress_text(&b.progress);
    let device: String = b.device.chars().take(120).collect();
    let device_id: String = b.device_id.chars().take(120).collect();
    let at = super::now();
    st.db.execute(
        "INSERT INTO compat_koreader_progress (user_id, document, progress, percentage, device, device_id, updated_at) VALUES (?,?,?,?,?,?,?)
         ON CONFLICT (user_id, document) DO UPDATE SET progress=excluded.progress, percentage=excluded.percentage,
         device=excluded.device, device_id=excluded.device_id, updated_at=excluded.updated_at",
        p![user, document, progress, b.percentage, device, device_id, at],
    )?;
    if let Some((edition, file)) = edition_of(&st, &document)? {
        let mut locator = json!({ "total_progression": b.percentage, "koreader": progress, "koreader_device_id": device_id });
        if let Some(i) = fragment(&progress) {
            if let Some(href) = super::epub::spine(&file).and_then(|s| s.get(i).cloned()) {
                locator["href"] = json!(href);
                locator["progression"] = json!(0.0);
            }
        }
        let kept = playback::progress(&st.db, user, edition)?;
        let place = playback::Progress {
            edition_id: edition,
            position_ms: 0,
            locator,
            fraction: b.percentage,
            finished: b.percentage >= 0.995 || kept.is_some_and(|k| k.finished),
            device: format!("koreader:{}", if device.is_empty() { "KOReader" } else { &device }),
            updated_at: at,
        };
        playback::save_progress(&st.db, user, edition, place)?;
    }
    Ok(Json(json!({ "document": document, "timestamp": at as i64 })).into_response())
}

/// The part of the book an XPointer is in, from 0 (`/body/DocFragment[12]/…` → 11).
pub fn fragment(xpointer: &str) -> Option<usize> {
    let rest = xpointer.split("DocFragment[").nth(1)?;
    let n: usize = rest.split(']').next()?.parse().ok()?;
    n.checked_sub(1)
}

/// `GET /koreader/syncs/progress/{document}`: the newest place in the book, KOReader's own or another app's.
async fn pull(State(st): State<AppState>, headers: HeaderMap, Path(document): Path<String>) -> Result<Response> {
    let Some(user) = koreader_user(&st, &headers) else { return Ok(unauthorized()) };
    let document = document.trim().to_lowercase();
    if !is_document(&document) {
        return Ok(refuse(403, 2003, "KOReader didn't say which book this is. Update KOReader and try again."));
    }
    let own = st.db.one("SELECT * FROM compat_koreader_progress WHERE user_id=? AND document=?", p![user, document])?;
    let mut answer = own.as_ref().map(|r| {
        json!({
            "document": document,
            "progress": r.s("progress"),
            "percentage": r.f("percentage"),
            "device": r.s("device"),
            "device_id": r.s("device_id"),
            "timestamp": r.f("updated_at") as i64,
        })
    });
    let own_at = own.as_ref().map(|r| r.f("updated_at")).unwrap_or(f64::MIN);
    if let Some((edition, file)) = edition_of(&st, &document)? {
        if let Some(p) = playback::progress(&st.db, user, edition)? {
            if p.updated_at > own_at + 0.5 {
                if let Some(xp) = xpointer_of(&p.locator, &file) {
                    answer = Some(json!({
                        "document": document,
                        "progress": xp,
                        "percentage": p.fraction,
                        "device": if p.device.is_empty() { "Folio".to_string() } else { p.device.clone() },
                        "device_id": format!("folio:{}", p.device),
                        "timestamp": p.updated_at as i64,
                    }));
                }
            }
        }
    }
    Ok(Json(answer.unwrap_or_else(|| json!({}))).into_response())
}

/// A place another app wrote, as an XPointer KOReader can go to: KOReader's own when it wrote it, else the start
/// of the spine item the place is in.
fn xpointer_of(locator: &Value, file: &std::path::Path) -> Option<String> {
    if let Some(x) = locator.get("koreader").and_then(Value::as_str).filter(|x| !x.is_empty()) {
        return Some(x.to_string());
    }
    let href = locator.get("href").and_then(Value::as_str)?;
    let i = super::epub::spine_index(&super::epub::spine(file)?, href)?;
    Some(format!("/body/DocFragment[{}]/body", i + 1))
}

/// KOReader's sampled MD5 of a file: 1024 bytes at 0, 1 KiB, 4 KiB, 16 KiB … 1 GiB (each 4 times further),
/// stopping at the end of the file.
pub fn partial_md5(path: &std::path::Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Md5::new();
    let mut buf = vec![0u8; 1024];
    for i in -1i32..=10 {
        let at: u64 = if i < 0 { 0 } else { 1024u64 << (2 * i as u64) };
        f.seek(SeekFrom::Start(at)).ok()?;
        let n = read_up_to(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

/// Reads until `buf` is full or the file ends; how many bytes.
fn read_up_to(f: &mut std::fs::File, buf: &mut [u8]) -> Option<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(n)
}

/// The edition KOReader's `document` is, and its file: the ebook file whose sampled MD5 or file-name MD5 it
/// is. The MD5s are worked out once per file (again when its size changes) and kept in
/// `compat_file_digests`.
pub fn edition_of(st: &AppState, document: &str) -> Result<Option<(i64, std::path::PathBuf)>> {
    let root = shelf::library_dir(st);
    let files = st.db.query(
        "SELECT f.id, f.edition_id, f.path, f.name, f.size, d.size AS dsize, d.partial_md5, d.name_md5
         FROM edition_files f LEFT JOIN compat_file_digests d ON d.file_id=f.id WHERE f.kind IN ('epub','pdf','azw3')",
        p![],
    )?;
    let mut found = None;
    for r in files {
        let path = root.join(r.s("path"));
        let (mut partial, mut name) = (r.s("partial_md5"), r.s("name_md5"));
        if r.s("partial_md5").is_empty() || r.i("dsize") != r.i("size") {
            let Some(digest) = partial_md5(&path) else { continue };
            partial = digest;
            name = hex::encode(Md5::digest(r.s("name").as_bytes()));
            st.db.execute(
                "INSERT INTO compat_file_digests (file_id, size, partial_md5, name_md5) VALUES (?,?,?,?)
                 ON CONFLICT (file_id) DO UPDATE SET size=excluded.size, partial_md5=excluded.partial_md5, name_md5=excluded.name_md5",
                p![r.i("id"), r.i("size"), partial, name],
            )?;
        }
        if found.is_none() && (partial == document || name == document) {
            found = Some((r.i("edition_id"), path));
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpointers_name_their_part() {
        assert_eq!(fragment("/body/DocFragment[12]/body/div/p[3]/text().0"), Some(11));
        assert_eq!(fragment("/body/DocFragment[0]/body"), None);
        assert_eq!(fragment("42"), None);
    }

    #[test]
    fn the_sampled_md5_reads_koreaders_pieces() {
        let p = std::env::temp_dir().join(format!("folio-kosync-md5-{}", std::process::id()));
        let bytes: Vec<u8> = (0..5000u32).map(|i| (i % 256) as u8).collect();
        std::fs::write(&p, &bytes).unwrap();
        // 0..1024, 1024..2048, 4096..5000 (16 KiB is past the end)
        let mut h = Md5::new();
        h.update(&bytes[0..1024]);
        h.update(&bytes[1024..2048]);
        h.update(&bytes[4096..5000]);
        assert_eq!(partial_md5(&p), Some(hex::encode(h.finalize())));
        assert!(is_document(&partial_md5(&p).unwrap()));
    }
}
