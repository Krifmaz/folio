//! Grouping a folder's files into library items and reading book details from folder names, like the original's
//! `utils/scandir.js`.

use super::files::{self, ext_in, FileItem, AUDIO_TYPES, EBOOK_TYPES, IMAGE_TYPES, METADATA_TYPES, TEXT_TYPES};
use crate::abs::library::names::parse_names;
use regex::Regex;
use std::sync::LazyLock;

/// Whether a file is media for a library (`isMediaFile`): audio, or for book libraries ebooks too.
pub fn is_media_file(media_type: &str, ext: &str, audiobooks_only: bool) -> bool {
    if ext.is_empty() {
        return false;
    }
    if media_type == "podcast" || audiobooks_only {
        return ext_in(ext, AUDIO_TYPES);
    }
    ext_in(ext, AUDIO_TYPES) || ext_in(ext, EBOOK_TYPES)
}

/// Whether a file that isn't media still matters to a rescan (`isScannableNonMediaFile`).
pub fn is_scannable_non_media(ext: &str) -> bool {
    !ext.is_empty() && (ext_in(ext, TEXT_TYPES) || ext_in(ext, METADATA_TYPES) || ext_in(ext, IMAGE_TYPES))
}

/// A folder (or a single file at the top) found to be a library item, and its files below it.
#[derive(Clone, Debug, PartialEq)]
pub enum Group {
    /// A media file at the top of the library folder: the item is the file itself.
    Single(String),
    /// A folder: its files, as paths below it.
    Folder(Vec<String>),
}

static CD_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(cd|dis[ck])\s*\d{1,3}$").unwrap());

/// Groups files into library item folders (`groupFileItemsIntoLibraryItemDirs`), in the order they were first met.
/// A folder becomes an item at the first level that holds media; a last "CD 1" or "Disc 2" folder belongs to its
/// parent. Other files join the item whose folder holds them.
pub fn group_items(media_type: &str, items: &[FileItem], audiobooks_only: bool, include_non_media: bool) -> Vec<(String, Group)> {
    let filtered: Vec<&FileItem> =
        items.iter().filter(|i| i.deep > 0 || (media_type == "book" && is_media_file(media_type, &i.extension, audiobooks_only))).collect();
    let (media, other): (Vec<&FileItem>, Vec<&FileItem>) = filtered.into_iter().partition(|i| {
        is_media_file(media_type, &i.extension, audiobooks_only) || (include_non_media && is_scannable_non_media(&i.extension))
    });
    let mut groups: Vec<(String, Group)> = Vec::new();
    let find = |groups: &Vec<(String, Group)>, key: &str| groups.iter().position(|(k, _)| k == key);
    for item in media {
        let mut parts: Vec<&str> = item.reldirpath.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            if find(&groups, &item.name).is_none() {
                groups.push((item.name.clone(), Group::Single(item.name.clone())));
            }
            continue;
        }
        let mut path = String::new();
        let n = parts.len();
        for _ in 0..n {
            let part = parts.remove(0);
            path = files::join(&[&path, part]);
            if let Some(i) = find(&groups, &path) {
                let rel = files::join(&[&parts.join("/"), &item.name]);
                if let Group::Folder(list) = &mut groups[i].1 {
                    list.push(rel);
                }
                break;
            } else if parts.is_empty() {
                groups.push((path.clone(), Group::Folder(vec![item.name.clone()])));
                break;
            } else if parts.len() == 1 && CD_DIR.is_match(parts[0]) {
                groups.push((path.clone(), Group::Folder(vec![files::join(&[parts[0], &item.name])])));
                break;
            }
        }
    }
    for item in other {
        let mut parts: Vec<&str> = item.reldirpath.split('/').collect();
        let mut path = String::new();
        let n = parts.len();
        for _ in 0..n {
            let part = parts.remove(0);
            path = files::join(&[&path, part]);
            if let Some(i) = find(&groups, &path) {
                let rel = files::join(&[&parts.join("/"), &item.name]);
                if let Group::Folder(list) = &mut groups[i].1 {
                    list.push(rel);
                }
                break;
            }
        }
    }
    groups
}

/// Book details read from an item's folder names (`LibraryItemFilenameMetadata`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderMetadata {
    /// The title (the last folder).
    pub title: String,
    /// The subtitle, when subtitles are read from folder names.
    pub subtitle: Option<String>,
    /// An ASIN in square brackets.
    pub asin: Option<String>,
    /// The authors (the folder two or more levels up).
    pub authors: Vec<String>,
    /// Narrators in curly brackets.
    pub narrators: Vec<String>,
    /// The series (the folder above the title, when there are two above it).
    pub series_name: Option<String>,
    /// The book's number in the series.
    pub series_sequence: Option<String>,
    /// A year before the title.
    pub published_year: Option<String>,
    /// Set for podcasts and single files: only the title is known.
    pub title_only: bool,
}

/// Reads author, series, number, year, narrators and ASIN from a book's path (`getBookDataFromDir`).
pub fn book_data_from_dir(rel_path: &str, parse_subtitle: bool) -> FolderMetadata {
    let mut split: Vec<&str> = rel_path.split('/').collect();
    let title_folder = split.pop().unwrap_or_default().to_string();
    let series = if split.len() > 1 { split.pop().map(String::from) } else { None };
    let author = if !split.is_empty() { split.pop().map(String::from) } else { None };
    let (after_asin, asin) = get_asin(&title_folder);
    let (after_narrators, narrators) = get_narrator(&after_asin);
    let (after_sequence, sequence) = if series.is_some() { get_sequence(&after_narrators) } else { (after_narrators, None) };
    let (after_year, year) = get_published_year(&after_sequence);
    let (title, subtitle) = if parse_subtitle { get_subtitle(&after_year) } else { (after_year, None) };
    FolderMetadata {
        title,
        subtitle,
        asin,
        authors: author.as_deref().map(parse_names).unwrap_or_default(),
        narrators: narrators.as_deref().map(parse_names).unwrap_or_default(),
        series_name: series,
        series_sequence: sequence,
        published_year: year,
        title_only: false,
    }
}

static NARRATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<title>.*) \{(?P<narrators>.*)\}$").unwrap());

/// Takes "{Narrator}" off the end of a folder name (`getNarrator`).
fn get_narrator(folder: &str) -> (String, Option<String>) {
    match NARRATOR.captures(folder) {
        Some(c) => (c["title"].to_string(), Some(c["narrators"].to_string())),
        None => (folder.to_string(), None),
    }
}

static SEQUENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?P<label>vol\.? |volume |book )?(?P<seq>\d{0,3}(?:\.\d{1,2})?)(?P<dot>\.?)(?: (?P<suffix>.*))?$").unwrap()
});

/// Takes the series number out of a folder name (`getSequence`): "Book 2 - Title", "6. Title", "0.5 - Title".
fn get_sequence(folder: &str) -> (String, Option<String>) {
    let mut parts: Vec<String> = folder.split(" - ").map(String::from).collect();
    let mut volume = None;
    for i in 0..parts.len() {
        let Some(c) = SEQUENCE.captures(&parts[i]) else { continue };
        let suffix = c.name("suffix").map(|m| m.as_str().to_string());
        let labelled = c.name("label").is_some_and(|m| !m.as_str().is_empty()) || c.name("dot").is_some_and(|m| !m.as_str().is_empty());
        if suffix.is_some() && !labelled {
            continue;
        }
        let seq = c.name("seq").map(|m| m.as_str()).unwrap_or("");
        // `isNaN(seq) ? seq : Number(seq).toString()`: "" and digits are numbers ("" becomes "0").
        volume = Some(match seq.parse::<f64>() {
            Ok(n) => crate::abs::library::util::js_num(n).to_string(),
            Err(_) if seq.is_empty() => "0".to_string(),
            Err(_) => seq.to_string(),
        });
        match suffix {
            Some(s) if !s.is_empty() => parts[i] = s,
            _ => {
                parts.remove(i);
            }
        }
        break;
    }
    (parts.join(" - "), volume)
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^ *\(?([0-9]{4})\)? * - *(.+)").unwrap());

/// Takes "1999 - " or "(1999) - " off the front of a folder name (`getPublishedYear`).
fn get_published_year(folder: &str) -> (String, Option<String>) {
    match YEAR.captures(folder) {
        Some(c) => (c[2].to_string(), Some(c[1].to_string())),
        None => (folder.to_string(), None),
    }
}

/// Splits "Title - Subtitle" (`getSubtitle`).
fn get_subtitle(folder: &str) -> (String, Option<String>) {
    let mut split = folder.split(" - ");
    let title = split.next().unwrap_or("").to_string();
    (title, Some(split.collect::<Vec<_>>().join(" - ")))
}

static ASIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?: |^)\[([A-Z0-9]{10})\]").unwrap());

/// Takes "[B0015T963C]" out of a folder name (`getASIN`); the brackets must end the name or come before a space.
fn get_asin(folder: &str) -> (String, Option<String>) {
    for c in ASIN.captures_iter(folder) {
        let whole = c.get(0).unwrap();
        let rest = &folder[whole.end()..];
        if rest.is_empty() || rest.starts_with(' ') {
            let asin = c[1].to_string();
            let replaced = folder.replacen(whole.as_str(), "", 1);
            return (replaced.trim().to_string(), Some(asin));
        }
    }
    (folder.trim().to_string(), None)
}

/// The details of a media folder (`getDataFromMediaDir`): its whole path, its path below the library folder and
/// what its names say.
pub fn data_from_media_dir(media_type: &str, folder_path: &str, rel_path: &str, parse_subtitle: bool) -> (String, String, FolderMetadata) {
    let full = files::join(&[folder_path, rel_path]);
    let meta = if media_type == "podcast" {
        FolderMetadata { title: rel_path.rsplit('/').next().unwrap_or("").to_string(), title_only: true, ..Default::default() }
    } else {
        book_data_from_dir(rel_path, parse_subtitle)
    };
    (full, rel_path.to_string(), meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, deep: usize) -> FileItem {
        let name = path.rsplit('/').next().unwrap().to_string();
        let dir = files::dirname(path);
        FileItem {
            extension: files::extname(&name),
            name,
            path: path.into(),
            reldirpath: if dir == "." { String::new() } else { dir },
            fullpath: format!("/m/{path}"),
            deep,
        }
    }

    #[test]
    fn groups_like_the_original() {
        let items = vec![
            item("top.m4b", 0),
            item("notes.txt", 0),
            item("A/B/1.mp3", 2),
            item("A/B/cover.jpg", 2),
            item("A/C/CD 1/1.mp3", 3),
            item("A/C/CD 2/1.mp3", 3),
        ];
        let g = group_items("book", &items, false, false);
        assert_eq!(
            g,
            vec![
                ("top.m4b".into(), Group::Single("top.m4b".into())),
                ("A/B".into(), Group::Folder(vec!["1.mp3".into(), "cover.jpg".into()])),
                ("A/C".into(), Group::Folder(vec!["CD 1/1.mp3".into(), "CD 2/1.mp3".into()])),
            ]
        );
    }

    #[test]
    fn folder_names_like_the_original() {
        let m = book_data_from_dir("Ann Leckie/Imperial Radch/1 - Ancillary Justice", false);
        assert_eq!(
            (m.title.as_str(), m.series_sequence.as_deref(), m.authors.clone()),
            ("Ancillary Justice", Some("1"), vec!["Ann Leckie".to_string()])
        );
        let m = book_data_from_dir("Author/Series/Vol. 3 Title Here - Sub [B0015T963C] {Reader One, Reader Two}", true);
        assert_eq!(m.asin.as_deref(), Some("B0015T963C"));
        assert_eq!(m.narrators, vec!["Reader One".to_string(), "Reader Two".to_string()]);
        assert_eq!(m.series_sequence.as_deref(), Some("3"));
        assert_eq!((m.title.as_str(), m.subtitle.as_deref()), ("Title Here", Some("Sub")));
        let m = book_data_from_dir("Author/Series/101 Dalmatians", false);
        assert_eq!((m.title.as_str(), m.series_sequence.as_deref()), ("101 Dalmatians", None));
        let m = book_data_from_dir("Author/Series/1980 - Book 02 - Title", false);
        assert_eq!((m.title.as_str(), m.series_sequence.as_deref(), m.published_year.as_deref()), ("Title", Some("2"), Some("1980")));
        let m = book_data_from_dir("Author/Title", false);
        assert_eq!((m.title.as_str(), m.series_name.as_deref()), ("Title", None));
    }
}
