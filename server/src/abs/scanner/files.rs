//! Walking a library folder and describing its files, like the original's `fileUtils.recurseFiles`,
//! `getFileTimestampsWithIno` and the `LibraryFile` and `FileMetadata` objects.

use serde_json::{json, Value};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// The picture formats the original knows (`globals.SupportedImageTypes`).
pub const IMAGE_TYPES: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// The audio formats the original knows (`globals.SupportedAudioTypes`).
pub const AUDIO_TYPES: &[&str] = &[
    "m4b", "mp3", "m4a", "flac", "opus", "ogg", "oga", "mp4", "aac", "wma", "aiff", "aif", "wav", "webm", "webma", "mka", "awb", "caf",
    "mpg", "mpeg",
];
/// The ebook formats the original knows (`globals.SupportedEbookTypes`).
pub const EBOOK_TYPES: &[&str] = &["epub", "pdf", "mobi", "azw3", "cbr", "cbz"];
/// The text files the original reads (`globals.TextFileTypes`).
pub const TEXT_TYPES: &[&str] = &["txt", "nfo"];
/// The metadata files the original reads (`globals.MetadataFileTypes`).
pub const METADATA_TYPES: &[&str] = &["opf", "abs", "xml", "json"];

/// The format of an extension like ".MP3": "mp3" (`FileMetadata.format`).
pub fn format_of(ext: &str) -> String {
    ext.strip_prefix('.').unwrap_or("").to_lowercase()
}

/// Whether an extension (with its dot) is one of `types`.
pub fn ext_in(ext: &str, types: &[&str]) -> bool {
    let f = format_of(ext);
    !f.is_empty() && types.contains(&f.as_str())
}

/// Node's `path.extname`: the last dot and what follows it in the last part, unless the part starts with its only
/// dot or ends with it.
pub fn extname(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rfind('.') {
        Some(i) if i > 0 && i < base.len() && !base[..i].chars().all(|c| c == '.') => base[i..].to_string(),
        _ => String::new(),
    }
}

/// Node's `path.basename`, optionally without an ending.
pub fn basename(path: &str, ext: Option<&str>) -> String {
    let trimmed = path.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
    match ext {
        Some(e) if !e.is_empty() && base.ends_with(e) && base != e => base[..base.len() - e.len()].to_string(),
        _ => base,
    }
}

/// Node's `path.dirname` for '/' paths.
pub fn dirname(path: &str) -> String {
    let trimmed = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].to_string(),
        None => ".".into(),
    }
}

/// Node's `path.posix.join`: the parts joined with '/' and normalized ("." and ".." resolved, no doubled slashes).
pub fn join(parts: &[&str]) -> String {
    let joined = parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join("/");
    if joined.is_empty() {
        return ".".into();
    }
    normalize(&joined)
}

/// Node's `path.posix.normalize`.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !absolute {
                    out.push("..");
                }
            }
            p => out.push(p),
        }
    }
    let mut s = out.join("/");
    if absolute {
        s = format!("/{s}");
    }
    if s.is_empty() {
        return ".".into();
    }
    if trailing && !s.ends_with('/') {
        s.push('/');
    }
    s
}

/// What `fs.stat` says about a file, as the original keeps it: whole milliseconds and the inode as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stamps {
    /// The size in bytes.
    pub size: i64,
    /// When the contents last changed.
    pub mtime_ms: i64,
    /// When the file's details last changed.
    pub ctime_ms: i64,
    /// When the file was made (0 where the system doesn't say).
    pub birthtime_ms: i64,
    /// The inode number, as text.
    pub ino: String,
}

/// The time stamps, size and inode of a path (`getFileTimestampsWithIno`); `None` when it can't be read.
pub fn stamps(path: &str) -> Option<Stamps> {
    let m = std::fs::metadata(path).ok()?;
    let to_ms = |t: std::io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
    };
    Some(Stamps {
        size: m.len() as i64,
        mtime_ms: m.mtime() * 1000 + m.mtime_nsec() / 1_000_000,
        ctime_ms: m.ctime() * 1000 + m.ctime_nsec() / 1_000_000,
        birthtime_ms: to_ms(m.created()),
        ino: m.ino().to_string(),
    })
}

/// Why a file or folder is skipped (`fileUtils.shouldIgnoreFile`), or `None` to keep it.
pub fn should_ignore(path: &str) -> Option<String> {
    if basename(path, None).starts_with('.') {
        return Some("dotfile".into());
    }
    if path.split('/').any(|p| p.starts_with('.')) {
        return Some("dotpath".into());
    }
    if path.contains("@eaDir") {
        return Some("@eaDir directory".into());
    }
    let ext = extname(path);
    let ignored = [".part", ".tmp", ".crdownload", ".download", ".bak", ".old", ".temp", ".tempfile", ".tempfile~"];
    if ignored.contains(&ext.to_lowercase().as_str()) {
        return Some(format!("{ext} file"));
    }
    None
}

/// A file found under a folder (the original's `FilePathItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct FileItem {
    /// The file name.
    pub name: String,
    /// The path below the folder walked.
    pub path: String,
    /// The folder below the folder walked ("" at the top).
    pub reldirpath: String,
    /// The whole path.
    pub fullpath: String,
    /// The extension with its dot.
    pub extension: String,
    /// How many folders down the file is (0 at the top).
    pub deep: usize,
}

/// Every file under a folder (`fileUtils.recurseFiles`), the least deep first. Hidden files and folders, folders
/// holding a `.ignore` file and temporary downloads are left out. Within one depth files come in name order.
pub fn recurse_files(path: &str, rel_to_replace: Option<&str>) -> Vec<FileItem> {
    let mut root = path.to_string();
    if !root.ends_with('/') {
        root.push('/');
    }
    let mut rel_root = rel_to_replace.map(String::from).unwrap_or_else(|| root.clone());
    if !rel_root.ends_with('/') {
        rel_root.push('/');
    }
    let mut found: Vec<(String, String, usize)> = Vec::new();
    walk(&root, 0, &mut found);
    let mut ignored_dirs: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    for (dir, name, deep) in found {
        let full = format!("{}/{}", dir.trim_end_matches('/'), name);
        let rel = full.replacen(&rel_root, "", 1);
        let mut reldir = dirname(&rel);
        if reldir == "." {
            reldir = String::new();
        }
        let dirn = dirname(&full);
        if name == ".ignore" && !reldir.is_empty() && !ignored_dirs.contains(&dirn) {
            ignored_dirs.push(dirn);
            continue;
        }
        if should_ignore(&rel).is_some() {
            continue;
        }
        kept.push((dir, name, deep, full));
    }
    let mut list: Vec<FileItem> = kept
        .into_iter()
        .filter(|(_, _, _, full)| !ignored_dirs.iter().any(|d| full.starts_with(&format!("{d}/"))))
        .map(|(dir, name, deep, full)| {
            let dir = dir.trim_end_matches('/').to_string();
            let in_root = format!("{dir}/") == rel_root;
            FileItem {
                extension: extname(&name),
                path: full.replacen(&rel_root, "", 1),
                reldirpath: if in_root { String::new() } else { dir.replacen(&rel_root, "", 1) },
                fullpath: full,
                name,
                deep,
            }
        })
        .collect();
    list.sort_by_key(|f| f.deep);
    list
}

/// Collects the files under `dir` as (folder, name, depth), folders followed through links like `realPath`.
fn walk(dir: &str, deep: usize, out: &mut Vec<(String, String, usize)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<(String, bool)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let is_dir = std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false);
            Some((name, is_dir))
        })
        .collect();
    entries.sort();
    let dir = dir.trim_end_matches('/');
    for (name, is_dir) in entries {
        if is_dir {
            walk(&format!("{dir}/{name}"), deep + 1, out);
        } else {
            out.push((dir.to_string(), name, deep));
        }
    }
}

/// A library file as the original stores it (`LibraryFile.toJSON`), made from a path on disk
/// (`setDataFromPath`). `rel_path` is the path below the item's folder.
pub fn library_file(path: &str, rel_path: &str, now: i64) -> Value {
    let st = stamps(path).unwrap_or_default();
    let ext = extname(rel_path);
    json!({
        "ino": st.ino,
        "metadata": {
            "filename": basename(rel_path, None),
            "ext": ext,
            "path": path,
            "relPath": rel_path,
            "size": st.size,
            "mtimeMs": st.mtime_ms,
            "ctimeMs": st.ctime_ms,
            "birthtimeMs": st.birthtime_ms,
        },
        "isSupplementary": null,
        "addedAt": now,
        "updatedAt": now,
        "fileType": crate::abs::models::library_item::file_type(Some(&ext)),
    })
}

/// The extension of a stored library file (`metadata.ext`).
pub fn file_ext(lf: &Value) -> String {
    lf["metadata"]["ext"].as_str().unwrap_or("").to_string()
}

/// The whole path of a stored library file (`metadata.path`).
pub fn file_path(lf: &Value) -> String {
    lf["metadata"]["path"].as_str().unwrap_or("").to_string()
}

/// The inode of a stored file, as text.
pub fn file_ino(lf: &Value) -> Option<String> {
    match &lf["ino"] {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Reads a text file, or "" when it can't be read (`readTextFile`).
pub fn read_text(path: &str) -> String {
    std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Whether a path exists.
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_like_node() {
        assert_eq!(extname("a/b.c.mp3"), ".mp3");
        assert_eq!(extname(".ignore"), "");
        assert_eq!(extname("noext"), "");
        assert_eq!(basename("/a/b/Part 01.mp3", Some(".mp3")), "Part 01");
        assert_eq!(dirname("a/b.mp3"), "a");
        assert_eq!(dirname("b.mp3"), ".");
        assert_eq!(join(&["/media", "A/B"]), "/media/A/B");
        assert_eq!(join(&["", "x.mp3"]), "x.mp3");
        assert_eq!(join(&["CD1", "x.mp3"]), "CD1/x.mp3");
    }

    #[test]
    fn ignores_like_the_original() {
        assert_eq!(should_ignore(".hidden").as_deref(), Some("dotfile"));
        assert_eq!(should_ignore("a/.b/c.mp3").as_deref(), Some("dotpath"));
        assert_eq!(should_ignore("a/@eaDir/c.mp3").as_deref(), Some("@eaDir directory"));
        assert_eq!(should_ignore("a/c.PART").as_deref(), Some(".PART file"));
        assert_eq!(should_ignore("a/c.mp3"), None);
    }

    #[test]
    fn walks_folders() {
        let dir = std::env::temp_dir().join(format!("folio-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("A/B/skip")).unwrap();
        std::fs::write(dir.join("A/B/1.mp3"), b"x").unwrap();
        std::fs::write(dir.join("A/B/skip/.ignore"), b"").unwrap();
        std::fs::write(dir.join("A/B/skip/2.mp3"), b"x").unwrap();
        std::fs::write(dir.join("top.m4b"), b"x").unwrap();
        let root = dir.display().to_string();
        let list = recurse_files(&root, None);
        let paths: Vec<_> = list.iter().map(|f| (f.path.as_str(), f.reldirpath.as_str(), f.deep)).collect();
        assert_eq!(paths, vec![("top.m4b", "", 0), ("A/B/1.mp3", "A/B", 2)]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
