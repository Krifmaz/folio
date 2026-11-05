//! What the outside apps see of the library: one [`Item`] per exact edition that has files, the files
//! themselves, and each person's [`Progress`] in it. The protocols (OPDS, KOReader, Audiobookshelf, Kobo) all
//! read the library through these shapes, so each one is written once against them and the library's own
//! tables are read in one place ([`super::shelf`]).

use crate::metadata::{EditionMeta, Format};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One file of an edition on disk (a row of `edition_files`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct File {
    /// the file's id in `edition_files`, stable while the file is kept
    pub id: i64,
    /// where the file is on disk (the library folder joined with the row's path)
    pub path: PathBuf,
    /// the file name shown to people
    pub name: String,
    /// `audio`, `epub`, `pdf` or `azw3`
    pub kind: String,
    /// its media type (`audio/mp4`, `application/epub+zip`)
    pub mime: String,
    /// the file's size in bytes
    pub size: u64,
    /// how long it plays, in seconds (audio files; 0 otherwise)
    pub duration: f64,
    /// the chapters of an audio file, counted from the start of this file
    pub chapters: Vec<FileChapter>,
    /// when the file came into the library, in seconds since 1970
    pub added_at: f64,
}

/// One chapter of an audio file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FileChapter {
    /// the chapter's title
    pub title: String,
    /// where it starts, in milliseconds from the start of its file
    pub start_ms: u64,
    /// where it ends, in milliseconds from the start of its file
    pub end_ms: u64,
}

impl File {
    /// The file's extension in lower case, without the dot (`epub`, `m4b`).
    pub fn ext(&self) -> String {
        std::path::Path::new(&self.name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
    }
}

/// One exact edition in the library, as the outside apps see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Item {
    /// the edition's id
    pub id: i64,
    /// the work the edition belongs to
    pub work_id: i64,
    /// everything the sources say about the edition
    pub meta: EditionMeta,
    /// the edition's files (no cover), audio files in playing order
    pub files: Vec<File>,
    /// the cover picture on disk, when there is one, and its media type
    pub cover: Option<(PathBuf, String)>,
    /// when the edition came into the library, in seconds since 1970
    pub added_at: f64,
    /// when it last changed, in seconds since 1970
    pub updated_at: f64,
}

impl Item {
    /// Whether the item is an audiobook.
    pub fn is_audio(&self) -> bool {
        self.meta.format == Format::Audiobook
    }

    /// The whole running time in seconds: the files' durations added, else the source's runtime.
    pub fn duration(&self) -> f64 {
        let files: f64 = self.files.iter().map(|f| f.duration).sum();
        if files > 0.0 {
            files
        } else {
            self.meta.runtime_min.map(|m| f64::from(m) * 60.0).unwrap_or(0.0)
        }
    }

    /// The files' sizes added.
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// The ebook's file: the first EPUB, else the first file.
    pub fn ebook_file(&self) -> Option<&File> {
        self.files.iter().find(|f| f.kind == "epub").or_else(|| self.files.iter().find(|f| f.kind != "audio"))
    }

    /// The title with its subtitle after a colon, when it has one.
    pub fn full_title(&self) -> String {
        if self.meta.subtitle.is_empty() {
            self.meta.title.clone()
        } else {
            format!("{}: {}", self.meta.title, self.meta.subtitle)
        }
    }

    /// The authors' names joined with commas.
    pub fn authors(&self) -> String {
        self.meta.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// How far one person is in one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Progress {
    /// the edition
    pub item_id: i64,
    /// how far, from 0 to 1
    pub fraction: f64,
    /// where in an audiobook, in seconds from the start
    pub position: f64,
    /// where in an ebook, in the reader's own words (an EPUB CFI, or KOReader's XPointer); empty when unknown
    pub location: String,
    /// whether the person has finished it
    pub finished: bool,
    /// when it last changed, in seconds since 1970
    pub updated_at: f64,
}

/// A place a person marked in an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Bookmark {
    /// the edition
    pub item_id: i64,
    /// where, in seconds from the start (audiobooks)
    pub position: f64,
    /// where in an ebook (an EPUB CFI); empty for audiobooks
    pub location: String,
    /// the person's note
    pub title: String,
    /// when it was made, in seconds since 1970
    pub created_at: f64,
}
