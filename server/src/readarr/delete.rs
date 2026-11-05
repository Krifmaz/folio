//! Deleting authors, books and book files the way Readarr does it: the row, then what Readarr's event handlers
//! do after it (`AuthorDeletedEvent`, `BookDeletedEvent`, `BookFileDeletedEvent`), in the same order.
//!
//! Readarr's own quirks are kept, because what is left behind is what its clients see afterwards:
//! - A deleted book's editions go first, so its files are found by no one later: they stay, on disk and in the
//!   database, even with `deleteFiles` (Readarr's housekeeping unlinks them later).
//! - Deleting an author with `deleteFiles` deletes its folder, unless another author's folder is the same or below
//!   it; its file rows stay, as above.
//! - Deleting an author deletes its history, download history, pending releases, blocklist and extra files, not
//!   its metadata (housekeeping does that later).
//!
//! Book covers are not kept by Folio yet (they come with `MediaCover`), so there is no cover folder to delete.

use super::disk::{self, Disk};
use super::json::{doc, now_param};
use super::validation::{is_parent_path, path_equals};
use super::{bookfiles, Answer, Fail};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// `DeleteMediaFileReason.Manual`, the reason the API's deletes give.
const MANUAL: &str = "Manual";

/// `EntityHistoryEventType.BookFileDeleted`.
const BOOK_FILE_DELETED: i64 = 5;

/// A root folder, as far as deleting needs it.
struct Root {
    /// its path (`/books/`)
    path: String,
    /// whether it is a Calibre library (and has Calibre's settings)
    calibre: bool,
}

/// The root folder a path is in, the deepest one (`GetBestRootFolder`); none when it is in none.
fn best_root(db: &SqliteDb, path: &str) -> kit_server_core::Result<Option<Root>> {
    let roots = db.query("SELECT Path, IsCalibreLibrary, CalibreSettings FROM RootFolders", vec![])?;
    Ok(roots
        .into_iter()
        .map(|r| Root {
            path: r.s("Path"),
            calibre: r.i("IsCalibreLibrary") != 0 && r.get("CalibreSettings").is_some_and(|v| !v.is_null()),
        })
        .filter(|r| path_equals(&r.path, path) || is_parent_path(&r.path, path))
        .max_by_key(|r| r.path.len()))
}

/// Adds an import list exclusion unless one for `foreign_id` exists (`ImportListExclusionService`).
fn exclude(db: &SqliteDb, foreign_id: &str, name: &str) -> kit_server_core::Result<()> {
    if db.one("SELECT Id FROM ImportListExclusions WHERE ForeignId = ?", vec![json!(foreign_id)])?.is_none() {
        db.execute("INSERT INTO ImportListExclusions (ForeignId, Name) VALUES (?, ?)", vec![json!(foreign_id), json!(name)])?;
    }
    Ok(())
}

/// Deletes a book and what goes with it (`BookService.DeleteBook` and the `BookDeletedEvent` handlers).
pub fn book(db: &SqliteDb, disk: &Disk, id: i64, delete_files: bool, add_exclusion: bool) -> Answer<()> {
    let Some(b) = db.one(
        "SELECT b.Id, b.ForeignBookId, b.Title, m.Name FROM Books b LEFT JOIN AuthorMetadata m ON m.Id = b.AuthorMetadataId WHERE b.Id = ?",
        vec![json!(id)],
    )?
    else {
        return Err(Fail::model_not_found("Book", id));
    };
    db.execute("DELETE FROM Books WHERE Id = ?", vec![json!(id)])?;
    after_book(db, disk, &b, delete_files, add_exclusion)
}

/// What Readarr does once a book row is gone: its editions and series links (right away), then its files, the
/// exclusion and its covers (`HandleAsync`).
fn after_book(db: &SqliteDb, disk: &Disk, b: &Row, delete_files: bool, add_exclusion: bool) -> Answer<()> {
    let id = b.i("Id");
    db.execute("DELETE FROM Editions WHERE BookId = ?", vec![json!(id)])?;
    db.execute("DELETE FROM SeriesBookLink WHERE BookId = ?", vec![json!(id)])?;
    // `GetFilesByBook` goes through the book's editions, which are gone by now, so it finds none: Readarr's files
    // of a deleted book stay where they are.
    let files = bookfiles::of_book(db, id)?;
    for f in &files {
        if delete_files {
            delete_file_on_disk(db, disk, f, "")?;
            db.execute("DELETE FROM BookFiles WHERE Id = ?", vec![json!(f.i("Id"))])?;
        } else {
            db.execute("UPDATE BookFiles SET EditionId = 0 WHERE Id = ?", vec![json!(f.i("Id"))])?;
        }
    }
    if add_exclusion {
        exclude(db, &b.s("ForeignBookId"), &format!("{} - {}", b.s("Name"), b.s("Title")))?;
    }
    Ok(())
}

/// Deletes an author and what goes with it (`AuthorService.DeleteAuthor` and the `AuthorDeletedEvent` handlers).
pub fn author(db: &SqliteDb, disk: &Disk, id: i64, delete_files: bool, add_exclusion: bool) -> Answer<()> {
    let Some(a) = db.one(
        "SELECT a.Id, a.Path, a.AuthorMetadataId, m.ForeignAuthorId, m.Name FROM Authors a
         LEFT JOIN AuthorMetadata m ON m.Id = a.AuthorMetadataId WHERE a.Id = ?",
        vec![json!(id)],
    )?
    else {
        return Err(Fail::model_not_found("Author", id));
    };
    db.execute("DELETE FROM Authors WHERE Id = ?", vec![json!(id)])?;
    // Right away: the author's books (each with its own handlers), history, download history, pending releases.
    let books = db.query(
        "SELECT b.Id, b.ForeignBookId, b.Title, m.Name FROM Books b LEFT JOIN AuthorMetadata m ON m.Id = b.AuthorMetadataId
         WHERE b.AuthorMetadataId = ? ORDER BY b.Id",
        vec![json!(a.i("AuthorMetadataId"))],
    )?;
    for b in &books {
        db.execute("DELETE FROM Books WHERE Id = ?", vec![json!(b.i("Id"))])?;
    }
    for b in &books {
        after_book(db, disk, b, false, false)?;
    }
    for table in ["History", "DownloadHistory", "PendingReleases"] {
        db.execute(&format!("DELETE FROM {table} WHERE AuthorId = ?"), vec![json!(id)])?;
    }
    // Then: the author's folder, the blocklist, extra files, the exclusion.
    if delete_files {
        delete_author_folder(db, disk, &a)?;
    }
    for table in ["Blocklist", "ExtraFiles", "MetadataFiles"] {
        db.execute(&format!("DELETE FROM {table} WHERE AuthorId = ?"), vec![json!(id)])?;
    }
    if add_exclusion {
        exclude(db, &a.s("ForeignAuthorId"), &a.s("Name"))?;
    }
    Ok(())
}

/// Deletes the author's folder like `MediaFileDeletionService.HandleAsync(AuthorDeletedEvent)`: not when another
/// author's folder is the same or below it, and not in a Calibre library (Calibre deletes those, which Folio can't
/// reach yet). Readarr only logs what goes wrong here; so does Folio.
fn delete_author_folder(db: &SqliteDb, disk: &Disk, a: &Row) -> Answer<()> {
    let path = a.s("Path");
    match best_root(db, &path)? {
        None => {
            tracing::warn!("The folder of the deleted author {path} is in no root folder, so it is kept.");
            return Ok(());
        }
        Some(r) if r.calibre => {
            tracing::warn!("The folder of the deleted author {path} is in a Calibre library, which Folio can't change yet.");
            return Ok(());
        }
        Some(_) => {}
    }
    for other in db.query("SELECT Path FROM Authors WHERE Id <> ?", vec![json!(a.i("Id"))])? {
        let other = other.s("Path");
        if is_parent_path(&path, &other) || path_equals(&path, &other) {
            tracing::warn!("The folder of the deleted author {path} holds another author's folder, so it is kept.");
            return Ok(());
        }
    }
    if disk.folder_exists(&path) {
        if let Err(e) = disk::recycle_folder(db, disk, &path) {
            tracing::warn!("The folder of the deleted author {path} could not be deleted: {e}");
        }
    }
    Ok(())
}

/// Deletes a book file the way `DELETE /api/v1/bookfile/{id}` does (`BookFileController.DeleteBookFile`): a file
/// of an author goes to the recycling bin under its folder's path below the root folder, a file of no author under
/// `Unmapped_Files`.
pub fn book_file(db: &SqliteDb, disk: &Disk, f: &Row) -> Answer<()> {
    let author_path = f.get("AuthorPath").and_then(Value::as_str).map(str::to_string);
    match author_path {
        Some(author_path) if f.i("EditionId") > 0 => {
            let root = disk::parent(&author_path).unwrap_or_default();
            if !disk.folder_exists(&root) {
                return Err(client(409, format!("Author's root folder ({root}) doesn't exist.")));
            }
            if !disk.has_folders(&root) {
                return Err(client(409, format!("Author's root folder ({root}) is empty.")));
            }
            if disk.folder_exists(&author_path) {
                let folder = disk::parent(&f.s("Path")).unwrap_or_default();
                let sub = disk::relative(&root, &folder).ok_or_else(|| {
                    Fail::exception(500, "NzbDrone.Common.Exceptions.NotParentException", format!("{folder} is not a child of {root}"))
                })?;
                delete_track_file(db, disk, f, &sub)
            } else {
                // The file is forgotten even when the author's folder is gone.
                forget(db, disk, f)
            }
        }
        _ => delete_track_file(db, disk, f, "Unmapped_Files"),
    }
}

/// `NzbDroneClientException`: Readarr's message for the user, with its status.
fn client(status: u16, message: String) -> Fail {
    Fail::exception(status, "NzbDrone.Core.Exceptions.NzbDroneClientException", message)
}

/// Deletes the file from disk when it is there, then forgets it (`DeleteTrackFile(bookFile, subfolder)`).
fn delete_track_file(db: &SqliteDb, disk: &Disk, f: &Row, subfolder: &str) -> Answer<()> {
    if disk.file_exists(&f.s("Path")) {
        delete_file_on_disk(db, disk, f, subfolder)?;
    }
    forget(db, disk, f)
}

/// Sends a file to the recycling bin (`MediaFileDeletionService.DeleteFile`). A file in no root folder or in a
/// Calibre library can't be deleted (Readarr fails on the first and asks Calibre for the second, which Folio can't
/// reach yet): both answer Readarr's "Unable to delete book file".
fn delete_file_on_disk(db: &SqliteDb, disk: &Disk, f: &Row, subfolder: &str) -> Answer<()> {
    let path = f.s("Path");
    let fail = |why: String| {
        tracing::warn!("The book file {path} could not be deleted: {why}");
        client(500, "Unable to delete book file".into())
    };
    match best_root(db, &path)? {
        None => Err(fail("it is in no root folder".into())),
        Some(r) if r.calibre => Err(fail("it is in a Calibre library, which Folio can't change yet".into())),
        Some(_) => disk::recycle_file(db, disk, &path, subfolder).map_err(|e| fail(e.to_string())),
    }
}

/// Forgets a file (`MediaFileService.Delete`) and, when it belonged to an edition, does what Readarr does after
/// (`BookFileDeletedEvent`): a "deleted" history entry, its extra files to the recycling bin, and empty folders
/// removed when the `DeleteEmptyFolders` setting is on.
fn forget(db: &SqliteDb, disk: &Disk, f: &Row) -> Answer<()> {
    let id = f.i("Id");
    db.execute("DELETE FROM BookFiles WHERE Id = ?", vec![json!(id)])?;
    if f.i("EditionId") <= 0 {
        return Ok(());
    }
    // Readarr's handlers read the file's author and edition; without them they fail (and only log it).
    let found = |k: &str| f.get(k).is_some_and(|v| !v.is_null());
    let Some(author_path) = f.get("AuthorPath").and_then(Value::as_str).filter(|_| found("AuthorFound")).map(str::to_string) else {
        return Ok(());
    };
    if found("EditionFound") {
        db.execute(
            "INSERT INTO History (SourceTitle, Date, Quality, Data, EventType, DownloadId, AuthorId, BookId) VALUES (?, ?, ?, ?, ?, NULL, ?, ?)",
            vec![
                json!(f.s("Path")),
                json!(now_param()),
                json!(indented_quality(&doc(f.get("Quality")))),
                json!(indented_data(&[
                    ("reason", json!(MANUAL)),
                    ("releaseGroup", f.get("ReleaseGroup").cloned().unwrap_or(Value::Null)),
                    ("indexerFlags", json!(indexer_flags(f.i("IndexerFlags")))),
                ])),
                json!(BOOK_FILE_DELETED),
                json!(f.i("AuthorFound")),
                json!(f.i("EditionBookId")),
            ],
        )?;
    }
    for table in ["MetadataFiles", "ExtraFiles"] {
        for extra in db.query(&format!("SELECT RelativePath FROM {table} WHERE BookFileId = ?"), vec![json!(id)])? {
            let path = disk::combine(&author_path, &extra.s("RelativePath"));
            if disk.file_exists(&path) {
                let root = disk::parent(&author_path).unwrap_or_default();
                let sub = disk::parent(&path).and_then(|p| disk::relative(&root, &p)).unwrap_or_default();
                if let Err(e) = disk::recycle_file(db, disk, &path, &sub) {
                    tracing::warn!("The extra file {path} could not be deleted: {e}");
                }
            }
        }
        db.execute(&format!("DELETE FROM {table} WHERE BookFileId = ?"), vec![json!(id)])?;
    }
    if disk::config(db, "DeleteEmptyFolders")?.is_some_and(|v| v.eq_ignore_ascii_case("true")) {
        let book_folder = disk::parent(&f.s("Path")).unwrap_or_default();
        if disk.no_files(&author_path) {
            if let Err(e) = disk.delete_folder(&author_path) {
                tracing::warn!("The empty folder {author_path} could not be deleted: {e}");
            }
        } else if disk.no_files(&book_folder) {
            disk.remove_empty_subfolders(&book_folder);
        }
    }
    Ok(())
}

/// A stored quality (`{"quality": 3, "revision": {…}}`) as Readarr writes it into a history row: indented.
fn indented_quality(q: &Value) -> String {
    let r = q.get("revision").cloned().unwrap_or(Value::Null);
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
    format!(
        "{{\n  \"quality\": {},\n  \"revision\": {{\n    \"version\": {},\n    \"real\": {},\n    \"isRepack\": {}\n  }}\n}}",
        n(q, "quality"),
        r.get("version").and_then(Value::as_i64).unwrap_or(1),
        n(&r, "real"),
        r.get("isRepack").and_then(Value::as_bool).unwrap_or(false),
    )
}

/// A history row's data (`Dictionary<string, string>`) as Readarr writes it: indented, in the order given.
fn indented_data(fields: &[(&str, Value)]) -> String {
    let lines: Vec<String> = fields.iter().map(|(k, v)| format!("  \"{k}\": {v}")).collect();
    format!("{{\n{}\n}}", lines.join(",\n"))
}

/// Indexer flags as .NET writes the `[Flags]` enum `IndexerFlags`: the names of the set flags (`Freeleech,
/// Internal`), or the number when none or an unknown one is set.
fn indexer_flags(n: i64) -> String {
    const NAMES: [(i64, &str); 7] =
        [(1, "Freeleech"), (2, "Halfleech"), (4, "DoubleUpload"), (8, "Internal"), (16, "Scene"), (32, "Freeleech75"), (64, "Freeleech25")];
    let known: i64 = NAMES.iter().map(|(v, _)| v).sum();
    if n == 0 || n & !known != 0 {
        return n.to_string();
    }
    NAMES.iter().filter(|(v, _)| n & v != 0).map(|(_, name)| *name).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_values_are_written_like_readarr() {
        assert_eq!(indexer_flags(0), "0");
        assert_eq!(indexer_flags(9), "Freeleech, Internal");
        assert_eq!(indexer_flags(128), "128");
        assert_eq!(
            indented_data(&[("reason", json!("Manual")), ("releaseGroup", Value::Null), ("indexerFlags", json!("0"))]),
            "{\n  \"reason\": \"Manual\",\n  \"releaseGroup\": null,\n  \"indexerFlags\": \"0\"\n}"
        );
        assert_eq!(
            indented_quality(&json!({"quality": 1, "revision": {"version": 1, "real": 0, "isRepack": false}})),
            "{\n  \"quality\": 1,\n  \"revision\": {\n    \"version\": 1,\n    \"real\": 0,\n    \"isRepack\": false\n  }\n}"
        );
    }
}
