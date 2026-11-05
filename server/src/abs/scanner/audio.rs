//! Audio files in a book: track numbers from names and tags, the track order, book details from the first
//! file's tags and chapters from the files, like the original's `AudioFileScanner` and
//! `parseOverdriveMediaMarkers`.

use super::files;
use super::meta::{parse_series_string, BookMeta, SeriesRef};
use super::scandir::FolderMetadata;
use crate::abs::library::names::parse_names;
use crate::abs::library::util::js_num;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// The track and disc numbers in an audio file's name (`getTrackAndDiscNumberFromFilename`). Like the original,
/// only the title and the year are taken out of the name first, and the year is replaced by "undefined".
pub fn numbers_from_filename(meta: &FolderMetadata, filename: &str, path: &str) -> (Option<i64>, Option<i64>) {
    static DISC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)(disc|cd) ?(\d\d?)(?-u:\b)").unwrap());
    static CD_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(cd|dis[ck])\s*\d{1,3}$").unwrap());
    static CD_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(cd|dis[ck])\s*").unwrap());
    static NUMBERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{1,4}").unwrap());
    let mut part = files::basename(filename, Some(&files::extname(filename)));
    if !meta.title.is_empty() {
        part = part.replacen(&meta.title, "", 1);
    }
    if let Some(year) = meta.published_year.as_deref().filter(|y| !y.is_empty()) {
        part = part.replacen(year, "undefined", 1);
    }
    let mut disc = None;
    if let Some(c) = DISC.captures(&part) {
        disc = c[2].parse::<i64>().ok();
        part = DISC.replace(&part, "").into_owned();
    }
    let dir = files::dirname(path);
    let last = dir.rsplit('/').next().unwrap_or("");
    if !last.is_empty() && CD_DIR.is_match(last) {
        if let Ok(n) = CD_PREFIX.replace(last, "").trim().parse::<i64>() {
            disc = Some(n);
        }
    }
    let track = NUMBERS.find(&part).and_then(|m| m.as_str().parse::<i64>().ok());
    (track, disc)
}

/// Whether sorted numbers have no gaps bigger than one (`isSequential`).
fn is_sequential(nums: &[i64]) -> bool {
    if nums.is_empty() {
        return false;
    }
    nums.windows(2).all(|w| w[1] - w[0] <= 1)
}

/// Sorted numbers with repeats (and anything not bigger than the last kept) left out (`removeDupes`).
fn remove_dupes(nums: &[i64]) -> Vec<i64> {
    if nums.len() <= 1 {
        return nums.to_vec();
    }
    let mut out = vec![nums[0]];
    for n in nums {
        if *n > *out.last().unwrap() {
            out.push(*n);
        }
    }
    out
}

/// Orders a book's audio files and numbers them from 1 (`runSmartTrackOrder`): by disc when every file has a disc
/// number in sequence, then by track number from the tags or from the names, whichever has more different numbers.
pub fn smart_track_order(files: &mut [Value]) {
    if files.is_empty() {
        return;
    }
    let collect = |key: &str| -> Vec<i64> {
        let mut v: Vec<i64> = files.iter().filter_map(|f| f[key].as_i64()).collect();
        v.sort();
        v
    };
    let discs_name = collect("discNumFromFilename");
    let discs_meta = collect("discNumFromMeta");
    let tracks_name = remove_dupes(&collect("trackNumFromFilename"));
    let tracks_meta = remove_dupes(&collect("trackNumFromMeta"));
    let disc_key = if discs_meta.len() == files.len() && is_sequential(&discs_meta) {
        Some("discNumFromMeta")
    } else if discs_name.len() == files.len() && is_sequential(&discs_name) {
        Some("discNumFromFilename")
    } else {
        None
    };
    let track_key = if tracks_name.len() > tracks_meta.len() { "trackNumFromFilename" } else { "trackNumFromMeta" };
    // A missing number counts as 0, as `a[key] - b[key]` does with null.
    let n = |f: &Value, k: &str| f[k].as_f64().unwrap_or(0.0);
    files.sort_by(|a, b| {
        let mut d = disc_key.map(|k| n(a, k) - n(b, k)).unwrap_or(0.0);
        if d == 0.0 {
            d = n(a, track_key) - n(b, track_key);
        }
        d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (i, f) in files.iter_mut().enumerate() {
        f["index"] = json!(i + 1);
    }
}

/// Splits a genre tag on "/", "//" or ";" (`parseGenresString`).
pub fn parse_genres(tag: &str) -> Vec<String> {
    if tag.is_empty() {
        return vec![];
    }
    for sep in ["/", "//", ";"] {
        if tag.contains(sep) {
            return tag.split(sep).map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect();
        }
    }
    vec![tag.to_string()]
}

/// Sets a book's details from its first audio file's tags, then its chapters (`setBookMetadataFromAudioMetaTags`).
pub fn book_meta_from_tags(book_title: &str, audio: &[Value], book: &mut BookMeta) {
    let Some(first) = audio.first() else { return };
    let tags = &first["metaTags"];
    let tag = |k: &str| tags.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(String::from);
    let map: &[(&str, Option<&str>, &str)] = &[
        ("tagComposer", None, "narrators"),
        ("tagDescription", Some("tagComment"), "description"),
        ("tagPublisher", None, "publisher"),
        ("tagDate", None, "publishedYear"),
        ("tagSubtitle", None, "subtitle"),
        ("tagAlbum", Some("tagTitle"), "title"),
        ("tagArtist", Some("tagAlbumArtist"), "authors"),
        ("tagGenre", None, "genres"),
        ("tagSeries", Some("tagGrouping"), "series"),
        ("tagIsbn", None, "isbn"),
        ("tagLanguage", None, "language"),
        ("tagASIN", None, "asin"),
    ];
    for (t, alt, key) in map {
        let mut value = tag(t);
        let mut is_alt = false;
        if value.is_none() {
            if let Some(a) = alt {
                value = tag(a);
                is_alt = true;
            }
        }
        let Some(value) = value.map(|v| v.trim().to_string()) else { continue };
        match *key {
            "narrators" => book.narrators = parse_names(&value),
            "authors" => book.authors = parse_names(&value),
            "genres" => book.genres = parse_genres(&value),
            "series" => {
                if is_alt {
                    let series: Vec<SeriesRef> = value.split(';').filter_map(|s| parse_series_string(s.trim())).collect();
                    if !series.is_empty() {
                        book.series = series;
                    }
                } else {
                    let part = tag("tagSeriesPart");
                    if value.contains(';') && part.as_deref().is_some_and(|p| p.contains(';')) {
                        let names: Vec<String> = value.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        let seqs: Vec<String> =
                            part.as_deref().unwrap_or("").split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                        if names.len() > 1 && names.len() == seqs.len() {
                            book.series = names
                                .into_iter()
                                .zip(seqs)
                                .map(|(name, seq)| SeriesRef { name, sequence: Some(seq).filter(|s| !s.is_empty()) })
                                .collect();
                            continue;
                        }
                    }
                    book.series = vec![SeriesRef { name: value, sequence: part }];
                }
            }
            _ => book.set(key, Value::String(value)),
        }
    }
    let chapters = chapters_from_files(book_title, audio);
    if !chapters.is_empty() {
        book.chapters = chapters;
    }
}

/// A number from a stored audio file.
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// A book's chapters from its audio files (`getBookChaptersFromAudioFiles`): OverDrive markers, else the first
/// file's chapters (or every file's), else one chapter per file named by its title tags or file names.
pub fn chapters_from_files(book_title: &str, audio: &[Value]) -> Vec<Value> {
    if let Some(od) = overdrive_chapters(audio).filter(|c| !c.is_empty()) {
        return od;
    }
    let mut chapters: Vec<Value> = Vec::new();
    let first_chapters = audio[0]["chapters"].as_array().cloned().unwrap_or_default();
    if !first_chapters.is_empty() {
        let same = audio.len() == 1
            || (audio.len() > 1
                && audio[1]["chapters"].as_array().is_some_and(|second| {
                    second.len() == first_chapters.len()
                        && first_chapters.iter().zip(second).all(|(a, b)| a["title"] == b["title"] && a["start"] == b["start"])
                }));
        if same {
            return first_chapters;
        }
        let mut id = 0usize;
        let mut start = 0.0;
        for file in audio {
            let dur = f(&file["duration"]);
            if dur == 0.0 {
                continue;
            }
            let cleaned: Vec<Value> = file["chapters"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|c| f(&c["end"]) - f(&c["start"]) >= 0.1)
                .collect();
            for (i, c) in cleaned.iter().enumerate() {
                let mut c = c.clone();
                c["id"] = json!(id + i);
                c["start"] = js_num(f(&c["start"]) + start);
                c["end"] = js_num(f(&c["end"]) + start);
                chapters.push(c);
            }
            id += cleaned.len();
            start += dur;
        }
        return chapters;
    } else if audio.len() > 1 {
        let mut titles: Vec<String> = Vec::new();
        for t in audio.iter().filter_map(|a| a["metaTags"]["tagTitle"].as_str()).filter(|t| !t.is_empty() && *t != book_title) {
            if !titles.iter().any(|x| x == t) {
                titles.push(t.to_string());
            }
        }
        let use_tags = titles.len() == audio.len();
        let mut id = 0usize;
        let mut start = 0.0;
        for file in audio {
            let dur = f(&file["duration"]);
            if dur == 0.0 {
                continue;
            }
            let name = file["metadata"]["filename"].as_str().unwrap_or("");
            let mut title = if name.is_empty() { format!("Chapter {id}") } else { files::basename(name, Some(&files::extname(name))) };
            if use_tags {
                title = file["metaTags"]["tagTitle"].as_str().unwrap_or("").to_string();
            }
            chapters.push(json!({"id": id, "start": js_num(start), "end": js_num(start + dur), "title": title}));
            id += 1;
            start += dur;
        }
    }
    chapters
}

/// Chapters from OverDrive's media markers in the files' tags (`parseOverdriveMediaMarkersAsChapters`), when
/// every file has them.
fn overdrive_chapters(audio: &[Value]) -> Option<Vec<Value>> {
    static WEIRD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([(]\d|[cC]ontinued)").unwrap());
    let markers: Vec<&str> =
        audio.iter().filter_map(|a| a["metaTags"]["tagOverdriveMediaMarker"].as_str()).filter(|m| !m.is_empty()).collect();
    if markers.is_empty() {
        return None;
    }
    let mut cleaned: Vec<Vec<(String, String)>> = Vec::new();
    for m in markers {
        let Some(json) = super::xml::to_json(m) else { continue };
        let Some(list) = json.pointer("/Markers/Marker").and_then(Value::as_array) else { continue };
        let text = |v: &Value| -> String {
            match v {
                Value::Array(a) => {
                    a.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| "[object Object]".into())).collect::<Vec<_>>().join(",")
                }
                Value::String(s) => s.clone(),
                _ => String::new(),
            }
        };
        let parsed: Vec<(String, String)> =
            list.iter().map(|mk| (text(&mk["Name"]), text(&mk["Time"]))).filter(|(name, _)| !WEIRD.is_match(name)).collect();
        cleaned.push(parsed);
    }
    if cleaned.len() != audio.len() {
        return None;
    }
    let mut out: Vec<Value> = Vec::new();
    let mut length = 0.0;
    for (i, track) in audio.iter().enumerate() {
        for (name, time) in &cleaned[i] {
            let mut parts: Vec<&str> = time.split(':').collect();
            let mut t = length + parse_float(parts.pop().unwrap_or("0"));
            if let Some(m) = parts.pop() {
                t += parse_float(m) * 60.0;
            }
            if let Some(h) = parts.pop() {
                t += parse_float(h) * 3600.0;
            }
            out.push(json!({"id": out.len(), "start": js_num(t), "title": name}));
        }
        length += f(&track["duration"]);
    }
    let n = out.len();
    for i in 0..n {
        let end = if i + 1 < n { out[i + 1]["start"].clone() } else { js_num(length) };
        out[i]["end"] = end;
    }
    Some(out)
}

/// JavaScript's `parseFloat`: the number at the start of the text (NaN when there is none).
pub fn parse_float(text: &str) -> f64 {
    static NUM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?").unwrap());
    let t = text.trim_start();
    if let Some(rest) = t.strip_prefix("Infinity").or_else(|| t.strip_prefix("+Infinity")) {
        let _ = rest;
        return f64::INFINITY;
    }
    NUM.find(t).and_then(|m| m.as_str().parse::<f64>().ok()).unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, track_meta: Option<i64>, duration: f64, title: Option<&str>) -> Value {
        json!({"metadata": {"filename": name, "path": format!("/b/{name}")}, "trackNumFromMeta": track_meta, "discNumFromMeta": null,
            "trackNumFromFilename": null, "discNumFromFilename": null, "duration": duration, "chapters": [],
            "metaTags": title.map(|t| json!({"tagTitle": t})).unwrap_or(json!({}))})
    }

    #[test]
    fn numbers_from_names_like_the_original() {
        let m = FolderMetadata { title: "Book".into(), published_year: Some("1999".into()), ..Default::default() };
        assert_eq!(numbers_from_filename(&m, "Book - Part 03.mp3", "/a/Book - Part 03.mp3"), (Some(3), None));
        assert_eq!(numbers_from_filename(&m, "Disc 2 - 07.mp3", "/a/Disc 2 - 07.mp3"), (Some(7), Some(2)));
        assert_eq!(numbers_from_filename(&m, "05.mp3", "/a/CD 3/05.mp3"), (Some(5), Some(3)));
        assert_eq!(numbers_from_filename(&m, "1999 intro.mp3", "/a/1999 intro.mp3"), (None, None));
    }

    #[test]
    fn orders_and_names_chapters_like_the_original() {
        let mut files = vec![file("b.mp3", Some(2), 15.0, Some("Part 2")), file("a.mp3", Some(1), 15.0, Some("Part 1"))];
        smart_track_order(&mut files);
        assert_eq!(files[0]["metadata"]["filename"], "a.mp3");
        assert_eq!(files[1]["index"], 2);
        let ch = chapters_from_files("Book", &files);
        assert_eq!(
            ch,
            vec![json!({"id": 0, "start": 0, "end": 15, "title": "Part 1"}), json!({"id": 1, "start": 15, "end": 30, "title": "Part 2"})]
        );
        let files = vec![file("a.mp3", None, 10.0, Some("Book")), file("b.mp3", None, 10.0, Some("Book"))];
        assert_eq!(chapters_from_files("Book", &files)[1]["title"], "b");
        assert_eq!(parse_genres("Sci-Fi / Fantasy"), vec!["Sci-Fi".to_string(), "Fantasy".to_string()]);
    }

    #[test]
    fn overdrive_markers() {
        let mut a = file("a.mp3", None, 100.0, None);
        a["metaTags"]["tagOverdriveMediaMarker"] = json!("<Markers><Marker><Name>One</Name><Time>0:00.000</Time></Marker><Marker><Name>One (continued)</Name><Time>0:30.000</Time></Marker><Marker><Name>Two</Name><Time>1:00.500</Time></Marker></Markers>");
        let ch = chapters_from_files("Book", &[a]);
        assert_eq!(
            ch,
            vec![json!({"id": 0, "start": 0, "end": 60.5, "title": "One"}), json!({"id": 1, "start": 60.5, "end": 100, "title": "Two"})]
        );
    }
}
