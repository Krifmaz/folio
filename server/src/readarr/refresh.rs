//! Readarr's author refresh (`RefreshAuthorService` with `RefreshBookService`, `RefreshEditionService`,
//! `RefreshSeriesService` and `RefreshSeriesBookLinkService`): an author's metadata, books, editions and series
//! brought up to what the metadata server has, filtered by the author's metadata profile. Then what Readarr does
//! once it has scanned the author's folder (`AuthorScannedHandler`): the monitoring chosen when the author was
//! added (`BookMonitoredService`), after which the add options are cleared.
//!
//! Readarr runs this as a command after an author is added. Folio runs it in the background the same way
//! ([`Jobs`]) until the command queue is ported (R5), and so far only for new authors. Not ported yet (each is
//! logged when it would happen): an author or book that moved to another id at the metadata server (Readarr's
//! merges), refetching a book the author's metadata no longer lists but the library keeps, writing tags into
//! files (`SyncTags`), searches, and the folder scan itself.

use super::bookinfo::{lock, ratings_json, AuthorInfo, Book, BookInfo, Edition, Failure, SeriesInfo, Shared};
use super::disk::Disk;
use super::json::{date, doc, now_param, stored_date, stored_doc};
use super::{authors, profiles, Fail};
use anyhow::{anyhow, bail, Result};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// .NET's `DateTime.MinValue` as Readarr stores it.
const MIN_DATE: &str = "0001-01-01 00:00:00Z";

/// The work Readarr does after it answers (its commands), run in the background; tests wait for it with
/// [`Jobs::idle`].
#[derive(Clone, Default)]
pub struct Jobs {
    /// how many jobs are running
    running: Arc<AtomicUsize>,
    /// woken when the last running job ends
    done: Arc<Notify>,
}

impl Jobs {
    /// Runs a job in the background.
    pub fn spawn(&self, job: impl Future<Output = ()> + Send + 'static) {
        self.running.fetch_add(1, Ordering::SeqCst);
        let jobs = self.clone();
        tokio::spawn(async move {
            job.await;
            if jobs.running.fetch_sub(1, Ordering::SeqCst) == 1 {
                jobs.done.notify_waiters();
            }
        });
    }

    /// Waits until no job is running.
    pub async fn idle(&self) {
        loop {
            let notified = self.done.notified();
            if self.running.load(Ordering::SeqCst) == 0 {
                return;
            }
            notified.await;
        }
    }
}

/// A Readarr error answer as an error to log.
fn failed(f: Fail) -> anyhow::Error {
    anyhow!("{}", f.body.map(|b| b.to_string()).unwrap_or_else(|| f.status.to_string()))
}

/// .NET's `NullReferenceException`.
fn null_reference() -> anyhow::Error {
    anyhow!("System.NullReferenceException: Object reference not set to an instance of an object.")
}

/// A stored time or one from the metadata, as text that sorts by time (`2013-10-01 00:00:00`).
fn time_key(v: &str) -> String {
    v.replacen('T', " ", 1).chars().take(19).collect()
}

/// Now as [`time_key`] gives it.
fn now_key() -> String {
    time_key(&now_param())
}

/// The ratings Readarr stores: `{votes, value, popularity}`.
fn ratings_doc(votes: i64, value: f64) -> Value {
    json!(ratings_json(votes, value).to_string())
}

/// A JSON document column as text, or nothing.
fn doc_text(v: &Value) -> Value {
    stored_doc(Some(v))
}

/// A list of values as a stored JSON document.
fn list_text(v: &[Value]) -> Value {
    json!(Value::Array(v.to_vec()).to_string())
}

/// The ratings Readarr stores for a book: `{votes, value, popularity}` with the average as .NET's `decimal`
/// writes it.
fn book_ratings_doc(b: &Book) -> Value {
    let popularity = super::json::double(b.value * b.votes as f64);
    json!(format!(r#"{{"votes":{},"value":{},"popularity":{popularity}}}"#, b.votes, b.value_text))
}

/// The average of a stored ratings document as it is written there.
fn stored_value_text(v: Option<&Value>) -> String {
    static VALUE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#""value"\s*:\s*([-0-9.eE+]+)"#).expect("a valid pattern"));
    v.and_then(Value::as_str).and_then(|t| VALUE.captures(t)).map(|c| c[1].to_string()).unwrap_or_else(|| "0".to_string())
}

/// The votes and average of a stored `{votes, value}` document.
fn stored_ratings(v: Option<&Value>) -> (i64, f64) {
    let r = doc(v);
    (r["votes"].as_i64().unwrap_or(0), r["value"].as_f64().unwrap_or(0.0))
}

/// A stored book as the model (`Book`), for comparing it with the metadata's.
fn book_from_row(r: &Row) -> Book {
    let (votes, value) = stored_ratings(r.get("Ratings"));
    Book {
        id: r.i("Id"),
        author_metadata_id: r.i("AuthorMetadataId"),
        foreign_book_id: r.s("ForeignBookId"),
        title_slug: r.s("TitleSlug"),
        title: r.get("Title").and_then(Value::as_str).map(str::to_string),
        release_date: date(r.get("ReleaseDate")).as_str().map(str::to_string),
        links: match doc(r.get("Links")) {
            Value::Array(a) => a,
            _ => Vec::new(),
        },
        genres: doc(r.get("Genres")),
        related_books: doc(r.get("RelatedBooks")),
        votes,
        value,
        value_text: stored_value_text(r.get("Ratings")),
        clean_title: r.s("CleanTitle"),
        monitored: r.i("Monitored") != 0,
        any_edition_ok: r.i("AnyEditionOk") != 0,
        last_info_sync: r.get("LastInfoSync").cloned().unwrap_or(Value::Null),
        last_search_time: r.get("LastSearchTime").cloned().unwrap_or(Value::Null),
        added: r.get("Added").cloned().unwrap_or(Value::Null),
        add_options: r.get("AddOptions").cloned().unwrap_or(Value::Null),
        ..Book::default()
    }
}

/// A stored edition as the model (`Edition`).
fn edition_from_row(r: &Row) -> Edition {
    let (votes, value) = stored_ratings(r.get("Ratings"));
    let text = |k: &str| r.get(k).cloned().unwrap_or(Value::Null);
    Edition {
        id: r.i("Id"),
        book_id: r.i("BookId"),
        foreign_edition_id: r.s("ForeignEditionId"),
        title_slug: r.s("TitleSlug"),
        isbn13: text("Isbn13"),
        asin: text("Asin"),
        title: r.s("Title"),
        language: text("Language"),
        overview: text("Overview"),
        format: text("Format"),
        is_ebook: r.i("IsEbook") != 0,
        disambiguation: text("Disambiguation"),
        publisher: text("Publisher"),
        page_count: r.i("PageCount"),
        release_date: date(r.get("ReleaseDate")).as_str().map(str::to_string),
        votes,
        value,
        images: super::bookinfo::shared(super::json::images(r.get("Images"))),
        links: match doc(r.get("Links")) {
            Value::Array(a) => a,
            _ => Vec::new(),
        },
        monitored: r.i("Monitored") != 0,
        manual_add: r.i("ManualAdd") != 0,
    }
}

/// Whether two editions are the same in every field Readarr compares (`Edition.Equals`).
fn same_edition(a: &Edition, b: &Edition) -> bool {
    let same_date = a.release_date.as_deref().map(time_key) == b.release_date.as_deref().map(time_key);
    a.id == b.id
        && a.book_id == b.book_id
        && a.foreign_edition_id == b.foreign_edition_id
        && a.title_slug == b.title_slug
        && a.isbn13 == b.isbn13
        && a.asin == b.asin
        && a.title == b.title
        && a.language == b.language
        && a.overview == b.overview
        && a.format == b.format
        && a.is_ebook == b.is_ebook
        && a.disambiguation == b.disambiguation
        && a.publisher == b.publisher
        && a.page_count == b.page_count
        && same_date
        && *lock(&a.images) == *lock(&b.images)
        && a.links == b.links
        && a.votes == b.votes
        && a.value == b.value
        && a.monitored == b.monitored
        && a.manual_add == b.manual_add
}

/// Whether a stored book and the metadata's (with the stored one's own fields) are the same (`Book.Equals`).
fn same_book(a: &Book, b: &Book) -> bool {
    let key = |d: &Option<String>| d.as_deref().map(time_key);
    a.id == b.id
        && a.author_metadata_id == b.author_metadata_id
        && a.foreign_book_id == b.foreign_book_id
        && a.title_slug == b.title_slug
        && a.title == b.title
        && key(&a.release_date) == key(&b.release_date)
        && a.links == b.links
        && a.genres == b.genres
        && a.related_books == b.related_books
        && a.votes == b.votes
        && a.value == b.value
        && a.clean_title == b.clean_title
        && a.monitored == b.monitored
        && a.any_edition_ok == b.any_edition_ok
        && a.last_info_sync == b.last_info_sync
        && a.last_search_time == b.last_search_time
        && a.added == b.added
}

/// Writes an author's metadata (`AuthorMetadataRepository.UpsertMany` of one): a new row, or the stored one
/// replaced when it differs. Gives its id and whether anything was written.
pub fn upsert_metadata(db: &SqliteDb, meta: &Row) -> kit_server_core::Result<(i64, bool)> {
    const COLUMNS: [&str; 18] = [
        "ForeignAuthorId",
        "TitleSlug",
        "Name",
        "Overview",
        "Disambiguation",
        "Gender",
        "Hometown",
        "Born",
        "Died",
        "Status",
        "Images",
        "Links",
        "Genres",
        "Ratings",
        "Aliases",
        "SortName",
        "NameLastFirst",
        "SortNameLastFirst",
    ];
    let value = |k: &str| -> Value {
        let v = meta.get(k).cloned().unwrap_or(Value::Null);
        if k == "Ratings" {
            let (votes, avg) = stored_ratings(Some(&v));
            return ratings_doc(votes, avg);
        }
        v
    };
    let values: Vec<Value> = COLUMNS.iter().map(|k| value(k)).collect();
    let existing =
        db.one("SELECT * FROM AuthorMetadata WHERE ForeignAuthorId = ?", vec![meta.get("ForeignAuthorId").cloned().unwrap_or_default()])?;
    match existing {
        Some(row) => {
            let id = row.i("Id");
            let same = COLUMNS.iter().zip(&values).all(|(k, v)| {
                let stored = row.get(*k).cloned().unwrap_or(Value::Null);
                match (stored.as_str(), v.as_str()) {
                    (Some(a), Some(b)) if a.starts_with(['{', '[']) => {
                        serde_json::from_str::<Value>(a).ok() == serde_json::from_str::<Value>(b).ok()
                    }
                    _ => stored == *v,
                }
            });
            if !same {
                let set = COLUMNS.iter().map(|k| format!("{k} = ?")).collect::<Vec<_>>().join(", ");
                let mut params = values;
                params.push(json!(id));
                db.execute(&format!("UPDATE AuthorMetadata SET {set} WHERE Id = ?"), params)?;
            }
            Ok((id, !same))
        }
        None => {
            let marks = super::json::marks(COLUMNS.len());
            let id = db.execute(&format!("INSERT INTO AuthorMetadata ({}) VALUES ({marks})", COLUMNS.join(", ")), values)?;
            Ok((id, true))
        }
    }
}

/// A book's row values, in the order of [`BOOK_COLUMNS`].
fn book_values(b: &Book) -> Vec<Value> {
    let add_options =
        if b.add_options.is_null() { json!({ "addType": "automatic", "searchForNewBook": false }) } else { doc(Some(&b.add_options)) };
    vec![
        json!(b.author_metadata_id),
        json!(b.foreign_book_id),
        json!(b.title_slug),
        json!(b.title),
        stored_date(b.release_date.as_ref().map(|d| json!(d)).as_ref()),
        list_text(&b.links),
        doc_text(&b.genres),
        book_ratings_doc(b),
        json!(b.clean_title),
        json!(b.monitored),
        json!(b.any_edition_ok),
        b.last_info_sync.clone(),
        b.added.clone(),
        doc_text(&add_options),
        doc_text(&b.related_books),
        b.last_search_time.clone(),
    ]
}

/// The columns a book is written with.
const BOOK_COLUMNS: [&str; 16] = [
    "AuthorMetadataId",
    "ForeignBookId",
    "TitleSlug",
    "Title",
    "ReleaseDate",
    "Links",
    "Genres",
    "Ratings",
    "CleanTitle",
    "Monitored",
    "AnyEditionOk",
    "LastInfoSync",
    "Added",
    "AddOptions",
    "RelatedBooks",
    "LastSearchTime",
];

/// An edition's row values, in the order of [`EDITION_COLUMNS`].
fn edition_values(e: &Edition) -> Vec<Value> {
    vec![
        json!(e.book_id),
        json!(e.foreign_edition_id),
        e.isbn13.clone(),
        e.asin.clone(),
        json!(e.title),
        json!(e.title_slug),
        e.language.clone(),
        e.overview.clone(),
        e.format.clone(),
        json!(e.is_ebook),
        e.disambiguation.clone(),
        e.publisher.clone(),
        json!(e.page_count),
        stored_date(e.release_date.as_ref().map(|d| json!(d)).as_ref()),
        list_text(&lock(&e.images)),
        list_text(&e.links),
        ratings_doc(e.votes, e.value),
        json!(e.monitored),
        json!(e.manual_add),
    ]
}

/// The columns an edition is written with.
const EDITION_COLUMNS: [&str; 19] = [
    "BookId",
    "ForeignEditionId",
    "Isbn13",
    "Asin",
    "Title",
    "TitleSlug",
    "Language",
    "Overview",
    "Format",
    "IsEbook",
    "Disambiguation",
    "Publisher",
    "PageCount",
    "ReleaseDate",
    "Images",
    "Links",
    "Ratings",
    "Monitored",
    "ManualAdd",
];

/// Inserts a row: its new id.
fn insert(db: &SqliteDb, table: &str, columns: &[&str], values: Vec<Value>) -> Result<i64> {
    Ok(db.execute(&format!("INSERT INTO {table} ({}) VALUES ({})", columns.join(", "), super::json::marks(columns.len())), values)?)
}

/// Writes every column of a row.
fn update(db: &SqliteDb, table: &str, columns: &[&str], mut values: Vec<Value>, id: i64) -> Result<()> {
    let set = columns.iter().map(|c| format!("{c} = ?")).collect::<Vec<_>>().join(", ");
    values.push(json!(id));
    db.execute(&format!("UPDATE {table} SET {set} WHERE Id = ?"), values)?;
    Ok(())
}

/// The number of files of an edition (`GetFilesByEdition`).
fn edition_files(db: &SqliteDb, id: i64) -> Result<usize> {
    Ok(db.query("SELECT Id FROM BookFiles WHERE EditionId = ?", vec![json!(id)])?.len())
}

/// Whether a book has files (`GetFilesByBook`).
fn book_has_files(db: &SqliteDb, id: i64) -> Result<bool> {
    Ok(!db.query("SELECT f.Id FROM BookFiles f JOIN Editions e ON e.Id = f.EditionId WHERE e.BookId = ?", vec![json!(id)])?.is_empty())
}

/// Whether a new book is monitored (`MonitorNewBookService.ShouldMonitorNewBook`): `monitor_new_items` 0 (all)
/// always, 1 (none) never, 2 (new) when it comes out no earlier than the newest of the books already up to date.
fn should_monitor_new_book(book: &Book, up_to_date: &[Book], monitor_new_items: i64) -> Result<bool> {
    match monitor_new_items {
        1 => Ok(false),
        0 => Ok(true),
        2 => {
            let key = |b: &Book| b.release_date.as_deref().map(time_key).unwrap_or_default();
            let newest = up_to_date.iter().map(key).max().unwrap_or_default();
            Ok(key(book) >= newest)
        }
        n => bail!("System.NotImplementedException: Unknown new item monitor type {n}"),
    }
}

/// The editions of a book, sorted like `SortChildren`: up to date, added, updated (stored editions changed by the
/// metadata) and deleted (stored editions the metadata no longer has).
#[derive(Default)]
struct SortedEditions {
    /// stored editions the same as the metadata's
    up_to_date: Vec<Edition>,
    /// the metadata's new editions
    added: Vec<Shared<Edition>>,
    /// stored editions the metadata changed
    updated: Vec<Edition>,
    /// stored editions the metadata no longer has
    deleted: Vec<Edition>,
}

/// One edition of the future ones, wherever it is kept.
enum Candidate<'a> {
    /// a stored edition
    Stored(&'a mut Edition),
    /// a new one
    New(&'a Shared<Edition>),
}

/// Monitors exactly one of a book's editions (`MonitorSingleEdition`): the monitored one when there is one,
/// else the one with the most files and then the most popular. Up-to-date editions it changes are written too.
fn monitor_single_edition(db: &SqliteDb, c: &mut SortedEditions) -> Result<()> {
    for e in &mut c.deleted {
        e.monitored = false;
    }
    let monitored_count = c.up_to_date.iter().filter(|e| e.monitored).count()
        + c.added.iter().filter(|e| lock(e).monitored).count()
        + c.updated.iter().filter(|e| e.monitored).count();
    if monitored_count == 1 {
        return Ok(());
    }
    let only_monitored = monitored_count > 0;
    let mut chosen_up_to_date: Vec<usize> = Vec::new();
    {
        let mut future: Vec<(Option<usize>, Candidate)> = Vec::new();
        for (i, e) in c.up_to_date.iter_mut().enumerate() {
            future.push((Some(i), Candidate::Stored(e)));
        }
        for e in &c.added {
            future.push((None, Candidate::New(e)));
        }
        for e in c.updated.iter_mut() {
            future.push((None, Candidate::Stored(e)));
        }
        let monitored = |f: &Candidate| match f {
            Candidate::Stored(e) => e.monitored,
            Candidate::New(e) => lock(e).monitored,
        };
        let pool: Vec<usize> = (0..future.len()).filter(|i| !only_monitored || monitored(&future[*i].1)).collect();
        if pool.is_empty() {
            return Ok(());
        }
        let mut score = Vec::new();
        for &i in &pool {
            let (id, popularity) = match &future[i].1 {
                Candidate::Stored(e) => (e.id, e.popularity()),
                Candidate::New(e) => {
                    let e = lock(e);
                    (e.id, e.popularity())
                }
            };
            let files = if id > 0 { edition_files(db, id)? } else { 0 };
            score.push((files, popularity));
        }
        let mut best = 0;
        for k in 1..pool.len() {
            let (f, p) = score[k];
            let (bf, bp) = score[best];
            if f > bf || (f == bf && p > bp) {
                best = k;
            }
        }
        let to_monitor = pool[best];
        for &i in &pool {
            match &mut future[i].1 {
                Candidate::Stored(e) => e.monitored = i == to_monitor,
                Candidate::New(e) => lock(e).monitored = i == to_monitor,
            }
            if let Some(u) = future[i].0 {
                chosen_up_to_date.push(u);
            }
        }
    }
    let mut moved = Vec::new();
    let mut kept = Vec::new();
    for (i, e) in std::mem::take(&mut c.up_to_date).into_iter().enumerate() {
        if chosen_up_to_date.contains(&i) {
            moved.push(e);
        } else {
            kept.push(e);
        }
    }
    c.up_to_date = kept;
    c.updated.extend(moved);
    Ok(())
}

/// The editions of the metadata, each once (`DistinctBy(ForeignEditionId)`).
fn distinct_editions(list: &[Shared<Edition>]) -> Vec<Shared<Edition>> {
    let mut seen = HashSet::new();
    list.iter().filter(|e| seen.insert(lock(e).foreign_edition_id.clone())).cloned().collect()
}

/// An edition with the metadata's fields (`Edition.UseMetadataFrom`): a blank description and no covers keep the
/// stored ones.
fn use_edition_metadata(local: &mut Edition, remote: &Edition) {
    local.foreign_edition_id = remote.foreign_edition_id.clone();
    local.title_slug = remote.title_slug.clone();
    local.isbn13 = remote.isbn13.clone();
    local.asin = remote.asin.clone();
    local.title = remote.title.clone();
    local.language = remote.language.clone();
    if remote.overview.as_str().is_some_and(|o| !o.trim().is_empty()) {
        local.overview = remote.overview.clone();
    }
    local.format = remote.format.clone();
    local.is_ebook = remote.is_ebook;
    local.disambiguation = remote.disambiguation.clone();
    local.publisher = remote.publisher.clone();
    local.page_count = remote.page_count;
    local.release_date = remote.release_date.clone();
    if !lock(&remote.images).is_empty() {
        local.images = remote.images.clone();
    }
    local.links = remote.links.clone();
    local.votes = remote.votes;
    local.value = remote.value;
}

/// Refreshes one book of the author (`RefreshBookService.RefreshEntityInfo`): its fields from the metadata's
/// book `remote`, then its editions. `stored` is the library's book (a new one is the metadata's itself).
fn refresh_book(db: &SqliteDb, disk: &Disk, stored: &Book, remote: Option<&Shared<Book>>, meta_id: i64) -> Result<()> {
    let Some(remote) = remote else {
        let manual = super::books::add_options(Some(&stored.add_options))["addType"] == json!("manual");
        if !manual && !book_has_files(db, stored.id)? {
            tracing::warn!("Readarr: book {} not found in the metadata, deleting it", stored.foreign_book_id);
            super::delete::book(db, disk, stored.id, false, false).map_err(failed)?;
        } else {
            tracing::warn!(
                "Readarr: book {} isn't in its author's metadata any more; Folio doesn't fetch it on its own yet",
                stored.foreign_book_id
            );
        }
        return Ok(());
    };
    let (book_id, remote_editions) = {
        let mut b = lock(remote);
        b.id = stored.id;
        b.author_metadata_id = meta_id;
        b.last_info_sync = json!(now_param());
        (b.id, distinct_editions(&b.editions))
    };
    let ids: Vec<Value> = remote_editions.iter().map(|e| json!(lock(e).foreign_edition_id.clone())).collect();
    let local = db.query(
        &format!("SELECT * FROM Editions WHERE BookId = ? OR ForeignEditionId IN ({}) ORDER BY Id", super::json::marks(ids.len().max(1))),
        std::iter::once(json!(book_id)).chain(if ids.is_empty() { vec![Value::Null] } else { ids }).collect(),
    )?;
    let mut local: Vec<Edition> = local.iter().map(edition_from_row).collect();
    let mut c = SortedEditions::default();
    for r in &remote_editions {
        let foreign = lock(r).foreign_edition_id.clone();
        let matching: Vec<usize> = local.iter().enumerate().filter(|(_, e)| e.foreign_edition_id == foreign).map(|(i, _)| i).collect();
        if matching.len() > 1 {
            bail!("System.InvalidOperationException: Sequence contains more than one matching element");
        }
        match matching.first() {
            Some(&i) => {
                let mut existing = local.remove(i);
                existing.book_id = book_id;
                {
                    let mut e = lock(r);
                    e.id = existing.id;
                    e.book_id = existing.book_id;
                    e.monitored = existing.monitored;
                    e.manual_add = existing.manual_add;
                }
                if same_edition(&existing, &lock(r)) {
                    c.up_to_date.push(existing);
                } else {
                    c.updated.push(existing);
                }
            }
            None => {
                lock(r).book_id = book_id;
                c.added.push(r.clone());
            }
        }
    }
    c.deleted = local;
    monitor_single_edition(db, &mut c)?;
    if c.added.iter().any(|e| lock(e).id != 0) {
        bail!("System.InvalidOperationException: Can't insert model with existing ID != 0");
    }
    for e in &c.added {
        let id = insert(db, "Editions", &EDITION_COLUMNS, edition_values(&lock(e)))?;
        lock(e).id = id;
    }
    for e in &c.deleted {
        db.execute("DELETE FROM Editions WHERE Id = ?", vec![json!(e.id)])?;
    }
    for e in &mut c.updated {
        let remote = remote_editions
            .iter()
            .find(|r| lock(r).foreign_edition_id == e.foreign_edition_id)
            .ok_or_else(|| anyhow!("System.InvalidOperationException: Sequence contains no matching element"))?;
        let r = lock(remote).clone();
        use_edition_metadata(e, &r);
        update(db, "Editions", &EDITION_COLUMNS, edition_values(e), e.id)?;
    }
    let b = lock(remote).clone();
    update(db, "Books", &BOOK_COLUMNS, book_values(&b), b.id)
}

/// The series of the author, refreshed (`RefreshSeriesService.RefreshSeriesInfo`): the series of its books in the
/// library, with their links to those books. A series the metadata lists without books stops it (Readarr's
/// `NullReferenceException`).
fn refresh_series(db: &SqliteDb, meta_id: i64, series: &[SeriesInfo], foreign_author_id: &str) -> Result<()> {
    let by_author = db.query(
        "SELECT DISTINCT s.* FROM Series s JOIN SeriesBookLink l ON l.SeriesId = s.Id JOIN Books b ON b.Id = l.BookId
         WHERE b.AuthorMetadataId = ? ORDER BY s.Id",
        vec![json!(meta_id)],
    )?;
    let ids: Vec<Value> = series.iter().map(|s| json!(s.foreign_series_id)).collect();
    let by_series = if ids.is_empty() {
        Vec::new()
    } else {
        db.query(&format!("SELECT * FROM Series WHERE ForeignSeriesId IN ({}) ORDER BY Id", super::json::marks(ids.len())), ids)?
    };
    let mut existing: Vec<Row> = Vec::new();
    for s in by_author.into_iter().chain(by_series) {
        if !existing.iter().any(|e| e.s("ForeignSeriesId") == s.s("ForeignSeriesId")) {
            existing.push(s);
        }
    }
    let books: HashSet<String> = db
        .query("SELECT ForeignBookId FROM Books WHERE AuthorMetadataId = ?", vec![json!(meta_id)])?
        .iter()
        .map(|b| b.s("ForeignBookId"))
        .collect();
    // The series with links to the author's books in the library, each with only those links.
    let mut remote: Vec<(&SeriesInfo, Vec<&super::bookinfo::SeriesItem>)> = Vec::new();
    for s in series {
        let items = s.items.as_ref().ok_or_else(null_reference)?;
        let links: Vec<_> = items.iter().filter(|l| books.contains(&lock(&l.book).foreign_book_id)).collect();
        if !links.is_empty() {
            remote.push((s, links));
        }
    }
    let mut all: Vec<(i64, Row)> = Vec::new();
    let mut added_ids: HashSet<String> = HashSet::new();
    for (s, _) in &remote {
        if existing.iter().any(|e| e.s("ForeignSeriesId") == s.foreign_series_id) || !added_ids.insert(s.foreign_series_id.clone()) {
            continue;
        }
        let title = s.title.clone().ok_or_else(|| anyhow!("SQLite Error 19: 'NOT NULL constraint failed: Series.Title'."))?;
        let id = db.execute(
            "INSERT INTO Series (ForeignSeriesId, Title, Description, Numbered, WorkCount, PrimaryWorkCount) VALUES (?, ?, ?, 0, 0, 0)",
            vec![json!(s.foreign_series_id), json!(title), json!(s.description)],
        )?;
        let row = db.one("SELECT * FROM Series WHERE Id = ?", vec![json!(id)])?.unwrap_or_default();
        all.push((id, row));
    }
    all.extend(existing.into_iter().map(|e| (e.i("Id"), e)));
    for (id, local) in all {
        let local_links = db.query(
            "SELECT l.* FROM SeriesBookLink l JOIN Books b ON b.Id = l.BookId JOIN AuthorMetadata m ON m.Id = b.AuthorMetadataId
             WHERE l.SeriesId = ? AND m.ForeignAuthorId = ? ORDER BY l.Id",
            vec![json!(id), json!(foreign_author_id)],
        )?;
        let Some((s, links)) = remote.iter().find(|(s, _)| s.foreign_series_id == local.s("ForeignSeriesId")) else {
            for l in &local_links {
                db.execute("DELETE FROM SeriesBookLink WHERE Id = ?", vec![json!(l.i("Id"))])?;
            }
            if db.query("SELECT Id FROM SeriesBookLink WHERE SeriesId = ?", vec![json!(id)])?.is_empty() {
                db.execute("DELETE FROM Series WHERE Id = ?", vec![json!(id)])?;
            }
            continue;
        };
        let mut deleted = local_links.clone();
        let mut added = Vec::new();
        let mut changed = Vec::new();
        for l in links {
            let book_id = lock(&l.book).id;
            let matching: Vec<usize> = deleted.iter().enumerate().filter(|(_, x)| x.i("BookId") == book_id).map(|(i, _)| i).collect();
            if matching.len() > 1 {
                bail!("System.InvalidOperationException: Sequence contains more than one matching element");
            }
            match matching.first() {
                Some(&i) => {
                    let stored = deleted.remove(i);
                    let same = stored.get("Position").and_then(Value::as_str) == l.position.as_deref()
                        && stored.i("SeriesPosition") == l.series_position
                        && (stored.i("IsPrimary") != 0) == l.primary;
                    if !same {
                        changed.push((stored.i("Id"), *l));
                    }
                }
                None => added.push((book_id, *l)),
            }
        }
        for (book_id, l) in added {
            db.execute(
                "INSERT INTO SeriesBookLink (SeriesId, BookId, Position, IsPrimary, SeriesPosition) VALUES (?, ?, ?, ?, ?)",
                vec![json!(id), json!(book_id), json!(l.position), json!(l.primary), json!(l.series_position)],
            )?;
        }
        for l in &deleted {
            db.execute("DELETE FROM SeriesBookLink WHERE Id = ?", vec![json!(l.i("Id"))])?;
        }
        for (link_id, l) in changed {
            db.execute(
                "UPDATE SeriesBookLink SET Position = ?, SeriesPosition = ?, IsPrimary = ? WHERE Id = ?",
                vec![json!(l.position), json!(l.series_position), json!(l.primary), json!(link_id)],
            )?;
        }
        let title = s.title.clone().ok_or_else(|| anyhow!("SQLite Error 19: 'NOT NULL constraint failed: Series.Title'."))?;
        db.execute(
            "UPDATE Series SET ForeignSeriesId = ?, Title = ?, Description = ?, Numbered = 0, WorkCount = 0, PrimaryWorkCount = 0 WHERE Id = ?",
            vec![json!(s.foreign_series_id), json!(title), json!(s.description), json!(id)],
        )?;
    }
    Ok(())
}

/// The author's books in the metadata the profile takes, each once and without the excluded ones
/// (`GetRemoteChildren`).
fn remote_books(db: &SqliteDb, data: &AuthorInfo, profile_id: i64) -> Result<Vec<Shared<Book>>> {
    let filtered = profiles::filter_books(db, data, profile_id)?;
    let mut seen = HashSet::new();
    let all: Vec<Shared<Book>> = filtered.into_iter().filter(|b| seen.insert(lock(b).foreign_book_id.clone())).collect();
    let excluded: HashSet<String> =
        db.query("SELECT ForeignId FROM ImportListExclusions", vec![])?.iter().map(|r| r.s("ForeignId")).collect();
    Ok(all.into_iter().filter(|b| !excluded.contains(&lock(b).foreign_book_id)).collect())
}

/// Refreshes the author (`RefreshEntityInfo` for an author): its metadata, then its books, then the author, then
/// its series.
async fn refresh_author_info(db: &SqliteDb, disk: &Disk, info: &BookInfo, author: &Row) -> Result<()> {
    let foreign = author.s("ForeignAuthorId");
    let data = match info.get_author_info(db, &foreign, true).await {
        Ok(d) => d,
        Err(Failure::AuthorNotFound) => {
            tracing::error!("Readarr: could not find author with id {foreign}");
            let files = db.query(
                "SELECT f.Id FROM BookFiles f JOIN Editions e ON e.Id = f.EditionId JOIN Books b ON b.Id = e.BookId WHERE b.AuthorMetadataId = ?",
                vec![json!(author.i("AuthorMetadataId"))],
            )?;
            if files.is_empty() {
                tracing::warn!("Readarr: author {foreign} not found in the metadata, deleting it");
                super::delete::author(db, disk, author.i("Id"), false, false).map_err(failed)?;
            }
            return Ok(());
        }
        Err(e) => bail!("{}", e.describe()),
    };
    let (meta_id, _) = upsert_metadata(db, &data.meta)?;
    if author.i("AuthorMetadataId") != meta_id {
        bail!("author {foreign} moved to other metadata; Folio doesn't merge authors yet");
    }
    let remote = remote_books(db, &data, author.i("MetadataProfileId"))?;
    let ids: Vec<Value> = remote.iter().map(|b| json!(lock(b).foreign_book_id.clone())).collect();
    let local_rows = db.query(
        &format!(
            "SELECT * FROM Books WHERE AuthorMetadataId = ? OR ForeignBookId IN ({}) ORDER BY Id",
            super::json::marks(ids.len().max(1))
        ),
        std::iter::once(json!(meta_id)).chain(if ids.is_empty() { vec![Value::Null] } else { ids }).collect(),
    )?;
    let mut local: Vec<Book> = local_rows.iter().map(book_from_row).collect();
    let mut up_to_date: Vec<(Book, Shared<Book>)> = Vec::new();
    let mut updated: Vec<(Book, Shared<Book>)> = Vec::new();
    let mut added: Vec<Shared<Book>> = Vec::new();
    for r in &remote {
        let foreign_book = lock(r).foreign_book_id.clone();
        let matching: Vec<usize> = local.iter().enumerate().filter(|(_, b)| b.foreign_book_id == foreign_book).map(|(i, _)| i).collect();
        if matching.len() > 1 {
            bail!("System.InvalidOperationException: Sequence contains more than one matching element");
        }
        match matching.first() {
            Some(&i) => {
                let mut existing = local.remove(i);
                existing.author_metadata_id = meta_id;
                let same = {
                    let mut b = lock(r);
                    b.id = existing.id;
                    b.author_metadata_id = existing.author_metadata_id;
                    b.monitored = existing.monitored;
                    b.any_edition_ok = existing.any_edition_ok;
                    b.last_info_sync = existing.last_info_sync.clone();
                    b.last_search_time = existing.last_search_time.clone();
                    b.added = existing.added.clone();
                    b.add_options = existing.add_options.clone();
                    same_book(&existing, &b)
                };
                if same {
                    up_to_date.push((existing, r.clone()));
                } else {
                    updated.push((existing, r.clone()));
                }
            }
            None => {
                let mut b = lock(r);
                b.author_metadata_id = meta_id;
                b.added = json!(now_param());
                b.last_info_sync = json!(MIN_DATE);
                b.monitored = author.i("Monitored") != 0;
                drop(b);
                added.push(r.clone());
            }
        }
    }
    let deleted = local;
    let current: Vec<Book> = up_to_date.iter().map(|(b, _)| b.clone()).collect();
    for b in &added {
        let monitored = should_monitor_new_book(&lock(b), &current, author.i("MonitorNewItems"))?;
        lock(b).monitored = monitored;
    }
    if added.iter().any(|b| lock(b).author_metadata_id == 0) {
        bail!("System.InvalidOperationException: Cannot insert book with AuthorMetadataId = 0");
    }
    if added.iter().any(|b| lock(b).id != 0) {
        bail!("System.InvalidOperationException: Can't insert model with existing ID != 0");
    }
    for b in &added {
        let id = insert(db, "Books", &BOOK_COLUMNS, book_values(&lock(b)))?;
        lock(b).id = id;
    }
    for (stored, r) in &up_to_date {
        refresh_book(db, disk, stored, Some(r), meta_id)?;
    }
    for r in &added {
        let stored = lock(r).clone();
        refresh_book(db, disk, &stored, Some(r), meta_id)?;
    }
    for (stored, r) in &updated {
        refresh_book(db, disk, stored, Some(r), meta_id)?;
    }
    for stored in &deleted {
        refresh_book(db, disk, stored, None, meta_id)?;
    }
    db.execute("UPDATE Authors SET LastInfoSync = ? WHERE Id = ?", vec![json!(now_param()), json!(author.i("Id"))])?;
    refresh_series(db, meta_id, &data.series, &foreign)
}

/// `MonitorTypes` from a stored or sent name or number.
fn monitor_type(v: &Value) -> Option<i64> {
    const NAMES: [&str; 8] = ["all", "future", "missing", "existing", "latest", "first", "none", "unknown"];
    match v {
        Value::Number(n) => n.as_i64().filter(|n| (0..8).contains(n)),
        Value::String(s) => NAMES.iter().position(|n| n.eq_ignore_ascii_case(s)).map(|p| p as i64).or_else(|| s.parse().ok()),
        _ => None,
    }
}

/// What Readarr does once the author's folder is scanned (`AuthorScannedHandler`): the monitoring of the add
/// options (`BookMonitoredService.SetBookMonitoredStatus`), the add options cleared, and the books' "search when
/// added" flags cleared (`SearchForRecentlyAdded`; Folio searches nothing yet).
fn after_scan(db: &SqliteDb, author_id: i64) -> Result<()> {
    let Some(author) = authors::find(db, author_id)? else { return Ok(()) };
    let books = super::books::rows(db, "b.AuthorMetadataId = ?", vec![json!(author.i("AuthorMetadataId"))])?;
    let options = doc(author.get("AddOptions"));
    if !options.is_null() {
        let with_files: HashSet<i64> = db
            .query(
                "SELECT DISTINCT b.Id FROM Books b JOIN Editions e ON e.BookId = b.Id JOIN BookFiles f ON f.EditionId = e.Id
                 WHERE b.AuthorMetadataId = ? AND e.Monitored = 1",
                vec![json!(author.i("AuthorMetadataId"))],
            )?
            .iter()
            .map(|r| r.i("Id"))
            .collect();
        let now = now_key();
        let release = |b: &Row| b.get("ReleaseDate").and_then(Value::as_str).map(time_key);
        let without_files: HashSet<i64> =
            books.iter().filter(|b| !with_files.contains(&b.i("Id")) && release(b).is_some_and(|d| d <= now)).map(|b| b.i("Id")).collect();
        let mut monitored: Vec<bool> = books.iter().map(|b| b.i("Monitored") != 0).collect();
        let wanted: Vec<String> = options["booksToMonitor"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let ids: Vec<i64> = books.iter().map(|b| b.i("Id")).collect();
        if !wanted.is_empty() {
            for (i, b) in books.iter().enumerate() {
                monitored[i] = wanted.contains(&b.s("ForeignBookId"));
            }
        } else {
            let set = |m: &mut Vec<bool>, which: &dyn Fn(i64) -> bool, to: bool| {
                for (i, id) in ids.iter().enumerate() {
                    if which(*id) {
                        m[i] = to;
                    }
                }
            };
            match monitor_type(&options["monitor"]).unwrap_or(0) {
                0 => set(&mut monitored, &|_| true, true),
                1 => {
                    set(&mut monitored, &|id| with_files.contains(&id), false);
                    set(&mut monitored, &|id| without_files.contains(&id), false);
                }
                2 => {
                    set(&mut monitored, &|id| with_files.contains(&id), false);
                    set(&mut monitored, &|id| without_files.contains(&id), true);
                }
                3 => {
                    set(&mut monitored, &|id| with_files.contains(&id), true);
                    set(&mut monitored, &|id| without_files.contains(&id), false);
                }
                4 | 5 => {
                    set(&mut monitored, &|_| true, false);
                    // Readarr's OrderBy is stable and puts books without a date first.
                    let mut order: Vec<usize> = (0..books.len()).collect();
                    let key = |i: &usize| release(&books[*i]).unwrap_or_default();
                    if monitor_type(&options["monitor"]) == Some(4) {
                        order.sort_by_key(|i| std::cmp::Reverse(key(i)));
                    } else {
                        order.sort_by_key(key);
                    }
                    if let Some(&i) = order.first() {
                        monitored[i] = true;
                    }
                }
                6 => set(&mut monitored, &|_| true, false),
                n => bail!("System.ArgumentOutOfRangeException: Specified argument was out of the range of valid values. ({n})"),
            }
        }
        for (b, m) in books.iter().zip(&monitored) {
            db.execute("UPDATE Books SET Monitored = ? WHERE Id = ?", vec![json!(m), json!(b.i("Id"))])?;
        }
        db.execute("UPDATE Authors SET AddOptions = NULL WHERE Id = ?", vec![json!(author_id)])?;
    }
    for b in &books {
        let mut o = super::books::add_options(b.get("AddOptions"));
        if o["searchForNewBook"] == json!(true) {
            o["searchForNewBook"] = json!(false);
            db.execute("UPDATE Books SET AddOptions = ? WHERE Id = ?", vec![json!(o.to_string()), json!(b.i("Id"))])?;
        }
    }
    Ok(())
}

/// Refreshes a newly added author and applies its add options (`RefreshAuthorCommand` with `isNewAuthor`).
/// Failures are logged, as Readarr logs them.
pub async fn new_author(db: SqliteDb, disk: Disk, info: BookInfo, author_id: i64) {
    let Ok(Some(author)) = authors::find(&db, author_id) else { return };
    if let Err(e) = refresh_author_info(&db, &disk, &info, &author).await {
        tracing::error!("Readarr: couldn't refresh info for author {}: {e:#}", author.s("ForeignAuthorId"));
    }
    if let Err(e) = after_scan(&db, author_id) {
        tracing::error!("Readarr: couldn't apply the add options of author {author_id}: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_books_are_monitored_like_readarr() {
        let book = |d: Option<&str>| Book { release_date: d.map(str::to_string), ..Book::default() };
        let existing = [book(Some("2014-01-01T00:00:00Z")), book(None)];
        assert!(should_monitor_new_book(&book(None), &existing, 0).unwrap());
        assert!(!should_monitor_new_book(&book(None), &existing, 1).unwrap());
        assert!(should_monitor_new_book(&book(Some("2015-01-01T00:00:00Z")), &existing, 2).unwrap());
        assert!(!should_monitor_new_book(&book(Some("2013-01-01T00:00:00Z")), &existing, 2).unwrap());
        assert!(should_monitor_new_book(&book(None), &[], 2).unwrap());
        assert_eq!(monitor_type(&json!("Latest")), Some(4));
        assert_eq!(monitor_type(&json!(6)), Some(6));
    }

    #[tokio::test]
    async fn jobs_report_when_they_are_done() {
        let jobs = Jobs::default();
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        jobs.spawn(async move {
            let _ = rx.await;
        });
        let waiting = tokio::spawn({
            let jobs = jobs.clone();
            async move { jobs.idle().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        tx.send(()).unwrap();
        waiting.await.unwrap();
        jobs.idle().await;
    }
}
