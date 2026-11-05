//! Readarr's disk work for deletes: where its paths are on this machine, and its recycling bin
//! (`RecycleBinProvider`), which keeps deleted files and folders in a folder of the user's choosing instead of
//! deleting them for good.
//!
//! Readarr's paths (`/books/Terry Pratchett`) are the paths inside its container. Folio sees the same library at
//! the same paths, so [`Disk`] uses them as they are; `READARR_DISK_ROOT` puts them under another folder instead
//! (the tests use a temporary folder).

use kit_server_core::{Db, SqliteDb};
use serde_json::{json, Value};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where Readarr's paths are on this machine.
#[derive(Clone, Debug)]
pub struct Disk {
    /// The folder Readarr's `/` is in: `/` itself unless `READARR_DISK_ROOT` says otherwise.
    root: PathBuf,
}

impl Default for Disk {
    fn default() -> Disk {
        Disk { root: PathBuf::from("/") }
    }
}

impl Disk {
    /// Readarr's paths under `root` (nothing or blank: as they are).
    pub fn new(root: Option<String>) -> Disk {
        match root.filter(|r| !r.trim().is_empty()) {
            Some(r) => Disk { root: PathBuf::from(r) },
            None => Disk::default(),
        }
    }

    /// Where a Readarr path is on this machine.
    pub fn real(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    /// Whether the folder exists.
    pub fn folder_exists(&self, path: &str) -> bool {
        self.real(path).is_dir()
    }

    /// Whether the file exists.
    pub fn file_exists(&self, path: &str) -> bool {
        self.real(path).is_file()
    }

    /// Whether the folder has folders in it (`GetDirectories(..).Any()`).
    pub fn has_folders(&self, path: &str) -> bool {
        std::fs::read_dir(self.real(path)).map(|d| d.flatten().any(|e| e.path().is_dir())).unwrap_or(false)
    }

    /// Whether the folder has no files in it or in any folder below it (`GetFiles(path, true).Empty()`).
    pub fn no_files(&self, path: &str) -> bool {
        fn none(p: &Path) -> bool {
            std::fs::read_dir(p).map(|d| d.flatten().all(|e| e.path().is_dir() && none(&e.path()))).unwrap_or(true)
        }
        none(&self.real(path))
    }

    /// Deletes a folder and everything in it.
    pub fn delete_folder(&self, path: &str) -> io::Result<()> {
        std::fs::remove_dir_all(self.real(path))
    }

    /// Deletes the empty folders below a folder, deepest first (`RemoveEmptySubfolders`); the folder itself stays.
    pub fn remove_empty_subfolders(&self, path: &str) {
        fn walk(p: &Path) {
            let Ok(dir) = std::fs::read_dir(p) else { return };
            for sub in dir.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
                walk(&sub);
                if std::fs::read_dir(&sub).map(|mut d| d.next().is_none()).unwrap_or(false) {
                    let _ = std::fs::remove_dir(&sub);
                }
            }
        }
        walk(&self.real(path));
    }
}

/// A setting of Readarr's (`Config`, keys in lower case), if it is set.
pub fn config(db: &SqliteDb, key: &str) -> kit_server_core::Result<Option<String>> {
    Ok(db
        .one("SELECT Value FROM Config WHERE Key = ?", vec![json!(key.to_lowercase())])?
        .map(|r| r.get("Value").and_then(Value::as_str).unwrap_or_default().to_string()))
}

/// The recycling bin folder (`RecycleBin`), if one is set.
fn recycle_bin(db: &SqliteDb) -> kit_server_core::Result<Option<String>> {
    Ok(config(db, "RecycleBin")?.filter(|b| !b.trim().is_empty()))
}

/// The last part of a path (`/a/b/` gives `b`).
fn name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_string()
}

/// Joins Readarr paths like .NET's `Path.Combine`: a second part that starts with `/` replaces the first.
pub fn combine(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_string()
    } else if b.is_empty() {
        a.to_string()
    } else {
        format!("{}/{b}", a.trim_end_matches('/'))
    }
}

/// Moves a file or folder, also to another disk (copy, then delete) when renaming can't.
fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            move_path(&e.path(), &to.join(e.file_name()))?;
        }
        std::fs::remove_dir(from)
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)
    }
}

/// Marks a file or folder as changed now, as Readarr does to what it puts in the bin (the bin's cleanup goes by it).
fn touch(p: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(p).or_else(|_| std::fs::File::open(p)) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Marks everything under a folder as changed now.
fn touch_all(p: &Path) {
    touch(p);
    if let Ok(dir) = std::fs::read_dir(p) {
        for e in dir.flatten() {
            if e.path().is_dir() {
                touch_all(&e.path());
            } else {
                touch(&e.path());
            }
        }
    }
}

/// Sends a folder to the recycling bin, or deletes it for good when there is no bin (`DeleteFolder`).
pub fn recycle_folder(db: &SqliteDb, disk: &Disk, path: &str) -> anyhow::Result<()> {
    match recycle_bin(db)? {
        None => disk.delete_folder(path)?,
        Some(bin) => {
            let to = disk.real(&combine(&bin, &name(path)));
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            move_path(&disk.real(path), &to)?;
            touch_all(&to);
        }
    }
    Ok(())
}

/// Sends a file to the recycling bin's `subfolder` (with `_2`, `_3`, … when the name is taken there), or deletes it
/// for good when there is no bin (`DeleteFile`).
pub fn recycle_file(db: &SqliteDb, disk: &Disk, path: &str, subfolder: &str) -> anyhow::Result<()> {
    match recycle_bin(db)? {
        None => std::fs::remove_file(disk.real(path))?,
        Some(bin) => {
            let folder = combine(&bin, subfolder);
            std::fs::create_dir_all(disk.real(&folder))?;
            let file = name(path);
            let (stem, ext) = match file.rfind('.') {
                Some(i) if i > 0 => (file[..i].to_string(), file[i..].to_string()),
                _ => (file.clone(), String::new()),
            };
            let mut to = combine(&folder, &file);
            let mut index = 1;
            while disk.real(&to).exists() {
                index += 1;
                to = combine(&folder, &format!("{stem}_{index}{ext}"));
            }
            move_path(&disk.real(path), &disk.real(&to))?;
            touch(&disk.real(&to));
        }
    }
    Ok(())
}

/// The part of `child` below `parent` (`GetRelativePath`), if `parent` is above it.
pub fn relative(parent: &str, child: &str) -> Option<String> {
    if !super::validation::is_parent_path(parent, child) {
        return None;
    }
    Some(super::validation::clean(child)[super::validation::clean(parent).len()..].trim_matches('/').to_string())
}

/// The folder a path is in (`GetParentPath`), if it has one.
pub fn parent(path: &str) -> Option<String> {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) if p.len() > 1 => Some("/".to_string()),
        Some(i) if i > 0 => Some(p[..i].to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_the_recycling_bin_work_like_readarr() {
        assert_eq!(parent("/books/A/b.epub").as_deref(), Some("/books/A"));
        assert_eq!(parent("/books").as_deref(), Some("/"));
        assert_eq!(relative("/books", "/books/A/B").as_deref(), Some("A/B"));
        assert_eq!(relative("/books", "/other"), None);
        assert_eq!(combine("/bin", ""), "/bin");
        assert_eq!(combine("/bin", "A/B"), "/bin/A/B");

        let dir = std::env::temp_dir().join(format!("folio-readarr-disk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let disk = Disk::new(Some(dir.to_string_lossy().into_owned()));
        std::fs::create_dir_all(disk.real("/books/A/B")).unwrap();
        std::fs::write(disk.real("/books/A/B/x.epub"), b"1").unwrap();
        std::fs::write(disk.real("/books/A/y.epub"), b"2").unwrap();
        let db = SqliteDb::open(&dir.join("r.db")).unwrap();
        super::super::create(&db).unwrap();
        db.execute("INSERT INTO Config (Key, Value) VALUES ('recyclebin', '/bin')", vec![]).unwrap();

        recycle_file(&db, &disk, "/books/A/B/x.epub", "A/B").unwrap();
        std::fs::write(disk.real("/books/A/B/x.epub"), b"3").unwrap();
        recycle_file(&db, &disk, "/books/A/B/x.epub", "A/B").unwrap();
        assert!(disk.file_exists("/bin/A/B/x.epub") && disk.file_exists("/bin/A/B/x_2.epub"));
        assert!(disk.no_files("/books/A/B") && !disk.no_files("/books/A"));
        assert!(disk.has_folders("/books/A"));
        disk.remove_empty_subfolders("/books/A");
        assert!(!disk.folder_exists("/books/A/B"));

        recycle_folder(&db, &disk, "/books/A").unwrap();
        assert!(!disk.folder_exists("/books/A") && disk.file_exists("/bin/A/y.epub"));
        db.execute("DELETE FROM Config", vec![]).unwrap();
        recycle_folder(&db, &disk, "/bin/A").unwrap();
        assert!(!disk.folder_exists("/bin/A"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
