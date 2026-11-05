//! A book with its authors and series, and the JSON the original sends for it (`server/models/Book.js`):
//! `toOldJSON`, `toOldJSONMinified` and `toOldJSONExpanded`, with the metadata objects inside them.

use super::{Book, Table};
use crate::abs::library::names::name_to_last_first;
use crate::abs::library::util::{array, b, clone_or_null, js_num, js_num_opt, s, title_prefix_at_end, Sorting};
use kit_server_core::{Db, Result, RowExt};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// An author as a book lists it: `{id, name}`.
#[derive(Debug, Clone, PartialEq)]
pub struct BookAuthorRef {
    /// The author's id.
    pub id: String,
    /// The author's name, "First Last".
    pub name: String,
}

/// A series as a book lists it, with the book's place in it.
#[derive(Debug, Clone, PartialEq)]
pub struct BookSeriesRef {
    /// The series' id.
    pub id: String,
    /// The series' name.
    pub name: String,
    /// The series' name with its prefix moved to the end, as stored.
    pub name_ignore_prefix: Option<String>,
    /// The book's number in the series, such as "1" or "2.5".
    pub sequence: Option<String>,
    /// The id of the `bookSeries` row that links them.
    pub book_series_id: String,
}

/// A book row with its authors and series loaded, in the order they were added.
#[derive(Debug, Clone, Default)]
pub struct BookFull {
    /// The `books` row.
    pub book: Book,
    /// Its authors, oldest link first.
    pub authors: Vec<BookAuthorRef>,
    /// Its series, oldest link first.
    pub series: Vec<BookSeriesRef>,
}

/// Loads books by id with their authors and series. Missing ids are left out.
pub fn load(db: &dyn Db, ids: &[String]) -> Result<HashMap<String, BookFull>> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    for chunk in ids.chunks(500) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let params: Vec<Value> = chunk.iter().map(|i| Value::from(i.as_str())).collect();
        let rows = db.query(&format!("SELECT * FROM `{}` WHERE `id` IN ({marks})", Book::TABLE), params.clone())?;
        for row in rows {
            let book: Book = super::from_row(row)?;
            let id = book.id.clone().unwrap_or_default();
            out.insert(id, BookFull { book, ..BookFull::default() });
        }
        let authors = db.query(
            &format!(
                "SELECT ba.bookId, a.id, a.name FROM bookAuthors ba JOIN authors a ON a.id = ba.authorId \
                 WHERE ba.bookId IN ({marks}) ORDER BY ba.createdAt ASC"
            ),
            params.clone(),
        )?;
        for r in authors {
            if let Some(full) = out.get_mut(&r.s("bookId")) {
                full.authors.push(BookAuthorRef { id: r.s("id"), name: r.s("name") });
            }
        }
        let series = db.query(
            &format!(
                "SELECT bs.id AS bsId, bs.bookId, bs.sequence, s.id, s.name, s.nameIgnorePrefix FROM bookSeries bs \
                 JOIN series s ON s.id = bs.seriesId WHERE bs.bookId IN ({marks}) ORDER BY bs.createdAt ASC"
            ),
            params,
        )?;
        for r in series {
            if let Some(full) = out.get_mut(&r.s("bookId")) {
                full.series.push(BookSeriesRef {
                    id: r.s("id"),
                    name: r.s("name"),
                    name_ignore_prefix: r.get("nameIgnorePrefix").and_then(Value::as_str).map(String::from),
                    sequence: r.get("sequence").and_then(Value::as_str).map(String::from),
                    book_series_id: r.s("bsId"),
                });
            }
        }
    }
    Ok(out)
}

impl BookFull {
    /// The authors' names joined with ", " (`authorName`).
    pub fn author_name(&self) -> String {
        self.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// The authors' names as "Last, First", joined with ", " (`authorNameLF`).
    pub fn author_name_lf(&self) -> String {
        self.authors.iter().map(|a| name_to_last_first(&a.name)).collect::<Vec<_>>().join(", ")
    }

    /// The series with the book's number, such as "Imperial Radch #1", joined with ", " (`seriesName`).
    pub fn series_name(&self) -> String {
        self.series
            .iter()
            .map(|se| match se.sequence.as_deref().filter(|q| !q.is_empty()) {
                Some(q) => format!("{} #{q}", se.name),
                None => se.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The narrators' names joined with ", ".
    pub fn narrator_name(&self) -> String {
        strings(&self.book.narrators).join(", ")
    }

    /// The audio files, as stored.
    pub fn audio_files(&self) -> Vec<Value> {
        match &self.book.audio_files {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        }
    }

    /// The audio files that are played (not marked `exclude`), in order (`includedAudioFiles`).
    pub fn included_audio_files(&self) -> Vec<Value> {
        self.audio_files().into_iter().filter(|af| !af.get("exclude").is_some_and(truthy)).collect()
    }

    /// The bytes of every audio file plus the ebook file (`size`); `null` when a size is missing, like the
    /// original's `NaN`.
    pub fn size(&self) -> Value {
        let mut total = Some(0.0);
        for af in self.audio_files() {
            total = total.zip(af.pointer("/metadata/size").and_then(Value::as_f64)).map(|(t, n)| t + n);
        }
        if let Some(ebook) = self.book.ebook_file.as_ref().filter(|v| truthy(v)) {
            total = total.zip(ebook.pointer("/metadata/size").and_then(Value::as_f64)).map(|(t, n)| t + n);
        }
        js_num_opt(total)
    }

    /// The tracks a player plays: each included audio file with `title`, `startOffset` and `contentUrl`
    /// (`getTracklist`).
    pub fn tracks(&self, library_item_id: &str) -> Value {
        let mut start = 0.0;
        let tracks = self
            .included_audio_files()
            .into_iter()
            .map(|mut af| {
                let title = af.pointer("/metadata/filename").cloned().unwrap_or(Value::Null);
                let ino = af.get("ino").and_then(Value::as_str).unwrap_or_default().to_string();
                let duration = af.get("duration").and_then(Value::as_f64).unwrap_or(f64::NAN);
                if let Value::Object(m) = &mut af {
                    m.insert("title".into(), title);
                    m.insert("startOffset".into(), js_num(start));
                    m.insert("contentUrl".into(), Value::from(format!("/api/items/{library_item_id}/file/{ino}")));
                }
                start += duration;
                af
            })
            .collect();
        Value::Array(tracks)
    }

    /// The metadata of `toOldJSON`: authors and series as `{id, name}` and `{id, name, sequence}`.
    pub fn metadata_json(&self) -> Map<String, Value> {
        let bk = &self.book;
        let authors: Vec<Value> = self.authors.iter().map(|a| json!({"id": a.id, "name": a.name})).collect();
        let series: Vec<Value> = self.series.iter().map(|se| json!({"id": se.id, "name": se.name, "sequence": se.sequence})).collect();
        let mut m = Map::new();
        m.insert("title".into(), s(&bk.title));
        m.insert("subtitle".into(), s(&bk.subtitle));
        m.insert("authors".into(), Value::Array(authors));
        m.insert("narrators".into(), array(&bk.narrators));
        m.insert("series".into(), Value::Array(series));
        m.insert("genres".into(), array(&bk.genres));
        m.insert("publishedYear".into(), s(&bk.published_year));
        m.insert("publishedDate".into(), s(&bk.published_date));
        m.insert("publisher".into(), s(&bk.publisher));
        m.insert("description".into(), s(&bk.description));
        m.insert("isbn".into(), s(&bk.isbn));
        m.insert("asin".into(), s(&bk.asin));
        m.insert("language".into(), s(&bk.language));
        m.insert("explicit".into(), b(bk.explicit));
        m.insert("abridged".into(), b(bk.abridged));
        m
    }

    /// The metadata of the minified JSON: names as strings instead of lists.
    pub fn metadata_minified(&self, sorting: &Sorting) -> Map<String, Value> {
        let bk = &self.book;
        let mut m = Map::new();
        m.insert("title".into(), s(&bk.title));
        m.insert("titleIgnorePrefix".into(), title_ignore(&bk.title, sorting));
        m.insert("subtitle".into(), s(&bk.subtitle));
        m.insert("authorName".into(), Value::from(self.author_name()));
        m.insert("authorNameLF".into(), Value::from(self.author_name_lf()));
        m.insert("narratorName".into(), Value::from(self.narrator_name()));
        m.insert("seriesName".into(), Value::from(self.series_name()));
        m.insert("genres".into(), array(&bk.genres));
        m.insert("publishedYear".into(), s(&bk.published_year));
        m.insert("publishedDate".into(), s(&bk.published_date));
        m.insert("publisher".into(), s(&bk.publisher));
        m.insert("description".into(), s(&bk.description));
        m.insert("isbn".into(), s(&bk.isbn));
        m.insert("asin".into(), s(&bk.asin));
        m.insert("language".into(), s(&bk.language));
        m.insert("explicit".into(), b(bk.explicit));
        m.insert("abridged".into(), b(bk.abridged));
        m
    }

    /// The metadata of the expanded JSON: both forms, plus the description without HTML.
    pub fn metadata_expanded(&self, sorting: &Sorting) -> Map<String, Value> {
        let bk = &self.book;
        let mut m = self.metadata_json();
        m.insert("titleIgnorePrefix".into(), title_ignore(&bk.title, sorting));
        m.insert("authorName".into(), Value::from(self.author_name()));
        m.insert("authorNameLF".into(), Value::from(self.author_name_lf()));
        m.insert("narratorName".into(), Value::from(self.narrator_name()));
        m.insert("seriesName".into(), Value::from(self.series_name()));
        let plain = bk.description.as_deref().filter(|d| !d.is_empty()).map(|d| crate::abs::html::strip_all_tags(Some(d)));
        m.insert("descriptionPlain".into(), plain.map_or(Value::Null, Value::from));
        m
    }

    /// `toOldJSON(libraryItemId)`.
    pub fn old_json(&self, library_item_id: &str) -> Value {
        let bk = &self.book;
        json!({
            "id": bk.id,
            "libraryItemId": library_item_id,
            "metadata": self.metadata_json(),
            "coverPath": bk.cover_path,
            "tags": array(&bk.tags),
            "audioFiles": clone_or_null(&bk.audio_files),
            "chapters": clone_or_null(&bk.chapters),
            "ebookFile": clone_or_null(&bk.ebook_file),
        })
    }

    /// `toOldJSONMinified()`.
    pub fn minified(&self, sorting: &Sorting) -> Map<String, Value> {
        let bk = &self.book;
        let mut m = Map::new();
        m.insert("id".into(), s(&bk.id));
        m.insert("metadata".into(), Value::Object(self.metadata_minified(sorting)));
        m.insert("coverPath".into(), s(&bk.cover_path));
        m.insert("tags".into(), array(&bk.tags));
        m.insert("numTracks".into(), Value::from(self.included_audio_files().len()));
        m.insert("numAudioFiles".into(), Value::from(self.audio_files().len()));
        let chapters = match &bk.chapters {
            Some(Value::Array(a)) => a.len(),
            _ => 0,
        };
        m.insert("numChapters".into(), Value::from(chapters));
        m.insert("duration".into(), js_num_opt(bk.duration));
        m.insert("size".into(), self.size());
        if let Some(format) = bk.ebook_file.as_ref().and_then(|e| e.get("ebookFormat")).filter(|v| !v.is_null()) {
            m.insert("ebookFormat".into(), format.clone());
        } else if let Some(Value::Null) = bk.ebook_file.as_ref().and_then(|e| e.get("ebookFormat")) {
            m.insert("ebookFormat".into(), Value::Null);
        }
        m
    }

    /// `toOldJSONExpanded(libraryItemId)`.
    pub fn expanded(&self, library_item_id: &str, sorting: &Sorting) -> Map<String, Value> {
        let bk = &self.book;
        let mut m = self.minified(sorting);
        m.insert("libraryItemId".into(), Value::from(library_item_id));
        m.insert("metadata".into(), Value::Object(self.metadata_expanded(sorting)));
        m.insert("audioFiles".into(), clone_or_null(&bk.audio_files));
        m.insert("chapters".into(), clone_or_null(&bk.chapters));
        m.insert("ebookFile".into(), clone_or_null(&bk.ebook_file));
        m.insert("tracks".into(), self.tracks(library_item_id));
        m
    }
}

/// A title with its sorting prefix moved to the end; an empty string when there is no title.
fn title_ignore(title: &Option<String>, sorting: &Sorting) -> Value {
    match title.as_deref() {
        Some(t) if !t.is_empty() => Value::from(title_prefix_at_end(t, sorting)),
        Some(t) => Value::from(t),
        None => Value::Null,
    }
}

/// JavaScript's truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// The strings of a stored JSON array.
pub fn strings(v: &Option<Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect(),
        _ => Vec::new(),
    }
}
