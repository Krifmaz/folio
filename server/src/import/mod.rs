//! The import: a verified download is filed in the library and cleaned up on the way.
//!
//! - Its place is `Author/Series/NN - Title (Year)/` under the library folder ([`layout`]).
//! - Audiobooks become one chaptered M4B with the edition's tags, chapters (from Audnexus when they fit) and
//!   cover ([`audio`]). Several MP3s are joined into it.
//! - EPUBs get the edition's title, authors, language, ISBN, series and (when missing) cover ([`epub`]).
//! - With "Clean up files" off, the downloaded files are hardlinked in unchanged instead. Lantern's download
//!   folder must be on the same disk. When it isn't, they are copied, and the note says so.
//!
//! `POST /api/grabs/{id}/file` (admins) files a checked download again after a failure.
//!
//! The download in Lantern's folder is never changed, so it keeps seeding. Every file filed is a row in
//! `edition_files`, which the player, the reader and the other apps read.

pub mod audio;
pub mod epub;
pub mod layout;

use crate::lantern::grabs::wanted_parts;
use crate::lantern::GrabFile;
use crate::metadata::{EditionMeta, Format};
use crate::verify;
use crate::wanted::store as wanted;
use crate::AppState;
use kit_server_core::db::now;
use kit_server_core::{err, p, Db, Result, RowExt};
use serde_json::json;
use std::path::{Path, PathBuf};

/// The MIME type of a library file by its extension.
pub fn mime(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "m4b" | "m4a" | "mp4" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "epub" | "kepub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "azw3" | "azw" => "application/vnd.amazon.ebook",
        "mobi" => "application/x-mobipocket-ebook",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        _ => "application/octet-stream",
    }
}

/// The kind of a library file (`audio`, `epub`, `pdf`, `azw3` or `cover`) as `edition_files` keeps it.
pub fn kind(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "epub" | "kepub" => "epub",
        "pdf" => "pdf",
        "azw3" | "azw" | "mobi" => "azw3",
        "jpg" | "jpeg" | "png" => "cover",
        _ => "audio",
    }
}

/// Puts `src` at `dst` as a hardlink; a copy when they are on different disks. `true` when it was copied.
pub fn link_or_copy(src: &Path, dst: &Path) -> Result<bool> {
    if dst.exists() {
        std::fs::remove_file(dst).map_err(|e| err(500, format!("{} is in the way and can't be replaced: {e}", dst.display())))?;
    }
    match std::fs::hard_link(src, dst) {
        Ok(()) => Ok(false),
        Err(_) => {
            std::fs::copy(src, dst).map_err(|e| err(500, format!("{} can't be put in the library: {e}", src.display())))?;
            Ok(true)
        }
    }
}

/// Downloads the edition's cover into `dir/cover.jpg`; `None` when there is none or it can't be had (the import
/// goes on without it).
async fn cover(st: &AppState, m: &EditionMeta, dir: &Path) -> Option<PathBuf> {
    if m.cover_url.is_empty() {
        return None;
    }
    let r = st.http.get(&m.cover_url).timeout(std::time::Duration::from_secs(30)).send().await.ok()?;
    if !r.status().is_success() {
        return None;
    }
    let bytes = r.bytes().await.ok()?;
    // only JPEG or PNG pictures, by their first bytes
    if !(bytes.starts_with(&[0xFF, 0xD8, 0xFF]) || bytes.starts_with(b"\x89PNG")) {
        return None;
    }
    let path = dir.join("cover.jpg");
    std::fs::write(&path, &bytes).ok()?;
    Some(path)
}

/// One file filed: where (under the library), and for audio its length and chapters.
struct Filed {
    /// its full path
    path: PathBuf,
    /// play order
    position: i64,
    /// length in ms (audio)
    duration_ms: u64,
    /// chapters as `edition_files` keeps them
    chapters: Vec<audio::FileChapter>,
}

/// Files the verified grab `grab_id`: the library's files for its edition are replaced, the grab and the
/// wanted entry become `imported`. Answers the note it left.
pub async fn run(st: &AppState, grab_id: i64) -> Result<String> {
    // claim it, so the minute loop and a background run never file the same download twice
    let claimed = st
        .db
        .query("UPDATE grabs SET status='importing', updated_at=? WHERE id=? AND status='verified' RETURNING id", p![now(), grab_id])?;
    if claimed.is_empty() {
        return Err(err(409, "Only a checked download can be filed, once."));
    }
    let g = st.db.one("SELECT * FROM grabs WHERE id=?", p![grab_id])?.ok_or_else(|| err(404, "That download isn't known any more."))?;
    let wanted_id = g.i("wanted_id");
    let (w, m, pr) = wanted_parts(&st.db, wanted_id)?;
    let edition_id = w.i("edition_id");
    match file(st, &g, &m, &pr).await {
        Ok((rel_dir, filed, notes)) => {
            let root = library_root(st);
            st.db.execute("DELETE FROM edition_files WHERE edition_id=?", p![edition_id])?;
            for f in &filed {
                let rel = f.path.strip_prefix(&root).unwrap_or(&f.path).to_string_lossy().replace('\\', "/");
                let name = f.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let size = std::fs::metadata(&f.path).map(|x| x.len()).unwrap_or(0);
                st.db.execute(
                    "INSERT INTO edition_files (edition_id, path, name, kind, mime, size, position, duration_ms, chapters, created_at)
                     VALUES (?,?,?,?,?,?,?,?,?,?)
                     ON CONFLICT(path) DO UPDATE SET edition_id=excluded.edition_id, name=excluded.name, kind=excluded.kind, mime=excluded.mime,
                       size=excluded.size, position=excluded.position, duration_ms=excluded.duration_ms, chapters=excluded.chapters",
                    p![edition_id, rel, name, kind(&f.path), mime(&f.path), size as i64, f.position, f.duration_ms as i64, json!(f.chapters).to_string(), now()],
                )?;
            }
            let note = format!("Filed in {}.{}", rel_dir.display(), notes.iter().map(|n| format!(" {n}")).collect::<String>());
            st.db.execute("UPDATE grabs SET status='imported', updated_at=? WHERE id=?", p![now(), grab_id])?;
            wanted::set_status(&st.db, wanted_id, "imported", &note)?;
            Ok(note)
        }
        Err(e) => {
            let note = format!("It was checked, but it can't be filed: {}", e.detail);
            st.db.execute("UPDATE grabs SET status='verified', error=?, updated_at=? WHERE id=?", p![note, now(), grab_id])?;
            wanted::set_status(&st.db, wanted_id, "verified", &note)?;
            Err(err(e.status.as_u16(), note))
        }
    }
}

/// The library folder from the settings.
pub fn library_root(st: &AppState) -> PathBuf {
    PathBuf::from(st.settings.str("library_dir").trim())
}

/// The filing itself: the folder (relative), the files made and notes for people.
async fn file(
    st: &AppState,
    g: &kit_server_core::Row,
    m: &EditionMeta,
    pr: &wanted::Profile,
) -> Result<(PathBuf, Vec<Filed>, Vec<String>)> {
    let root = library_root(st);
    if root.as_os_str().is_empty() {
        return Err(err(400, "No library folder is set. Set it under Settings → Downloads."));
    }
    let files: Vec<GrabFile> = serde_json::from_str(&g.s("files")).unwrap_or_default();
    let mut mine: Vec<PathBuf> = files.iter().filter(|f| verify::takes(m.format, &f.path)).map(|f| PathBuf::from(&f.path)).collect();
    mine.sort();
    if mine.is_empty() {
        return Err(err(422, "none of the files that arrived is the book"));
    }
    let (rel_dir, name) = layout::place(m);
    let dir = root.join(&rel_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        err(500, format!("the folder {} can't be made ({e}); check that Folio may write to the library folder", dir.display()))
    })?;
    let rewrite = st.settings.bool("import_rewrite");
    let mut notes = Vec::new();
    let pic = cover(st, m, &dir).await;
    let mut filed = Vec::new();
    if let Some(c) = &pic {
        filed.push(Filed { path: c.clone(), position: 0, duration_ms: 0, chapters: Vec::new() });
    }
    match m.format {
        Format::Audiobook => {
            let facts = verify::audio::probe(&st.settings.str("ffprobe"), &mine).await?;
            if rewrite {
                let parts: Vec<(String, u64)> =
                    facts.files.iter().map(|f| (f.tags.get("title").cloned().unwrap_or_default(), f.duration_ms)).collect();
                let mut chapters = audio::chapters(m, &parts, facts.duration_ms);
                if chapters.is_empty() {
                    // a lone file whose own chapters are kept
                    chapters = facts.files[0]
                        .chapter_list
                        .iter()
                        .map(|(t, s, e)| audio::FileChapter { title: t.clone(), start_ms: *s, end_ms: *e })
                        .collect();
                }
                let out = dir.join(format!("{name}.m4b"));
                let work = st.data_dir.join("import-work").join(g.i("id").to_string());
                audio::make_m4b(audio::Job {
                    ffmpeg: &st.settings.str("ffmpeg"),
                    inputs: &facts.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>(),
                    metadata: audio::ffmetadata(m, &chapters),
                    cover: pic.as_deref(),
                    work: &work,
                    out: &out,
                })
                .await?;
                if facts.files.len() > 1 {
                    notes.push(format!("{} files were joined into one M4B with {} chapters.", facts.files.len(), chapters.len()));
                }
                filed.push(Filed { path: out, position: 0, duration_ms: facts.duration_ms, chapters });
            } else {
                for (i, f) in facts.files.iter().enumerate() {
                    let dst = dir.join(f.path.file_name().unwrap_or_default());
                    if link_or_copy(&f.path, &dst)? && notes.is_empty() {
                        notes.push("Lantern's download folder is on another disk, so the files were copied, not linked.".into());
                    }
                    let chapters =
                        f.chapter_list.iter().map(|(t, s, e)| audio::FileChapter { title: t.clone(), start_ms: *s, end_ms: *e }).collect();
                    filed.push(Filed { path: dst, position: i as i64, duration_ms: f.duration_ms, chapters });
                }
            }
        }
        Format::Ebook => {
            let ext_of = |p: &Path| p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let best = mine
                .iter()
                .min_by_key(|p| pr.file_formats.iter().position(|x| *x == ext_of(p)).unwrap_or(usize::MAX))
                .cloned()
                .unwrap_or_default();
            let ext = ext_of(&best);
            let dst = dir.join(format!("{name}.{ext}"));
            if rewrite && ext == "epub" {
                let bytes = pic.as_ref().and_then(|c| std::fs::read(c).ok());
                let (src, out, meta) = (best.clone(), dst.clone(), m.clone());
                tokio::task::spawn_blocking(move || epub::rewrite(&src, &out, &meta, bytes.as_deref()))
                    .await
                    .map_err(|e| err(500, format!("the EPUB rewrite stopped: {e}")))??;
            } else if link_or_copy(&best, &dst)? {
                notes.push("Lantern's download folder is on another disk, so the file was copied, not linked.".into());
            }
            filed.push(Filed { path: dst, position: 0, duration_ms: 0, chapters: Vec::new() });
        }
    }
    if pic.is_none() && !m.cover_url.is_empty() {
        notes.push("The cover couldn't be downloaded.".into());
    }
    Ok((rel_dir, filed, notes))
}

/// Files every checked download not filed yet.
pub async fn pending(st: &AppState) -> Result<usize> {
    let mut n = 0;
    // one that failed waits for "File again" (POST /api/grabs/{id}/file) instead of failing every minute
    for r in st.db.query("SELECT id FROM grabs WHERE status='verified' AND error='' ORDER BY id", p![])? {
        match run(st, r.i("id")).await {
            Ok(_) => n += 1,
            Err(e) => tracing::warn!("download {} can't be filed: {}", r.i("id"), e.detail),
        }
    }
    Ok(n)
}

/// The import's route: `POST /api/grabs/{id}/file` (admins) files a checked download again after a failure.
pub fn router() -> axum::Router<AppState> {
    axum::Router::new().route("/api/grabs/{id}/file", axum::routing::post(file_again))
}

/// `POST /api/grabs/{id}/file`.
async fn file_again(
    axum::extract::State(st): axum::extract::State<AppState>,
    _: kit_server_core::Admin,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<axum::Json<serde_json::Value>> {
    st.db.execute("UPDATE grabs SET error='' WHERE id=? AND status='verified'", p![id])?;
    let note = run(&st, id).await?;
    Ok(axum::Json(json!({ "ok": true, "note": note })))
}

/// Checks and files in the background (after Lantern's callback or an Accept), so the request answers at once.
pub fn spawn_pending(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move {
        if let Err(e) = verify::pending(&st).await {
            tracing::warn!("downloads can't be checked: {}", e.detail);
        }
        if let Err(e) = pending(&st).await {
            tracing::warn!("downloads can't be filed: {}", e.detail);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_their_kind_and_type() {
        assert_eq!((kind(Path::new("a/B.M4B")), mime(Path::new("a/B.M4B"))), ("audio", "audio/mp4"));
        assert_eq!((kind(Path::new("b.epub")), mime(Path::new("b.epub"))), ("epub", "application/epub+zip"));
        assert_eq!(kind(Path::new("cover.jpg")), "cover");
        assert_eq!(kind(Path::new("x.azw3")), "azw3");
    }

    #[test]
    fn a_link_is_made_and_replaces_what_is_in_the_way() {
        let d = std::env::temp_dir().join(format!("folio-link-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), b"book").unwrap();
        std::fs::write(d.join("b"), b"old").unwrap();
        assert!(!link_or_copy(&d.join("a"), &d.join("b")).unwrap(), "same disk: linked");
        assert_eq!(std::fs::read(d.join("b")).unwrap(), b"book");
        let _ = std::fs::remove_dir_all(&d);
    }
}
