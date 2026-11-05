//! Adding and rescanning books, like the original's `BookScanner`: probing the audio files, choosing the ebook,
//! gathering the book's details in the library's metadata order, linking authors and series, finding a cover and
//! writing the item's `metadata.json`.

use super::audio;
use super::files::{self, ext_in, EBOOK_TYPES};
use super::item_data::ItemScanData;
use super::meta::{self, BookMeta, EbookScan};
use super::probe;
use super::{Scan, ScanLog};
use crate::abs::library::names::name_to_last_first;
use crate::abs::library::util::title_ignore_prefix;
use crate::abs::library::{self as lib};
use crate::abs::models::{self, book as bookm, Author, Book, BookAuthor, BookSeries, LibraryItem, Series};
use crate::abs::ABS_VERSION;
use kit_server_core::Result;
use serde_json::{json, Value};

/// The library's metadata order when it has none (`Library.defaultMetadataPrecedence`).
pub const DEFAULT_PRECEDENCE: &[&str] = &["folderStructure", "audioMetatags", "nfoFile", "txtFiles", "opfFile", "absMetadata"];

/// The metadata order of a library's settings.
pub fn precedence(settings: &Value) -> Vec<String> {
    settings["metadataPrecedence"]
        .as_array()
        .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_else(|| DEFAULT_PRECEDENCE.iter().map(|s| s.to_string()).collect())
}

/// Probes audio files (`executeMediaFileScans`); files ffprobe can't read are left out.
pub fn probe_files(media_type: &str, data: &ItemScanData, audio_files: &[Value], log: &mut ScanLog) -> Vec<Value> {
    let now = models::now_ms();
    let mut out = Vec::new();
    for lf in audio_files {
        let path = files::file_path(lf);
        match probe::probe(&path) {
            Ok(p) => {
                let (track, disc) = if media_type == "book" {
                    let name = lf["metadata"]["filename"].as_str().unwrap_or("");
                    let (t, d) = audio::numbers_from_filename(&data.media_metadata, name, &path);
                    (t, d)
                } else {
                    (None, None)
                };
                out.push(probe::audio_file(lf, &p, track, disc, now));
            }
            Err(e) => log.error(format!("{e} : \"{path}\"")),
        }
    }
    out
}

/// The ebook of a set of ebook files: the first EPUB, else the first one.
fn choose_ebook(ebooks: &[Value]) -> Option<Value> {
    ebooks.iter().find(|lf| files::format_of(&files::file_ext(lf)) == "epub").or_else(|| ebooks.first()).cloned()
}

/// A library file made into the book's ebook file (with `ebookFormat`).
fn as_ebook_file(lf: &Value) -> Value {
    let mut e = lf.clone();
    e["ebookFormat"] = json!(files::format_of(&files::file_ext(lf)));
    e
}

/// Reads an ebook file's details.
fn scan_ebook(ebook: Option<&Value>) -> Option<EbookScan> {
    let e = ebook?;
    meta::parse_ebook(&files::file_path(e), e["ebookFormat"].as_str().unwrap_or(""))
}

/// Gathers a book's details from every source in the library's order (`getBookMetadataFromScanData`), then the
/// cover from the folder's pictures, the title without its prefix and a cleaned description.
pub fn book_meta(
    scan: &Scan,
    audio_files: &[Value],
    ebook: Option<&EbookScan>,
    data: &ItemScanData,
    lib_settings: &Value,
    existing_id: Option<&str>,
    log: &mut ScanLog,
) -> BookMeta {
    let mut book = BookMeta::default();
    book.set("title", Value::String(data.media_metadata.title.clone()));
    let order = precedence(lib_settings);
    log.debug(format!("\"{}\" Getting metadata with precedence [{}]", book.title(), order.join(", ")));
    for source in &order {
        match source.as_str() {
            "folderStructure" => data.apply_folder_names(&mut book),
            "audioMetatags" => {
                if !audio_files.is_empty() {
                    let title = book.text("title").filter(|t| !t.is_empty()).unwrap_or_else(|| data.media_metadata.title.clone());
                    audio::book_meta_from_tags(&title, audio_files, &mut book);
                } else if let Some(opf) = ebook.and_then(|e| e.metadata.as_ref()) {
                    meta::apply_opf(opf, &mut book);
                }
            }
            "nfoFile" => {
                if let Some(nfo) = data.nfo_file() {
                    let text = files::read_text(&files::file_path(&nfo));
                    if !text.is_empty() {
                        meta::apply_nfo(&meta::parse_nfo(&text), &mut book);
                    }
                }
            }
            "txtFiles" => {
                if let Some(desc) = data.desc_txt() {
                    let text = files::read_text(&files::file_path(&desc));
                    if !text.trim().is_empty() {
                        book.set("description", json!(text.trim()));
                    }
                }
                if let Some(reader) = data.reader_txt() {
                    let text = files::read_text(&files::file_path(&reader));
                    let first = text.split('\n').next().map(|l| l.strip_suffix('\r').unwrap_or(l).trim().to_string()).unwrap_or_default();
                    if !first.is_empty() {
                        book.narrators = lib::names::parse_names(&first);
                    }
                }
            }
            "opfFile" => {
                if let Some(opf) = data.opf_file() {
                    let text = files::read_text(&files::file_path(&opf));
                    if let Some(parsed) = super::xml::to_json(&text).and_then(|j| meta::opf_from_json(&j)) {
                        meta::apply_opf(&parsed, &mut book);
                    }
                }
            }
            "absMetadata" => {
                let mut text = data.metadata_json().map(|f| files::read_text(&files::file_path(&f))).filter(|t| !t.is_empty());
                let mut file = data.metadata_json().map(|f| files::file_path(&f));
                if text.is_none() {
                    if let Some(id) = existing_id {
                        let p = scan.metadata_path.join("items").join(id).join("metadata.json").display().to_string();
                        if files::exists(&p) {
                            text = Some(files::read_text(&p)).filter(|t| !t.is_empty());
                            file = Some(p);
                        }
                    }
                }
                if let Some(text) = text {
                    log.info(format!("Found metadata file \"{}\"", file.unwrap_or_default()));
                    let parsed = meta::parse_abs_json(&text).unwrap_or_default();
                    meta::apply_abs_json(&parsed, &mut book);
                }
            }
            other => log.error(format!("Invalid metadata source \"{other}\"")),
        }
    }
    let images = data.image_files();
    if !images.is_empty() {
        let cover = images.iter().find(|i| is_cover_name(&files::file_path(i))).unwrap_or(&images[0]);
        book.set("coverPath", json!(files::file_path(cover)));
    }
    book.set("titleIgnorePrefix", json!(title_ignore_prefix(&book.title(), &scan.sorting)));
    if let Some(d) = book.text("description").filter(|d| !d.is_empty()) {
        book.set("description", json!(crate::abs::html::sanitize(Some(&d))));
    }
    book
}

/// Whether a picture's path ends in "/cover.<ext>" (`/\/cover\.[^.\/]*$/`).
fn is_cover_name(path: &str) -> bool {
    path.rsplit_once("/cover.").is_some_and(|(_, ext)| !ext.contains('.') && !ext.contains('/'))
}

/// Adds a library item for a new book (`scanNewBookLibraryItem`); `None` when the folder has neither audio nor an
/// ebook.
pub fn new_book_item(scan: &Scan, data: &ItemScanData, lib_settings: &Value, log: &mut ScanLog) -> Result<Option<LibraryItem>> {
    let db = scan.db;
    let mut audio_files = probe_files(&data.media_type, data, &data.audio_files(), log);
    audio::smart_track_order(&mut audio_files);
    let audiobooks_only = crate::abs::settings::truthy(lib_settings.get("audiobooksOnly"));
    let ebook = if audiobooks_only { None } else { choose_ebook(&data.ebook_files()) };
    if ebook.is_none() && audio_files.is_empty() {
        log.warn(format!("Library item at path \"{}\" has no audio files and no ebook file - ignoring", data.rel_path));
        return Ok(None);
    }
    let ebook = ebook.map(|e| as_ebook_file(&e));
    let ebook_scan = scan_ebook(ebook.as_ref());
    let m = book_meta(scan, &audio_files, ebook_scan.as_ref(), data, lib_settings, None, log);
    let duration: f64 = audio_files.iter().filter_map(|a| a["duration"].as_f64()).sum();
    let now = models::now_ms();
    let item_id = models::new_id();
    let book_id = models::new_id();
    let title = m.title();
    let mut cover = m.text("coverPath");
    let item_dir = if data.is_file { None } else { Some(data.path.clone()) };
    if cover.is_none() {
        cover = super::covers::save_embedded(scan, &audio_files, &item_id, item_dir.as_deref());
        if cover.is_some() {
            log.debug(format!("Extracted embedded cover from audio file for book \"{title}\""));
        } else if let Some(es) = ebook_scan.as_ref().filter(|e| e.cover_path.is_some()) {
            cover = super::covers::save_ebook(scan, es, &item_id, item_dir.as_deref());
        }
    }
    let book_row = Book {
        id: Some(book_id.clone()),
        title: Some(title.clone()),
        title_ignore_prefix: m.text("titleIgnorePrefix"),
        subtitle: m.text("subtitle"),
        published_year: m.text("publishedYear"),
        published_date: m.text("publishedDate"),
        publisher: m.text("publisher"),
        description: m.text("description"),
        isbn: m.text("isbn"),
        asin: m.text("asin"),
        language: m.text("language"),
        explicit: Some(m.fields.get("explicit").is_some_and(bookm::truthy)),
        abridged: Some(m.fields.get("abridged").is_some_and(bookm::truthy)),
        cover_path: cover,
        duration: Some(duration),
        narrators: Some(json!(m.narrators)),
        audio_files: Some(Value::Array(audio_files.clone())),
        ebook_file: ebook.clone(),
        chapters: Some(Value::Array(m.chapters.clone())),
        tags: Some(json!(m.tags)),
        genres: Some(json!(m.genres)),
        created_at: Some(models::date_text(now)),
        updated_at: Some(models::date_text(now)),
    };
    models::insert(db, &book_row)?;
    // Authors: existing ones by name, new ones made; new links get set times so their order holds.
    let mut new_authors: Vec<(String, String)> = Vec::new();
    for (i, name) in m.authors.iter().enumerate() {
        let (author_id, created) = match lib::author_id_by_name(db, &data.library_id, name)? {
            Some(id) => {
                log.authors_num_books_changed.insert(id.clone());
                (id, now)
            }
            None => {
                let id = models::new_id();
                let t = models::date_text(models::now_ms());
                models::insert(
                    db,
                    &Author {
                        id: Some(id.clone()),
                        name: Some(name.clone()),
                        last_first: Some(name_to_last_first(name)),
                        library_id: Some(data.library_id.clone()),
                        created_at: Some(t.clone()),
                        updated_at: Some(t),
                        ..Default::default()
                    },
                )?;
                new_authors.push((id.clone(), name.clone()));
                (id, now + i as i64)
            }
        };
        models::insert(
            db,
            &BookAuthor {
                id: Some(models::new_id()),
                created_at: Some(models::date_text(created)),
                book_id: Some(book_id.clone()),
                author_id: Some(author_id),
            },
        )?;
    }
    let mut new_series: Vec<(String, String)> = Vec::new();
    for s in m.series.iter().filter(|s| !s.name.is_empty()) {
        let series_id = match lib::series_id_by_name(db, &data.library_id, &s.name)? {
            Some(id) => id,
            None => {
                let id = models::new_id();
                let t = models::date_now();
                models::insert(
                    db,
                    &Series {
                        id: Some(id.clone()),
                        name: Some(s.name.clone()),
                        name_ignore_prefix: Some(title_ignore_prefix(&s.name, &scan.sorting)),
                        library_id: Some(data.library_id.clone()),
                        created_at: Some(t.clone()),
                        updated_at: Some(t),
                        ..Default::default()
                    },
                )?;
                new_series.push((id.clone(), s.name.clone()));
                id
            }
        };
        models::insert(
            db,
            &BookSeries {
                id: Some(models::new_id()),
                sequence: s.sequence.clone(),
                created_at: Some(models::date_now()),
                book_id: Some(book_id.clone()),
                series_id: Some(series_id),
            },
        )?;
    }
    // The ebook chosen is the main one; other ebooks are supplementary.
    let mut library_files = data.library_files.clone();
    for lf in library_files.iter_mut() {
        if ext_in(&files::file_ext(lf), EBOOK_TYPES) {
            lf["isSupplementary"] = json!(files::file_ino(lf) != ebook.as_ref().and_then(files::file_ino));
        }
    }
    let item = LibraryItem {
        id: Some(item_id.clone()),
        ino: Some(data.ino.clone()),
        path: Some(data.path.clone()),
        rel_path: Some(data.rel_path.clone()),
        media_id: Some(book_id.clone()),
        media_type: Some(data.media_type.clone()),
        is_file: Some(data.is_file),
        is_missing: Some(false),
        is_invalid: Some(false),
        mtime: Some(models::date_text(data.mtime_ms)),
        ctime: Some(models::date_text(data.ctime_ms)),
        birthtime: Some(models::date_text(data.birthtime_ms)),
        size: Some(data.size()),
        last_scan: Some(models::date_text(now)),
        last_scan_version: Some(ABS_VERSION.into()),
        library_files: Some(Value::Array(library_files)),
        extra_data: Some(json!({})),
        title: Some(title.clone()),
        title_ignore_prefix: Some(title_ignore_prefix(&title, &scan.sorting)),
        author_names_first_last: Some(m.authors.join(", ")),
        author_names_last_first: Some(m.authors.iter().map(|a| name_to_last_first(a)).collect::<Vec<_>>().join(", ")),
        created_at: Some(models::date_text(now)),
        updated_at: Some(models::date_text(now)),
        library_id: Some(data.library_id.clone()),
        library_folder_id: Some(data.library_folder_id.clone()),
    };
    models::insert(db, &item)?;
    // The library's filter menus learn the new names (`Database.add*ToFilterData`).
    lib::with_filter_data(&data.library_id, |fd| {
        for (id, name) in &new_series {
            lib::push_named(fd, "series", id, name);
        }
        for (id, name) in &new_authors {
            lib::push_named(fd, "authors", id, name);
        }
        lib::push_values(fd, "narrators", &m.narrators);
        lib::push_values(fd, "genres", &m.genres);
        lib::push_values(fd, "tags", &m.tags);
        lib::push_values(fd, "publishers", &m.text("publisher").into_iter().collect::<Vec<_>>());
        lib::push_values(fd, "languages", &m.text("language").into_iter().collect::<Vec<_>>());
        if let Some(year) = m.text("publishedYear").and_then(|y| lib::js_number(&y)) {
            lib::push_values(fd, "publishedDecades", &[((year / 10.0).floor() * 10.0).to_string()]);
        }
    });
    for (id, _) in &new_authors {
        if let Some(a) = models::find::<Author>(db, id)? {
            scan.emit("author_added", &super::author_json(&a));
        }
    }
    save_metadata_file(scan, &item_id, log)?;
    Ok(Some(item))
}

/// Writes an item's `metadata.json` in the metadata folder (`saveMetadataFile`). Storing it beside the item
/// (`storeMetadataWithItem`) comes with the item edits.
pub fn save_metadata_file(scan: &Scan, item_id: &str, log: &mut ScanLog) -> Result<()> {
    let Some((_, b)) = crate::abs::items::expanded_by_id(scan.db, item_id)? else { return Ok(()) };
    let dir = scan.metadata_path.join("items").join(item_id);
    let format = scan.settings.str("metadataFileFormat").to_string();
    let path = dir.join(format!("metadata.{}", if format.is_empty() { "json" } else { &format }));
    let bk = &b.book;
    let json = json!({
        "tags": bk.tags.clone().unwrap_or(json!([])),
        "chapters": bk.chapters.clone().unwrap_or(json!([])),
        "title": bk.title,
        "subtitle": bk.subtitle,
        "authors": b.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "narrators": bk.narrators.clone().unwrap_or(json!([])),
        "series": b.series.iter().map(|s| match s.sequence.as_deref().filter(|q| !q.is_empty()) { Some(q) => format!("{} #{q}", s.name), None => s.name.clone() }).collect::<Vec<_>>(),
        "genres": bk.genres.clone().unwrap_or(json!([])),
        "publishedYear": bk.published_year,
        "publishedDate": bk.published_date,
        "publisher": bk.publisher,
        "description": bk.description,
        "isbn": bk.isbn,
        "asin": bk.asin,
        "language": bk.language,
        "explicit": bk.explicit.unwrap_or(false),
        "abridged": bk.abridged.unwrap_or(false),
    });
    let written = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::write(&path, serde_json::to_string_pretty(&crate::abs::library::util::js_numbers(json)).unwrap_or_default())
    });
    match written {
        Ok(()) => log.debug(format!("Success saving abmetadata to \"{}\"", path.display())),
        Err(e) => log.error(format!("Failed to save json file at \"{}\": {e}", path.display())),
    }
    Ok(())
}

/// Whether two lists of chapters are the same (`areEquivalent`).
fn same_chapters(a: &Value, b: &[Value]) -> bool {
    crate::abs::library::util::js_numbers(a.clone()) == crate::abs::library::util::js_numbers(Value::Array(b.to_vec()))
}

/// Rescans an existing book (`rescanExistingBookLibraryItem`): audio files added, removed and changed, the cover,
/// the ebook, every detail the sources give, the authors and series. Returns whether anything changed.
pub fn rescan_book_item(scan: &Scan, item: &mut LibraryItem, data: &ItemScanData, lib_settings: &Value, log: &mut ScanLog) -> Result<bool> {
    let db = scan.db;
    let item_id = item.id.clone().unwrap_or_default();
    let Some((_, mut full)) = crate::abs::items::expanded_by_id(db, &item_id)? else { return Ok(false) };
    let audiobooks_only = crate::abs::settings::truthy(lib_settings.get("audiobooksOnly"));
    let mut audio_files: Vec<Value> = full.book.audio_files.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
    let mut has_media_changes = data.has_audio_changes() || data.audio_files().len() != audio_files.len();
    let media_type = item.media_type.clone().unwrap_or_else(|| "book".into());
    if has_media_changes {
        audio_files.retain(|af| !data.audio_file_removed(af));
        let modified = data.audio_modified();
        if !modified.is_empty() {
            let news: Vec<Value> = modified.iter().map(|(_, n)| n.clone()).collect();
            let mut scanned = probe_files(&media_type, data, &news, log);
            for af in audio_files.iter_mut() {
                let pos = scanned
                    .iter()
                    .position(|s| files::file_path(s) == files::file_path(af))
                    .or_else(|| scanned.iter().position(|s| files::file_ino(s) == files::file_ino(af)));
                if let Some(p) = pos {
                    let s = scanned.remove(p);
                    update_from_scan(af, &s);
                }
            }
            audio_files.extend(scanned);
        }
        let added = data.audio_added();
        if !added.is_empty() {
            audio_files.extend(probe_files(&media_type, data, &added, log));
        }
        let missing: Vec<Value> =
            data.audio_files().into_iter().filter(|lf| !audio_files.iter().any(|af| files::file_ino(af) == files::file_ino(lf))).collect();
        if !missing.is_empty() {
            audio_files.extend(probe_files(&media_type, data, &missing, log));
        }
        audio::smart_track_order(&mut audio_files);
        full.book.duration = Some(audio_files.iter().filter_map(|a| a["duration"].as_f64()).sum());
    }
    let mut cover = full.book.cover_path.clone();
    if let Some(c) = cover.clone() {
        if data.image_removed().iter().any(|lf| files::file_path(lf) == c) && !files::exists(&c) {
            cover = None;
            has_media_changes = true;
        }
    }
    if let Some(c) = cover.clone() {
        if let Some((_, new)) = data.image_modified().iter().find(|(old, _)| files::file_path(old) == c) {
            let p = files::file_path(new);
            if p != c {
                log.debug(format!("Updating book cover \"{c}\" => \"{p}\""));
                cover = Some(p);
                has_media_changes = true;
            }
        }
    }
    let images = data.image_files();
    if cover.is_none() && !images.is_empty() {
        let c = images.iter().find(|i| is_cover_name(&files::file_path(i))).unwrap_or(&images[0]);
        cover = Some(files::file_path(c));
        has_media_changes = true;
    }
    let mut ebook = full.book.ebook_file.clone().filter(|e| !e.is_null());
    if let Some(e) = ebook.clone() {
        if audiobooks_only || data.ebook_file_removed(&e) {
            ebook = None;
            has_media_changes = true;
        }
    }
    if let Some(e) = ebook.clone() {
        if let Some((_, new)) = data.ebook_modified().iter().find(|(old, _)| files::file_path(old) == files::file_path(&e)) {
            ebook = Some(as_ebook_file(new));
            has_media_changes = true;
        }
    }
    if ebook.is_none() && !audiobooks_only {
        if let Some(e) = choose_ebook(&data.ebook_files()) {
            ebook = Some(as_ebook_file(&e));
            has_media_changes = true;
        }
    }
    let ebook_scan = scan_ebook(ebook.as_ref());
    let m = book_meta(scan, &audio_files, ebook_scan.as_ref(), data, lib_settings, Some(&item_id), log);
    let mut authors_updated = false;
    let mut series_updated = false;
    let book_id = full.book.id.clone().unwrap_or_default();
    let mut bk = full.book.clone();
    // Details, in the original's key order; unset details and empty lists are left as they are.
    for (key, value) in &m.fields {
        if key == "coverPath" {
            continue;
        }
        let current = book_field(&bk, key);
        let new = value.clone();
        if current != new {
            log.debug(format!("Updating book {key} \"{current}\" => \"{new}\""));
            set_book_field(&mut bk, key, &new);
            has_media_changes = true;
        }
    }
    for (key, list) in [("narrators", &m.narrators), ("genres", &m.genres), ("tags", &m.tags)] {
        if list.is_empty() {
            continue;
        }
        let existing: Vec<String> = bookm::strings(match key {
            "narrators" => &bk.narrators,
            "genres" => &bk.genres,
            _ => &bk.tags,
        });
        if list.iter().any(|x| !existing.contains(x)) || existing.iter().any(|x| !list.contains(x)) {
            let v = Some(json!(list));
            match key {
                "narrators" => bk.narrators = v,
                "genres" => bk.genres = v,
                _ => bk.tags = v,
            }
            has_media_changes = true;
        }
    }
    if !m.chapters.is_empty() && !same_chapters(bk.chapters.as_ref().unwrap_or(&json!([])), &m.chapters) {
        bk.chapters = Some(Value::Array(m.chapters.clone()));
        has_media_changes = true;
    }
    if !m.authors.is_empty() {
        for name in &m.authors {
            if full.authors.iter().any(|a| &a.name == name) {
                continue;
            }
            let author_id = match lib::author_id_by_name(db, item.library_id.as_deref().unwrap_or_default(), name)? {
                Some(id) => {
                    log.authors_num_books_changed.insert(id.clone());
                    id
                }
                None => {
                    let id = models::new_id();
                    let t = models::date_now();
                    let a = Author {
                        id: Some(id.clone()),
                        name: Some(name.clone()),
                        last_first: Some(name_to_last_first(name)),
                        library_id: item.library_id.clone(),
                        created_at: Some(t.clone()),
                        updated_at: Some(t),
                        ..Default::default()
                    };
                    models::insert(db, &a)?;
                    lib::with_filter_data(item.library_id.as_deref().unwrap_or_default(), |fd| lib::push_named(fd, "authors", &id, name));
                    scan.emit("author_added", &super::author_json(&a));
                    id
                }
            };
            models::insert(
                db,
                &BookAuthor {
                    id: Some(models::new_id()),
                    created_at: Some(models::date_now()),
                    book_id: Some(book_id.clone()),
                    author_id: Some(author_id),
                },
            )?;
            authors_updated = true;
        }
        for a in &full.authors {
            if !m.authors.contains(&a.name) {
                db.execute("DELETE FROM bookAuthors WHERE bookId = ? AND authorId = ?", vec![json!(book_id), json!(a.id)])?;
                log.authors_num_books_changed.insert(a.id.clone());
                log.authors_removed_from_books.push(a.id.clone());
                authors_updated = true;
            }
        }
    }
    if !m.series.is_empty() {
        for s in &m.series {
            match full.series.iter().find(|x| x.name == s.name) {
                None => {
                    let series_id = match lib::series_id_by_name(db, item.library_id.as_deref().unwrap_or_default(), &s.name)? {
                        Some(id) => id,
                        None => {
                            let id = models::new_id();
                            let t = models::date_now();
                            models::insert(
                                db,
                                &Series {
                                    id: Some(id.clone()),
                                    name: Some(s.name.clone()),
                                    name_ignore_prefix: Some(title_ignore_prefix(&s.name, &scan.sorting)),
                                    library_id: item.library_id.clone(),
                                    created_at: Some(t.clone()),
                                    updated_at: Some(t),
                                    ..Default::default()
                                },
                            )?;
                            lib::with_filter_data(item.library_id.as_deref().unwrap_or_default(), |fd| {
                                lib::push_named(fd, "series", &id, &s.name)
                            });
                            id
                        }
                    };
                    models::insert(
                        db,
                        &BookSeries {
                            id: Some(models::new_id()),
                            sequence: s.sequence.clone(),
                            created_at: Some(models::date_now()),
                            book_id: Some(book_id.clone()),
                            series_id: Some(series_id),
                        },
                    )?;
                    series_updated = true;
                }
                Some(existing) => {
                    if let Some(seq) = s.sequence.as_ref().filter(|q| !q.is_empty()) {
                        if existing.sequence.as_ref() != Some(seq) {
                            db.execute(
                                "UPDATE bookSeries SET sequence = ? WHERE id = ?",
                                vec![json!(seq), json!(existing.book_series_id)],
                            )?;
                            series_updated = true;
                        }
                    }
                }
            }
        }
        for x in &full.series {
            if !m.series.iter().any(|s| s.name == x.name) {
                db.execute("DELETE FROM bookSeries WHERE id = ?", vec![json!(x.book_series_id)])?;
                log.series_removed_from_books.push(x.id.clone());
                series_updated = true;
            }
        }
    }
    // The cover: the details' cover replaces a missing one or one whose file is gone.
    if let Some(found) = m.text("coverPath") {
        match &cover {
            Some(c) if *c != found && !files::exists(c) => {
                cover = Some(found);
                has_media_changes = true;
            }
            None => {
                cover = Some(found);
                has_media_changes = true;
            }
            _ => {}
        }
    }
    let item_dir = if item.is_file == Some(true) { None } else { item.path.clone() };
    if cover.is_none() {
        cover = super::covers::save_embedded(scan, &audio_files, &item_id, item_dir.as_deref());
        if cover.is_none() {
            if let Some(es) = ebook_scan.as_ref().filter(|e| e.cover_path.is_some()) {
                cover = super::covers::save_ebook(scan, es, &item_id, item_dir.as_deref());
            }
        }
        if cover.is_some() {
            has_media_changes = true;
        }
    }
    let mut item_updated = false;
    if has_media_changes {
        bk.cover_path = cover;
        bk.audio_files = Some(Value::Array(audio_files.clone()));
        bk.ebook_file = ebook.clone();
        bk.duration = full.book.duration;
        bk.updated_at = Some(models::date_now());
        models::update(db, &bk)?;
        save_metadata_file(scan, &item_id, log)?;
    }
    let has_media = !audio_files.is_empty() || ebook.is_some();
    if !has_media && item.is_missing != Some(true) {
        log.info(format!("Book \"{}\" has no audio files and no ebook file. Setting library item as missing", m.title()));
        item.is_missing = Some(true);
        item_updated = true;
    } else if has_media && item.is_missing == Some(true) {
        item.is_missing = Some(false);
        item_updated = true;
    }
    let mut lfs: Vec<Value> = item.library_files.as_ref().and_then(Value::as_array).cloned().unwrap_or_default();
    for lf in lfs.iter_mut() {
        if ext_in(&files::file_ext(lf), EBOOK_TYPES) {
            let main = ebook.as_ref().is_some_and(|e| files::file_ino(lf) == files::file_ino(e));
            if lf["isSupplementary"] != json!(!main) {
                lf["isSupplementary"] = json!(!main);
                item_updated = true;
            }
        }
    }
    if item_updated {
        item.library_files = Some(Value::Array(lfs));
        item.updated_at = Some(models::date_now());
        models::update(db, item)?;
    }
    Ok(has_media_changes || item_updated || series_updated || authors_updated)
}

/// A detail of a stored book by the original's key, as JSON.
fn book_field(b: &Book, key: &str) -> Value {
    let s = |v: &Option<String>| v.clone().map(Value::String).unwrap_or(Value::Null);
    match key {
        "title" => s(&b.title),
        "titleIgnorePrefix" => s(&b.title_ignore_prefix),
        "subtitle" => s(&b.subtitle),
        "publishedYear" => s(&b.published_year),
        "publishedDate" => s(&b.published_date),
        "publisher" => s(&b.publisher),
        "description" => s(&b.description),
        "isbn" => s(&b.isbn),
        "asin" => s(&b.asin),
        "language" => s(&b.language),
        "explicit" => b.explicit.map(Value::Bool).unwrap_or(Value::Null),
        "abridged" => b.abridged.map(Value::Bool).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Sets a detail of a stored book by the original's key.
fn set_book_field(b: &mut Book, key: &str, v: &Value) {
    let s = v.as_str().map(String::from);
    match key {
        "title" => b.title = s,
        "titleIgnorePrefix" => b.title_ignore_prefix = s,
        "subtitle" => b.subtitle = s,
        "publishedYear" => b.published_year = s,
        "publishedDate" => b.published_date = s,
        "publisher" => b.publisher = s,
        "description" => b.description = s,
        "isbn" => b.isbn = s,
        "asin" => b.asin = s,
        "language" => b.language = s,
        "explicit" => b.explicit = v.as_bool(),
        "abridged" => b.abridged = v.as_bool(),
        _ => {}
    }
}

/// Updates a stored audio file from a new probe (`AudioFile.updateFromScan`): everything but the added and
/// updated times and the "manually verified" mark.
fn update_from_scan(af: &mut Value, scanned: &Value) {
    let Some(obj) = scanned.as_object() else { return };
    for (k, v) in obj {
        if ["manuallyVerified", "ctimeMs", "addedAt", "updatedAt"].contains(&k.as_str()) {
            continue;
        }
        if k == "metadata" {
            if let (Some(old), Some(new)) = (af["metadata"].as_object_mut(), v.as_object()) {
                for (mk, mv) in new {
                    if old.contains_key(mk) {
                        old.insert(mk.clone(), mv.clone());
                    }
                }
            }
            continue;
        }
        af[k] = v.clone();
    }
}

/// Removes authors that no book has any more and that have no details of their own
/// (`checkAuthorsRemovedFromBooks`), and series without books (`checkSeriesRemovedFromBooks`).
pub fn finalize_authors_and_series(scan: &Scan, library_id: &str, log: &mut ScanLog) -> Result<()> {
    let db = scan.db;
    if !log.authors_num_books_changed.is_empty() {
        let ids: Vec<String> = log.authors_num_books_changed.drain().collect();
        let mut authors = Vec::new();
        for id in &ids {
            let n = db
                .one("SELECT count(*) AS n FROM bookAuthors WHERE authorId = ?", vec![json!(id)])?
                .and_then(|r| r.get("n").and_then(Value::as_i64))
                .unwrap_or(0);
            authors.push(json!({"id": id, "numBooks": n}));
        }
        if !authors.is_empty() {
            scan.emit("authors_num_books_updated", &json!({"libraryId": library_id, "authors": authors}));
        }
    }
    for id in std::mem::take(&mut log.authors_removed_from_books) {
        let row = db.one(
            "SELECT id FROM authors AS author WHERE id = ? AND (asin IS NULL OR asin = '') AND (description IS NULL OR description = '') AND (imagePath IS NULL OR imagePath = '') AND (SELECT count(*) FROM bookAuthors ba WHERE ba.authorId = author.id) = 0",
            vec![json!(id)],
        )?;
        if row.is_some() {
            db.execute("DELETE FROM authors WHERE id = ?", vec![json!(id)])?;
            lib::with_filter_data(library_id, |fd| lib::remove_named(fd, "authors", &id));
            scan.emit("author_removed", &json!({"id": id, "libraryId": library_id}));
        }
    }
    for id in std::mem::take(&mut log.series_removed_from_books) {
        let row = db.one(
            "SELECT id FROM series WHERE id = ? AND (SELECT count(*) FROM bookSeries bs WHERE bs.seriesId = series.id) = 0",
            vec![json!(id)],
        )?;
        if row.is_some() {
            db.execute("DELETE FROM series WHERE id = ?", vec![json!(id)])?;
            lib::with_filter_data(library_id, |fd| lib::remove_named(fd, "series", &id));
            scan.emit("series_removed", &json!({"id": id, "libraryId": library_id}));
        }
    }
    Ok(())
}
