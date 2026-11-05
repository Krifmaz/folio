//! Readarr's book files (`Readarr.Api.V1/BookFiles`): the files of authors and books, and files not matched to
//! any book, as `GET /api/v1/bookfile` sends them.
//!
//! ```text
//! GET /api/v1/bookfile?authorId=|bookId=&bookId=|bookFileIds=&bookFileIds=|unmapped=true   files
//! GET /api/v1/bookfile/{id}                                                                one file, with its tags
//! ```
//!
//! Tags are read like Readarr reads a file it can't parse (an empty set with the quality of the extension); the
//! tags of real EPUB, PDF, MOBI and audio files come with the import's port.

use super::json::{date_or_min, doc, marks, object, Query};
use super::{authors, books, quality, Answer, Fail, Key};
use crate::AppState;
use axum::extract::{Path, RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// The book file routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/bookfile", get(all)).route("/api/v1/bookfile/{id}", get(one))
}

/// A file with its edition's book and its author (`MediaFileRepository`'s joins).
const SELECT: &str = "SELECT f.*, e.Id AS EditionFound, e.BookId AS EditionBookId, a.Id AS AuthorFound, a.QualityProfileId AS AuthorProfile
    FROM BookFiles f LEFT JOIN Editions e ON f.EditionId = e.Id LEFT JOIN Books b ON e.BookId = b.Id
    LEFT JOIN Authors a ON b.AuthorMetadataId = a.AuthorMetadataId";

/// The files matching `filter` (SQL after `WHERE`), in the order they were added.
fn rows(db: &SqliteDb, filter: &str, params: Vec<Value>) -> kit_server_core::Result<Vec<Row>> {
    db.query(&format!("{SELECT} WHERE {filter} ORDER BY f.Id"), params)
}

/// A file's audio details as Readarr sends them (`MediaInfoResource`).
fn media_info(m: &Value) -> Value {
    if m.is_null() {
        return Value::Null;
    }
    let n = |k: &str| m.get(k).and_then(Value::as_i64).unwrap_or(0);
    let format = m.get("audioFormat").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let codec = [
        ("wavpack", "WavPack"),
        ("flac", "FLAC"),
        ("alac", "ALAC"),
        ("aac", "AAC"),
        ("mp4a", "AAC"),
        ("mp3", "MP3"),
        ("mpeg", "MP3"),
        ("opus", "OPUS"),
        ("vorbis", "OGG"),
        ("ogg", "OGG"),
        ("pcm", "PCM"),
        ("wav", "PCM"),
        ("wma", "WMA"),
        ("ape", "APE"),
        ("mp2", "MP2"),
        ("mp1", "MP1"),
    ]
    .iter()
    .find(|(k, _)| format.contains(k))
    .map(|(_, v)| *v)
    .unwrap_or("Unknown");
    let khz = n("audioSampleRate") as f64 / 1000.0;
    let rate = if (khz * 10.0).round() % 10.0 == 0.0 { format!("{}kHz", khz.round()) } else { format!("{:.1}kHz", khz) };
    json!({
        "audioChannels": n("audioChannels"),
        "audioBitRate": format!("{} kbps", n("audioBitrate")),
        "audioCodec": codec,
        "audioBits": if n("audioBits") == 0 { String::new() } else { format!("{}bit", n("audioBits")) },
        "audioSampleRate": rate,
        "id": 0,
    })
}

/// A file as `BookFileResource.ToResource` makes it: with its author's id, cutoff and indexer flags when it
/// belongs to a book of an author Readarr has (`with_author`), else without.
fn resource(db: &SqliteDb, f: &Row, with_author: bool) -> kit_server_core::Result<Value> {
    let stored = doc(f.get("Quality"));
    let mapped = with_author && f.i("EditionId") > 0 && f.get("AuthorFound").is_some_and(|v| !v.is_null());
    let cutoff = if mapped { quality::cutoff_not_met(db, f.i("AuthorProfile"), &stored)? } else { false };
    Ok(object([
        ("authorId", json!(if mapped { f.i("AuthorFound") } else { 0 })),
        ("bookId", json!(f.i("EditionBookId"))),
        ("path", f.get("Path").cloned().unwrap_or(Value::Null)),
        ("size", json!(f.i("Size"))),
        ("dateAdded", date_or_min(f.get("DateAdded"))),
        ("quality", quality::model(&stored)),
        ("qualityWeight", json!(quality::weight(&stored))),
        ("indexerFlags", if mapped { json!(f.i("IndexerFlags")) } else { Value::Null }),
        ("mediaInfo", media_info(&doc(f.get("MediaInfo")))),
        ("qualityCutoffNotMet", json!(cutoff)),
        ("id", json!(f.i("Id"))),
    ]))
}

/// A file's tags as Readarr reads them when it can't parse the file: none, with the quality of its extension.
fn tags(path: &str) -> Value {
    json!({
        "authors": [],
        "discNumber": 0,
        "discCount": 0,
        "year": 0,
        "duration": "00:00:00",
        "quality": quality::from_extension(path),
        "trackNumbers": [],
    })
}

/// `GET /api/v1/bookfile`.
async fn all(State(st): State<AppState>, _: Key, RawQuery(q): RawQuery) -> Answer {
    let db = &st.readarr;
    let q = Query::of(q.as_deref());
    let author = q.int("authorId");
    let file_ids = q.ints("bookFileIds");
    let book_ids = q.ints("bookId");
    let unmapped = q.bool("unmapped");
    if author.is_none() && file_ids.is_empty() && book_ids.is_empty() && unmapped.is_none() {
        return Err(Fail::exception(
            500,
            "NzbDrone.Core.Exceptions.BadRequestException",
            "authorId, bookId, bookFileIds or unmapped must be provided",
        ));
    }
    let mut out = Vec::new();
    if unmapped == Some(true) {
        for f in rows(db, "f.EditionId = 0", vec![])? {
            out.push(resource(db, &f, false)?);
        }
    } else if let (Some(author), true) = (author, book_ids.is_empty()) {
        authors::get_or_404(db, author)?;
        for f in rows(db, "a.Id = ?", vec![json!(author)])? {
            out.push(resource(db, &f, true)?);
        }
    } else if !book_ids.is_empty() {
        for id in book_ids {
            books::get_or_404(db, id)?;
            for f in rows(db, "b.Id = ?", vec![json!(id)])? {
                out.push(resource(db, &f, true)?);
            }
        }
    } else if !file_ids.is_empty() {
        let found = rows(db, &format!("f.Id IN ({})", marks(file_ids.len())), file_ids.iter().map(|i| json!(i)).collect())?;
        if found.len() != file_ids.len() {
            return Err(Fail::exception(
                500,
                "System.ApplicationException",
                format!("Expected query to return {} rows but returned {}", file_ids.len(), found.len()),
            ));
        }
        for f in found {
            out.push(resource(db, &f, true)?);
        }
    }
    Ok(Json(Value::Array(out)))
}

/// `GET /api/v1/bookfile/{id}`: the file with its tags.
async fn one(State(st): State<AppState>, _: Key, Path(id): Path<String>) -> Answer {
    let db = &st.readarr;
    let id: i64 = id.parse().map_err(|_| Fail::not_found())?;
    let Some(f) = rows(db, "f.Id = ?", vec![json!(id)])?.into_iter().next() else { return Err(Fail::not_found()) };
    let mut v = resource(db, &f, true)?;
    v["audioTags"] = tags(&f.s("Path"));
    Ok(Json(v))
}
