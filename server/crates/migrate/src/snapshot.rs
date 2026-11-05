//! Reading a source's database without touching it.
//!
//! Opening a SQLite file, even read-only, can write beside it (a `-shm` file, a checkpoint of the write-ahead
//! log). So the database and its `-wal` are copied into a scratch folder, the copy is opened, and the copy is
//! removed when the [`Snapshot`] is dropped. The source app can keep running while this happens.

use crate::{Error, Result, Source};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Counts snapshots, so two taken at once never share a name.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The database file to read for `source` at `path`: `path` itself when it is a file, else the source's file
/// inside the folder (`metadata.db` in a Calibre library).
pub fn database_file(source: Source, path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        let f = path.join(source.file_name());
        if f.is_file() {
            return Ok(f);
        }
        return Err(Error::new(format!(
            "There is no {} in {}. Choose the {} file itself, or the folder that holds it.",
            source.file_name(),
            path.display(),
            source.name()
        )));
    }
    Err(Error::new(format!("{} can't be found. Check the path and that Folio can see that folder.", path.display())))
}

/// A private copy of a database, open for reading; removed when dropped.
pub struct Snapshot {
    /// the open copy
    conn: Option<Connection>,
    /// the copy's file (its `-wal` and `-shm` sit beside it)
    file: PathBuf,
}

impl Snapshot {
    /// Copies the database `file` (and its `-wal`, when there is one) into `scratch` and opens the copy.
    pub fn take(file: &Path, scratch: &Path) -> Result<Snapshot> {
        fs::create_dir_all(scratch).map_err(|e| Error::new(format!("Folio can't make its scratch folder {}: {e}.", scratch.display())))?;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let copy = scratch.join(format!("migrate-{}-{n}.sqlite", std::process::id()));
        let mut snap = Snapshot { conn: None, file: copy.clone() };
        fs::copy(file, &copy).map_err(|e| Error::new(format!("{} can't be read: {e}. Check that Folio may read it.", file.display())))?;
        let wal = sidecar(file, "-wal");
        if wal.is_file() {
            fs::copy(&wal, sidecar(&copy, "-wal"))
                .map_err(|e| Error::new(format!("{} can't be read: {e}. Check that Folio may read it.", wal.display())))?;
        }
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| Error::new(format!("{} can't be opened as a database: {e}.", file.display())))?;
        // a file that isn't SQLite only fails on the first read
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
            .map_err(|_| Error::new(format!("{} is not a database Folio can read. Choose the app's database file.", file.display())))?;
        snap.conn = Some(conn);
        Ok(snap)
    }

    /// The open copy.
    pub fn conn(&self) -> &Connection {
        self.conn.as_ref().expect("a snapshot is open until it is dropped")
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        drop(self.conn.take());
        for f in [self.file.clone(), sidecar(&self.file, "-wal"), sidecar(&self.file, "-shm"), sidecar(&self.file, "-journal")] {
            let _ = fs::remove_file(f);
        }
    }
}

/// `file` with `suffix` added to its name (`db.sqlite-wal`).
fn sidecar(file: &Path, suffix: &str) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}
