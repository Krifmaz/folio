//! Taking a book's cover out of its audio files or its ebook, like the original's
//! `CoverManager.saveEmbeddedCoverArt` and `saveEbookCoverArt`.

use super::meta::{extract_ebook_cover, EbookScan};
use super::Scan;
use crate::abs::settings::truthy;
use serde_json::Value;
use std::process::Command;

/// The folder a cover goes to: the item's own folder with `storeCoverWithItem`, else `<metadata>/items/<id>`.
fn cover_dir(scan: &Scan, item_id: &str, item_dir: Option<&str>) -> String {
    match item_dir {
        Some(d) if truthy(scan.settings.get("storeCoverWithItem")) => d.to_string(),
        _ => scan.metadata_path.join("items").join(item_id).display().to_string(),
    }
}

/// Saves the picture embedded in the first audio file that has one, with ffmpeg; the cover's path, or `None` when
/// no file has one, a cover is already there or ffmpeg fails.
pub fn save_embedded(scan: &Scan, audio_files: &[Value], item_id: &str, item_dir: Option<&str>) -> Option<String> {
    let with_cover = audio_files.iter().find(|a| a["embeddedCoverArt"].as_str().is_some_and(|c| !c.is_empty()))?;
    let dir = cover_dir(scan, item_id, item_dir);
    std::fs::create_dir_all(&dir).ok()?;
    let name = if with_cover["embeddedCoverArt"] == "png" { "cover.png" } else { "cover.jpg" };
    let out = format!("{}/{name}", dir.trim_end_matches('/'));
    if std::path::Path::new(&out).exists() {
        return None;
    }
    let input = with_cover["metadata"]["path"].as_str()?;
    let ffmpeg = std::env::var("FFMPEG_PATH").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "ffmpeg".into());
    let ok = Command::new(ffmpeg)
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-y", "-map", "0:v:0", "-frames:v", "1"])
        .arg(&out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(out)
}

/// Saves an ebook's cover picture as `cover.<ext>` (".jpeg" as ".jpg"), replacing one already there.
pub fn save_ebook(scan: &Scan, ebook: &EbookScan, item_id: &str, item_dir: Option<&str>) -> Option<String> {
    let inner = ebook.cover_path.as_deref()?;
    let dir = cover_dir(scan, item_id, item_dir);
    std::fs::create_dir_all(&dir).ok()?;
    let mut ext = super::files::extname(inner);
    if ext.is_empty() {
        ext = ".jpg".into();
    }
    if ext == ".jpeg" {
        ext = ".jpg".into();
    }
    let out = format!("{}/cover{ext}", dir.trim_end_matches('/'));
    extract_ebook_cover(ebook, &out).then_some(out)
}
