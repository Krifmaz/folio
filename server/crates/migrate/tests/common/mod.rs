//! Helpers for the source tests: a fresh folder per test, a database made from SQL in it, and a fingerprint of a
//! folder to prove reading changed nothing.

#![allow(dead_code)]

use rusqlite::Connection;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A new, empty folder for the test `name` (removed first if an earlier run left it).
pub fn folder(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("folio-migrate-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Makes the database `file` from `sql` and closes it.
pub fn database(file: &Path, sql: &str) {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    let c = Connection::open(file).unwrap();
    c.execute_batch(sql).unwrap();
}

/// Every file under `dir` with its bytes and modification time: equal fingerprints mean nothing changed.
pub fn fingerprint(dir: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let m = fs::metadata(&p).unwrap().modified().unwrap();
                out.insert(p.clone(), (fs::read(&p).unwrap(), m));
            }
        }
    }
    out
}

/// Whether `dir` is empty (or gone): the scratch copies were cleaned up.
pub fn is_empty(dir: &Path) -> bool {
    fs::read_dir(dir).map(|mut r| r.next().is_none()).unwrap_or(true)
}
