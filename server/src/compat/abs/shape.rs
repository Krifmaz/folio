//! Folio's books in Audiobookshelf's shapes: libraries, library items (whole and minified), audio files and
//! tracks, chapters, authors, series, a person's media progress and bookmarks, and the user. The field names and
//! units are Audiobookshelf's: times of day in milliseconds since 1970, lengths and places in seconds.

use super::super::model::{File, Item};
use super::super::opds::urlencode;
use crate::library::text;
use crate::playback::store::{Bookmark, Progress};
use serde_json::{json, Value};

/// The version of Audiobookshelf's API Folio answers as. Apps change what they ask for by it; 2.25 is the last
/// with the plain token sign-in Folio speaks.
pub const SERVER_VERSION: &str = "2.25.1";

/// The audiobooks library's id.
pub const AUDIOBOOKS: &str = "audiobooks";
/// The ebooks library's id.
pub const EBOOKS: &str = "ebooks";

/// The library an item is in.
pub fn library_of(it: &Item) -> &'static str {
    if it.is_audio() {
        AUDIOBOOKS
    } else {
        EBOOKS
    }
}

/// An item's id in Audiobookshelf's form (`li_12` for edition 12).
pub fn item_id(edition: i64) -> String {
    format!("li_{edition}")
}

/// The edition of an item id (`li_12` or `12`).
pub fn edition_of(id: &str) -> Option<i64> {
    id.strip_prefix("li_").unwrap_or(id).parse().ok()
}

/// A file's `ino` (`f7` for file 7).
pub fn ino(file: i64) -> String {
    format!("f{file}")
}

/// The file of an `ino` (`f7` or `7`).
pub fn file_of(ino: &str) -> Option<i64> {
    ino.strip_prefix('f').unwrap_or(ino).parse().ok()
}

/// An author's id: `au_` and the name's key.
pub fn author_id(name: &str) -> String {
    format!("au_{}", text::person_key(name))
}

/// A series' id: `se_` and the name's key.
pub fn series_id(name: &str) -> String {
    format!("se_{}", text::person_key(name))
}

/// Seconds since 1970 as milliseconds.
pub fn ms(secs: f64) -> i64 {
    (secs * 1000.0) as i64
}

/// One of Audiobookshelf's two libraries.
pub fn library(id: &str) -> Value {
    let (name, icon, order) = if id == AUDIOBOOKS { ("Audiobooks", "audiobookshelf", 1) } else { ("Ebooks", "books-1", 2) };
    json!({
        "id": id,
        "name": name,
        "folders": [{ "id": "folio", "fullPath": "/library", "libraryId": id, "addedAt": 0 }],
        "displayOrder": order,
        "icon": icon,
        "mediaType": "book",
        "provider": "audible",
        "settings": {
            "coverAspectRatio": 1,
            "disableWatcher": true,
            "skipMatchingMediaWithAsin": true,
            "skipMatchingMediaWithIsbn": true,
            "autoScanCronExpression": null,
            "audiobooksOnly": id == AUDIOBOOKS,
            "hideSingleBookSeries": false,
            "onlyShowLaterBooksInContinueSeries": false,
            "metadataPrecedence": [],
            "markAsFinishedPercentComplete": null,
            "markAsFinishedTimeRemaining": 10
        },
        "createdAt": 0,
        "lastUpdate": 0
    })
}

/// A file's `metadata`: name, extension, paths and size.
fn file_meta(it: &Item, f: &File, root: &std::path::Path) -> Value {
    let rel = f.path.strip_prefix(root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| f.name.clone());
    let at = ms(f.added_at);
    let _ = it;
    json!({
        "filename": f.name,
        "ext": format!(".{}", f.ext()),
        "path": f.path.to_string_lossy(),
        "relPath": rel,
        "size": f.size,
        "mtimeMs": at,
        "ctimeMs": at,
        "birthtimeMs": at
    })
}

/// The codec Audiobookshelf names for a file (`mp3`, `aac`).
fn codec(f: &File) -> &'static str {
    match f.ext().as_str() {
        "mp3" => "mp3",
        "m4b" | "m4a" | "mp4" | "aac" => "aac",
        "ogg" | "opus" => "opus",
        "flac" => "flac",
        _ => "",
    }
}

/// The audio files of an item, numbered from 1.
fn audio_files(it: &Item, root: &std::path::Path) -> Vec<Value> {
    it.files
        .iter()
        .filter(|f| f.kind == "audio")
        .enumerate()
        .map(|(i, f)| {
            json!({
                "index": i + 1,
                "ino": ino(f.id),
                "metadata": file_meta(it, f, root),
                "addedAt": ms(f.added_at),
                "updatedAt": ms(f.added_at),
                "trackNumFromMeta": null,
                "discNumFromMeta": null,
                "trackNumFromFilename": i + 1,
                "discNumFromFilename": null,
                "manuallyVerified": false,
                "invalid": false,
                "exclude": false,
                "error": null,
                "format": f.ext().to_uppercase(),
                "duration": f.duration,
                "bitRate": null,
                "language": null,
                "codec": codec(f),
                "timeBase": "1/44100",
                "channels": 2,
                "channelLayout": "stereo",
                "chapters": [],
                "embeddedCoverArt": null,
                "metaTags": {},
                "mimeType": f.mime
            })
        })
        .collect()
}

/// The audio tracks a player plays, each with where it starts in the book and its address.
pub fn tracks(it: &Item, root: &std::path::Path) -> Vec<Value> {
    let mut start = 0.0;
    it.files
        .iter()
        .filter(|f| f.kind == "audio")
        .enumerate()
        .map(|(i, f)| {
            let t = json!({
                "index": i + 1,
                "startOffset": start,
                "duration": f.duration,
                "title": f.name,
                "contentUrl": format!("/api/items/{}/file/{}", item_id(it.id), ino(f.id)),
                "mimeType": f.mime,
                "codec": codec(f),
                "metadata": file_meta(it, f, root)
            });
            start += f.duration;
            t
        })
        .collect()
}

/// The book's chapters: the files' own (each counted from the book's start), else Audnexus's, else one per file.
pub fn chapters(it: &Item) -> Vec<Value> {
    let mut out = Vec::new();
    let mut start = 0.0;
    for f in it.files.iter().filter(|f| f.kind == "audio") {
        for c in &f.chapters {
            out.push((start + c.start_ms as f64 / 1000.0, start + c.end_ms as f64 / 1000.0, c.title.clone()));
        }
        start += f.duration;
    }
    if out.is_empty() {
        out = it
            .meta
            .chapters
            .iter()
            .map(|c| (c.start_ms as f64 / 1000.0, (c.start_ms + c.length_ms) as f64 / 1000.0, c.title.clone()))
            .collect();
    }
    if out.is_empty() {
        let mut start = 0.0;
        for f in it.files.iter().filter(|f| f.kind == "audio") {
            out.push((start, start + f.duration, f.name.clone()));
            start += f.duration;
        }
    }
    out.into_iter().enumerate().map(|(i, (s, e, t))| json!({ "id": i, "start": s, "end": e, "title": t })).collect()
}

/// The book's metadata as Audiobookshelf writes it; `minified` adds the joined names lists show.
fn metadata(it: &Item, minified: bool) -> Value {
    let m = &it.meta;
    let title_ignore = {
        let t = &m.title;
        ["The ", "A ", "An "].iter().find_map(|a| t.strip_prefix(a).map(|r| format!("{r}, {}", a.trim()))).unwrap_or_else(|| t.clone())
    };
    let series: Vec<Value> = m.series.iter().map(|s| json!({ "id": series_id(&s.name), "name": s.name, "sequence": s.position })).collect();
    let year = m.year().map(|y| y.to_string());
    let mut v = json!({
        "title": m.title,
        "titleIgnorePrefix": title_ignore,
        "subtitle": if m.subtitle.is_empty() { Value::Null } else { json!(m.subtitle) },
        "narrators": m.narrators,
        "genres": m.genres,
        "publishedYear": year,
        "publishedDate": if m.release_date.len() == 10 { json!(m.release_date) } else { Value::Null },
        "publisher": if m.publisher.is_empty() { Value::Null } else { json!(m.publisher) },
        "description": if m.description.is_empty() { Value::Null } else { json!(m.description) },
        "isbn": if m.isbn13.is_empty() { Value::Null } else { json!(m.isbn13) },
        "asin": if m.asin.is_empty() { Value::Null } else { json!(m.asin) },
        "language": if m.language.is_empty() { Value::Null } else { json!(m.language) },
        "explicit": false,
        "abridged": m.abridged.unwrap_or(false)
    });
    let authors_lf = m.authors.iter().map(|a| text::sort_name(&a.name)).collect::<Vec<_>>().join(", ");
    let series_name = m
        .series
        .iter()
        .map(|s| if s.position.is_empty() { s.name.clone() } else { format!("{} #{}", s.name, s.position) })
        .collect::<Vec<_>>()
        .join(", ");
    if minified {
        v["authorName"] = json!(it.authors());
        v["authorNameLF"] = json!(authors_lf);
        v["narratorName"] = json!(m.narrators.join(", "));
        v["seriesName"] = json!(series_name);
    } else {
        v["authors"] = json!(m.authors.iter().map(|a| json!({ "id": author_id(&a.name), "name": a.name })).collect::<Vec<_>>());
        v["series"] = json!(series);
        v["authorName"] = json!(it.authors());
        v["authorNameLF"] = json!(authors_lf);
        v["narratorName"] = json!(m.narrators.join(", "));
        v["seriesName"] = json!(series_name);
    }
    v
}

/// The ebook file of an item, when it has one.
fn ebook_file(it: &Item, root: &std::path::Path) -> Value {
    match (it.is_audio(), it.ebook_file()) {
        (false, Some(f)) => json!({
            "ino": ino(f.id),
            "metadata": file_meta(it, f, root),
            "ebookFormat": f.ext(),
            "addedAt": ms(f.added_at),
            "updatedAt": ms(f.added_at)
        }),
        _ => Value::Null,
    }
}

/// The cover's path as Audiobookshelf gives it (apps only check it is there, then ask `/cover`).
fn cover_path(it: &Item) -> Value {
    it.cover.as_ref().map(|(p, _)| json!(p.to_string_lossy())).unwrap_or(Value::Null)
}

/// A library item; `minified` for lists (no files), else whole (`expanded`).
pub fn item(it: &Item, root: &std::path::Path, minified: bool) -> Value {
    let id = item_id(it.id);
    let folder = it.files.first().and_then(|f| f.path.parent()).map(|p| p.to_path_buf()).unwrap_or_default();
    let rel = folder.strip_prefix(root).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let audio: Vec<&File> = it.files.iter().filter(|f| f.kind == "audio").collect();
    let ebook_format = if it.is_audio() { Value::Null } else { it.ebook_file().map(|f| json!(f.ext())).unwrap_or(Value::Null) };
    let mut media = json!({
        "id": format!("bk_{}", it.id),
        "libraryItemId": id,
        "metadata": metadata(it, minified),
        "coverPath": cover_path(it),
        "tags": [],
        "duration": it.duration(),
        "size": it.size(),
        "numTracks": audio.len(),
        "numAudioFiles": audio.len(),
        "numChapters": chapters(it).len(),
        "numMissingParts": 0,
        "numInvalidAudioFiles": 0,
        "ebookFormat": ebook_format
    });
    if !minified {
        media["audioFiles"] = json!(audio_files(it, root));
        media["chapters"] = json!(chapters(it));
        media["tracks"] = json!(tracks(it, root));
        media["missingParts"] = json!([]);
        media["ebookFile"] = ebook_file(it, root);
    }
    let mut v = json!({
        "id": id,
        "ino": id,
        "oldLibraryItemId": null,
        "libraryId": library_of(it),
        "folderId": "folio",
        "path": folder.to_string_lossy(),
        "relPath": rel,
        "isFile": false,
        "mtimeMs": ms(it.updated_at),
        "ctimeMs": ms(it.updated_at),
        "birthtimeMs": ms(it.added_at),
        "addedAt": ms(it.added_at),
        "updatedAt": ms(it.updated_at),
        "lastScan": ms(it.updated_at),
        "scanVersion": SERVER_VERSION,
        "isMissing": false,
        "isInvalid": false,
        "mediaType": "book",
        "media": media,
        "numFiles": it.files.len() + usize::from(it.cover.is_some()),
        "size": it.size()
    });
    if !minified {
        let mut files: Vec<Value> = it
            .files
            .iter()
            .map(|f| {
                let kind = if f.kind == "audio" { "audio" } else { "ebook" };
                json!({ "ino": ino(f.id), "metadata": file_meta(it, f, root), "addedAt": ms(f.added_at), "updatedAt": ms(f.added_at), "fileType": kind })
            })
            .collect();
        if let Some((p, _)) = &it.cover {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            files.push(json!({ "ino": format!("cover_{}", it.id), "metadata": { "filename": name, "ext": ".jpg", "path": p.to_string_lossy(), "relPath": name, "size": 0 }, "fileType": "image" }));
        }
        v["libraryFiles"] = json!(files);
    }
    v
}

/// A person's progress in an item, in Audiobookshelf's shape.
pub fn progress(user_id: i64, it: &Item, p: &Progress) -> Value {
    let duration = it.duration();
    let updated = ms(p.updated_at);
    let cfi = p.locator.get("cfi").and_then(Value::as_str).unwrap_or("");
    json!({
        "id": format!("{}-{}", user_id, item_id(it.id)),
        "userId": user_id.to_string(),
        "libraryItemId": item_id(it.id),
        "episodeId": null,
        "mediaItemId": format!("bk_{}", it.id),
        "mediaItemType": "book",
        "duration": if it.is_audio() { duration } else { 0.0 },
        "progress": p.fraction,
        "currentTime": p.position_ms as f64 / 1000.0,
        "isFinished": p.finished,
        "hideFromContinueListening": false,
        "ebookLocation": if it.is_audio() || cfi.is_empty() { Value::Null } else { json!(cfi) },
        "ebookProgress": if it.is_audio() { Value::Null } else { json!(p.fraction) },
        "lastUpdate": updated,
        "startedAt": updated,
        "finishedAt": if p.finished { json!(updated) } else { Value::Null }
    })
}

/// A bookmark in Audiobookshelf's shape (audiobooks have only times).
pub fn bookmark(b: &Bookmark) -> Value {
    json!({
        "libraryItemId": item_id(b.edition_id),
        "title": b.note,
        "time": b.position_ms as f64 / 1000.0,
        "createdAt": ms(b.created_at)
    })
}

/// An author with how many books they have.
pub fn author(name: &str, books: usize, library: &str) -> Value {
    json!({
        "id": author_id(name),
        "asin": null,
        "name": name,
        "description": null,
        "imagePath": null,
        "libraryId": library,
        "addedAt": 0,
        "updatedAt": 0,
        "numBooks": books
    })
}

/// A series (without its books).
pub fn series(name: &str, library: &str) -> Value {
    let ignore = ["The ", "A ", "An "]
        .iter()
        .find_map(|a| name.strip_prefix(a).map(|r| format!("{r}, {}", a.trim())))
        .unwrap_or_else(|| name.to_string());
    json!({
        "id": series_id(name),
        "name": name,
        "nameIgnorePrefix": ignore,
        "description": null,
        "libraryId": library,
        "addedAt": 0,
        "updatedAt": 0
    })
}

/// The address of an item's cover.
pub fn cover_url(it: &Item) -> String {
    format!("/api/items/{}/cover", urlencode(&item_id(it.id)))
}
