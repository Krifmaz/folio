//! The book details a scan gathers (the original's `bookMetadata` object) and the readers that fill it: OPF and
//! EPUB package files (`parseOpfMetadata`, `parseEpubMetadata`), NFO files (`parseNfoMetadata`) and
//! `metadata.json` (`abmetadataGenerator.parseJson`).

use super::xml;
use crate::abs::html::strip_all_tags;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::io::Read;
use std::sync::LazyLock;

/// A series a book is in, with its number when known.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesRef {
    /// The series' name.
    pub name: String,
    /// The book's number in it.
    pub sequence: Option<String>,
}

/// What a scan knows about a book. Text and switches not set yet are missing from `fields` (the original's
/// `undefined`); lists start empty.
#[derive(Clone, Debug, Default)]
pub struct BookMeta {
    /// title, subtitle, publishedYear, publishedDate, publisher, description, isbn, asin, language, explicit,
    /// abridged, coverPath and titleIgnorePrefix, when set.
    pub fields: Map<String, Value>,
    /// The narrators.
    pub narrators: Vec<String>,
    /// The genres.
    pub genres: Vec<String>,
    /// The tags.
    pub tags: Vec<String>,
    /// The authors' names.
    pub authors: Vec<String>,
    /// The series.
    pub series: Vec<SeriesRef>,
    /// The chapters (`{id, start, end, title}`).
    pub chapters: Vec<Value>,
}

impl BookMeta {
    /// A text field, when set to text.
    pub fn text(&self, key: &str) -> Option<String> {
        self.fields.get(key).and_then(Value::as_str).map(String::from)
    }

    /// Sets a field.
    pub fn set(&mut self, key: &str, value: Value) {
        self.fields.insert(key.to_string(), value);
    }

    /// The title ("" when unset).
    pub fn title(&self) -> String {
        self.text("title").unwrap_or_default()
    }
}

/// Details read from an OPF package (`parseOpfMetadataJson`'s result).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpfMeta {
    /// Text details in the original's key order: title, subtitle, publishedYear, publisher, isbn, asin,
    /// description, language (`None` when missing).
    pub texts: Vec<(&'static str, Option<String>)>,
    /// The authors.
    pub authors: Vec<String>,
    /// The narrators.
    pub narrators: Vec<String>,
    /// The genres (`dc:subject`).
    pub genres: Vec<String>,
    /// The series (calibre's meta tags).
    pub series: Vec<SeriesRef>,
    /// The tags (`dc:tag`).
    pub tags: Vec<String>,
}

/// The first value of a tag, as text (`fetchTagString`).
fn tag_string(meta: &Value, tag: &str) -> Option<String> {
    let v = meta.get(tag)?.as_array()?.first()?;
    match v {
        Value::Object(o) => o.get("_").and_then(Value::as_str).map(String::from),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// The namespace prefix an element declares on itself, or "opf".
fn ns_prefix(attrs: &Map<String, Value>) -> String {
    attrs.keys().find(|k| k.starts_with("xmlns:")).and_then(|k| k.split(':').nth(1)).unwrap_or("opf").to_string()
}

/// Strings in a list, without repeats, in order.
fn unique_strings(list: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in list.and_then(Value::as_array).into_iter().flatten() {
        if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
            if !out.iter().any(|o| o == s) {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// Reads a package document already turned into `xml2js` shape (`parseOpfMetadataJson`).
pub fn opf_from_json(json: &Value) -> Option<OpfMeta> {
    let obj = json.as_object()?;
    let package_key = obj.keys().find(|k| k.rsplit(':').next() == Some("package"))?.clone();
    let prefix = package_key.split(':').next().unwrap_or("").to_string();
    let package = &obj[&package_key];
    let mut metadata = package.get(format!("{prefix}:metadata")).or_else(|| package.get("metadata"))?.clone();
    if let Value::Array(list) = &metadata {
        metadata = list.first()?.clone();
    }
    let meta_list = metadata.get(format!("{prefix}:meta")).or_else(|| metadata.get("meta")).cloned();
    // `metadata.meta` becomes a map of named metas plus the `refines` list.
    let mut named = Map::new();
    let mut refines: Vec<Value> = Vec::new();
    for m in meta_list.as_ref().and_then(Value::as_array).into_iter().flatten() {
        let attrs = m.get("$");
        if let Some(name) = attrs.and_then(|a| a.get("name")).and_then(Value::as_str).filter(|n| !n.is_empty()) {
            let content = attrs.and_then(|a| a.get("content")).and_then(Value::as_str).unwrap_or("");
            named.insert(name.to_string(), json!([content]));
        } else if let Some(r) = attrs.and_then(|a| a.get("refines")).and_then(Value::as_str).filter(|r| !r.is_empty()) {
            refines.push(json!({"value": m.get("_").cloned().unwrap_or(Value::Null), "refines": r, "property": attrs.and_then(|a| a.get("property")).cloned().unwrap_or(Value::Null)}));
        }
    }
    // Creators: (name, role, file-as); a creator without attributes or text is left out of every role.
    let mut creators: Vec<(String, Option<String>)> = Vec::new();
    for c in metadata.get("dc:creator").and_then(Value::as_array).into_iter().flatten() {
        let (Some(attrs), Some(text)) =
            (c.get("$").and_then(Value::as_object), c.get("_").and_then(Value::as_str).filter(|t| !t.is_empty()))
        else {
            continue;
        };
        let ns = ns_prefix(attrs);
        let mut role = attrs.get(&format!("{ns}:role")).and_then(Value::as_str).filter(|r| !r.is_empty()).map(String::from);
        if let Some(id) = attrs.get("id").and_then(Value::as_str).filter(|i| !i.is_empty()) {
            let target = format!("#{id}");
            if let Some(r) = refines.iter().find(|r| r["refines"] == target.as_str() && r["property"] == "role") {
                if let Some(v) = r["value"].as_str().filter(|v| !v.is_empty()) {
                    role = Some(v.to_string());
                }
            }
        }
        creators.push((text.to_string(), role));
    }
    let by_role = |role: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (name, r) in &creators {
            if r.as_deref() == Some(role) && !out.contains(name) {
                out.push(name.clone());
            }
        }
        out
    };
    let authors: Vec<String> = by_role("aut").into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    let mut narrators = by_role("nrt");
    if narrators.is_empty() {
        let calibre =
            named.get("calibre:user_metadata:#narrators").and_then(|v| v.get(0)).and_then(Value::as_str).map(|s| s.replace("&quot;", "\""));
        narrators = calibre
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("#value#").and_then(Value::as_array).cloned())
            .map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
    }
    let narrators: Vec<String> = narrators.into_iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    let date = tag_string(&metadata, "dc:date").and_then(|d| {
        let first = d.split('-').next().unwrap_or("").to_string();
        (first.chars().count() == 4 && crate::abs::library::js_number(&first).is_some()).then_some(first)
    });
    let identifier = |scheme: &str| -> Option<String> {
        metadata.get("dc:identifier").and_then(Value::as_array)?.iter().find_map(|i| {
            let attrs = i.get("$")?.as_object()?;
            let ns = ns_prefix(attrs);
            (attrs.get(&format!("{ns}:scheme")).and_then(Value::as_str) == Some(scheme))
                .then(|| i.get("_").and_then(Value::as_str).map(String::from))
                .flatten()
        })
    };
    let description = tag_string(&metadata, "dc:description")
        .filter(|d| !d.is_empty())
        .map(|d| strip_all_tags(Some(&d.replace("&lt;", "<").replace("&gt;", ">"))));
    Some(OpfMeta {
        texts: vec![
            ("title", tag_string(&metadata, "dc:title")),
            ("subtitle", tag_string(&metadata, "dc:subtitle")),
            ("publishedYear", date),
            ("publisher", tag_string(&metadata, "dc:publisher")),
            ("isbn", identifier("ISBN")),
            ("asin", identifier("ASIN")),
            ("description", description),
            ("language", tag_string(&metadata, "dc:language")),
        ],
        authors,
        narrators,
        genres: unique_strings(metadata.get("dc:subject")),
        series: calibre_series(meta_list.as_ref()),
        tags: unique_strings(metadata.get("dc:tag")),
    })
}

/// Calibre's series and series numbers (`fetchSeries`).
fn calibre_series(meta: Option<&Value>) -> Vec<SeriesRef> {
    let Some(list) = meta.and_then(Value::as_array) else { return vec![] };
    let attr = |m: &Value, k: &str| m.get("$").and_then(|a| a.get(k)).and_then(Value::as_str).map(String::from);
    let mut out: Vec<SeriesRef> = Vec::new();
    for (i, m) in list.iter().enumerate() {
        if attr(m, "name").as_deref() == Some("calibre:series") {
            let Some(name) = attr(m, "content").map(|c| c.trim().to_string()).filter(|c| !c.is_empty()) else { continue };
            let next = list.get(i + 1);
            let sequence = next
                .filter(|n| attr(n, "name").as_deref() == Some("calibre:series_index"))
                .and_then(|n| attr(n, "content"))
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty());
            out.push(SeriesRef { name, sequence });
        }
    }
    if out.len() == 1 && out[0].sequence.is_none() {
        if let Some(idx) = list.iter().find(|m| {
            attr(m, "name").as_deref() == Some("calibre:series_index") && attr(m, "content").is_some_and(|c| !c.trim().is_empty())
        }) {
            out[0].sequence = attr(idx, "content").map(|c| c.trim().to_string());
        }
    }
    let mut deduped: Vec<SeriesRef> = Vec::new();
    for s in out {
        if !deduped.iter().any(|d| d.name == s.name) {
            deduped.push(s);
        }
    }
    deduped
}

/// Applies an OPF's (or an EPUB's) details to a book: lists only when not empty, text only when set
/// (`OpfFileScanner.scanBookOpfFile`).
pub fn apply_opf(opf: &OpfMeta, book: &mut BookMeta) {
    for (key, value) in &opf.texts {
        if let Some(v) = value.as_ref().filter(|v| !v.is_empty()) {
            book.set(key, Value::String(v.clone()));
        }
    }
    if !opf.tags.is_empty() {
        book.tags = opf.tags.clone();
    }
    if !opf.genres.is_empty() {
        book.genres = opf.genres.clone();
    }
    if !opf.authors.is_empty() {
        book.authors = opf.authors.clone();
    }
    if !opf.narrators.is_empty() {
        book.narrators = opf.narrators.clone();
    }
    if !opf.series.is_empty() {
        book.series = opf.series.clone();
    }
}

/// What reading an ebook found (`EBookFileScanData`): its details and where its cover is inside it.
#[derive(Clone, Debug, Default)]
pub struct EbookScan {
    /// The ebook's path.
    pub path: String,
    /// "epub", "cbz" or "cbr".
    pub format: String,
    /// The details in the ebook's package.
    pub metadata: Option<OpfMeta>,
    /// The cover picture's path inside the ebook.
    pub cover_path: Option<String>,
}

/// Reads one file out of a zip, as text.
fn zip_text(path: &str, entry: &str) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let mut f = zip.by_name(entry).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Reads an EPUB's package for its details and cover (`parseEpubMetadata.parse`).
pub fn parse_epub(path: &str) -> Option<EbookScan> {
    let container = xml::to_json(&zip_text(path, "META-INF/container.xml")?)?;
    let package_path =
        container.pointer("/container/rootfiles/0/rootfile/0/$/full-path").and_then(Value::as_str).filter(|p| !p.is_empty())?.to_string();
    let package = xml::to_json(&zip_text(path, &package_path)?)?;
    let opf = opf_from_json(&package)?;
    let mut scan = EbookScan { path: path.into(), format: "epub".into(), metadata: Some(opf), cover_path: None };
    let mut meta = package.pointer("/package/metadata").cloned();
    if let Some(Value::Array(l)) = &meta {
        meta = l.first().cloned();
    }
    let cover_id = meta
        .as_ref()
        .and_then(|m| m.get("meta"))
        .and_then(Value::as_array)
        .and_then(|l| l.iter().find(|m| m.pointer("/$/name").and_then(Value::as_str) == Some("cover")))
        .and_then(|m| m.pointer("/$/content").and_then(Value::as_str).map(String::from));
    let items = package.pointer("/package/manifest/0/item").and_then(Value::as_array).cloned().unwrap_or_default();
    let attr = |i: &Value, k: &str| i.get("$").and_then(|a| a.get(k)).and_then(Value::as_str).map(String::from);
    let mut found = cover_id.and_then(|id| items.iter().find(|i| attr(i, "id").as_deref() == Some(id.as_str())).cloned());
    if found.is_none() {
        found = items.iter().find(|i| attr(i, "properties").is_some_and(|p| p.split(' ').any(|x| x == "cover-image"))).cloned();
    }
    if found.is_none() {
        found = items.iter().find(|i| attr(i, "media-type").is_some_and(|t| t.starts_with("image/"))).cloned();
    }
    if let Some(href) = found.and_then(|i| attr(&i, "href")).filter(|h| !h.is_empty()) {
        scan.cover_path = Some(super::files::join(&[&super::files::dirname(&package_path), &href]));
    }
    Some(scan)
}

/// Reads an ebook for its details (`parseEbookMetadata.parse`): EPUBs for now; comics' ComicInfo.xml comes with
/// the comic support.
pub fn parse_ebook(path: &str, format: &str) -> Option<EbookScan> {
    match format {
        "epub" => parse_epub(path),
        _ => None,
    }
}

/// Copies an ebook's cover picture out to `out` (`extractCoverImage`).
pub fn extract_ebook_cover(scan: &EbookScan, out: &str) -> bool {
    let Some(inner) = &scan.cover_path else { return false };
    let Ok(file) = std::fs::File::open(&scan.path) else { return false };
    let Ok(mut zip) = zip::ZipArchive::new(file) else { return false };
    let Ok(mut f) = zip.by_name(inner) else { return false };
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return false;
    }
    if let Some(dir) = std::path::Path::new(out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(out, buf).is_ok()
}

/// Details read from an NFO file (`parseNfoMetadata`), keys as the original names them.
pub fn parse_nfo(text: &str) -> Map<String, Value> {
    static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?):(.*)$").unwrap());
    static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?): (.*)$").unwrap());
    static DESC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*book description\s*$").unwrap());
    static RULE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*=+\s*$").unwrap());
    static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4}").unwrap());
    let split_list = |v: &str| -> Value { json!(Regex::new(r"\s*,\s*").unwrap().split(v).filter(|s| !s.is_empty()).collect::<Vec<_>>()) };
    let mut m = Map::new();
    let mut in_description = false;
    for line in text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if DESC.is_match(line) {
            in_description = true;
            continue;
        }
        if in_description {
            if RULE.is_match(line) {
                continue;
            }
            let d = m.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            m.insert("description".into(), json!(format!("{d}{line}\n")));
            continue;
        }
        let Some(c) = KEY.captures(line) else { continue };
        let key = c[1].to_lowercase().trim().to_string();
        let value = c[2].trim().to_string();
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "title" => match TITLE.captures(&value) {
                Some(t) => {
                    m.insert("title".into(), json!(t[1].trim()));
                    m.insert("subtitle".into(), json!(t[2].trim()));
                }
                None => {
                    m.insert("title".into(), json!(value));
                }
            },
            "author" => {
                m.insert("authors".into(), split_list(&value));
            }
            "narrator" | "read by" => {
                m.insert("narrators".into(), split_list(&value));
            }
            "series name" => {
                m.insert("series".into(), json!(value));
            }
            "genre" => {
                m.insert("genres".into(), split_list(&value));
            }
            "tags" => {
                m.insert("tags".into(), split_list(&value));
            }
            "copyright"
            | "audible.com release"
            | "audiobook copyright"
            | "book copyright"
            | "recording copyright"
            | "release date"
            | "date" => {
                if let Some(y) = YEAR.find_iter(&value).last() {
                    m.insert("publishedYear".into(), json!(y.as_str()));
                }
            }
            "position in series" => {
                m.insert("sequence".into(), json!(value));
            }
            "unabridged" => {
                m.insert("abridged".into(), json!(value.to_lowercase() != "yes"));
            }
            "abridged" => {
                m.insert("abridged".into(), json!(value.to_lowercase() != "no"));
            }
            "publisher" => {
                m.insert("publisher".into(), json!(value));
            }
            "asin" => {
                m.insert("asin".into(), json!(value));
            }
            "isbn" | "isbn-10" | "isbn-13" => {
                m.insert("isbn".into(), json!(value));
            }
            "language" | "lang" => {
                m.insert("language".into(), json!(value));
            }
            _ => {}
        }
    }
    if let Some(d) = m.get("description").and_then(Value::as_str).map(|d| d.trim().to_string()) {
        m.insert("description".into(), json!(d));
    }
    m
}

/// Strings of a JSON list.
fn str_list(v: &Value) -> Vec<String> {
    v.as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Applies an NFO's details to a book (`NfoFileScanner.scanBookNfoFile`).
pub fn apply_nfo(nfo: &Map<String, Value>, book: &mut BookMeta) {
    for (key, value) in nfo {
        match key.as_str() {
            "tags" | "genres" | "authors" | "narrators" => {
                let list = str_list(value);
                if !list.is_empty() {
                    match key.as_str() {
                        "tags" => book.tags = list,
                        "genres" => book.genres = list,
                        "authors" => book.authors = list,
                        _ => book.narrators = list,
                    }
                }
            }
            "series" => {
                if let Some(name) = value.as_str().filter(|s| !s.is_empty()) {
                    let sequence = nfo.get("sequence").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
                    book.series = vec![SeriesRef { name: name.to_string(), sequence }];
                }
            }
            "sequence" => {}
            _ => {
                if crate::abs::models::book::truthy(value) {
                    book.set(key, value.clone());
                }
            }
        }
    }
}

/// Details in a `metadata.json` (`abmetadataGenerator.parseJson` for books): each key checked for its type,
/// lists cleaned, series split into name and number, chapters checked. `None` when it isn't JSON.
pub fn parse_abs_json(text: &str) -> Option<Map<String, Value>> {
    let mut data: Map<String, Value> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(o)) => o,
        Ok(_) => Map::new(),
        Err(_) => return None,
    };
    if let Some(Value::Object(old)) = data.get("metadata").cloned() {
        for (k, v) in old {
            let key = match k.as_str() {
                "feedUrl" => "feedURL".to_string(),
                "imageUrl" => "imageURL".to_string(),
                "itunesPageUrl" => "itunesPageURL".to_string(),
                "type" => "podcastType".to_string(),
                _ => k,
            };
            data.insert(key, v);
        }
    }
    data.remove("metadata");
    let keys: &[(&str, &str)] = &[
        ("tags", "stringArray"),
        ("title", "string"),
        ("subtitle", "string"),
        ("authors", "stringArray"),
        ("narrators", "stringArray"),
        ("series", "stringArray"),
        ("genres", "stringArray"),
        ("publishedYear", "string"),
        ("publishedDate", "string"),
        ("publisher", "string"),
        ("description", "string"),
        ("isbn", "string"),
        ("asin", "string"),
        ("language", "string"),
        ("explicit", "boolean"),
        ("abridged", "boolean"),
    ];
    let mut out = Map::new();
    for (key, kind) in keys {
        let Some(v) = data.get(*key) else { continue };
        let checked = match *kind {
            "string" => match v {
                Value::Null => Some(Value::Null),
                Value::Number(n) => Some(Value::String(js_number_text(n))),
                Value::String(_) => Some(v.clone()),
                _ => None,
            },
            "boolean" => match v {
                Value::Null => Some(Value::Null),
                Value::Bool(_) => Some(v.clone()),
                Value::String(s) if s.eq_ignore_ascii_case("true") => Some(Value::Bool(true)),
                Value::String(s) if s.eq_ignore_ascii_case("false") => Some(Value::Bool(false)),
                _ => None,
            },
            _ => match v {
                Value::Null => Some(json!([])),
                Value::Array(l) => {
                    let mut seen: Vec<String> = Vec::new();
                    for t in l.iter().filter_map(Value::as_str).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
                        if !seen.contains(&t) {
                            seen.push(t);
                        }
                    }
                    Some(json!(seen))
                }
                _ => None,
            },
        };
        if let Some(c) = checked {
            out.insert(key.to_string(), c);
        }
    }
    if let Some(Value::Array(series)) = out.get("series").cloned() {
        if !series.is_empty() {
            let parsed: Vec<Value> = series
                .iter()
                .filter_map(Value::as_str)
                .filter_map(parse_series_string)
                .map(|s| json!({"name": s.name, "sequence": s.sequence}))
                .collect();
            out.insert("series".into(), json!(parsed));
        }
    }
    if let Some(ch) = data.get("chapters") {
        match ch {
            Value::Null => {
                out.insert("chapters".into(), json!([]));
            }
            Value::Array(list) => {
                if let Some(clean) = clean_chapters(list) {
                    out.insert("chapters".into(), Value::Array(clean));
                }
            }
            _ => {}
        }
    }
    Some(out)
}

/// A JavaScript number as `String(n)` writes it.
fn js_number_text(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) => crate::abs::library::util::js_num(f).to_string(),
        None => n.to_string(),
    }
}

/// Checks `metadata.json` chapters (`cleanChaptersArray`): every chapter needs a start, an end and a title, or none
/// are used.
fn clean_chapters(list: &[Value]) -> Option<Vec<Value>> {
    let num = |v: &Value| -> Option<Value> {
        match v {
            Value::Number(_) => Some(v.clone()),
            Value::Bool(b) => Some(json!(if *b { 1 } else { 0 })),
            Value::String(s) => crate::abs::library::js_number(s).map(|_| v.clone()),
            Value::Array(a) if a.is_empty() => Some(v.clone()),
            _ => None,
        }
    };
    let mut out = Vec::new();
    for (i, c) in list.iter().enumerate() {
        let start = num(c.get("start").unwrap_or(&Value::Null))?;
        let end = num(c.get("end").unwrap_or(&Value::Null))?;
        let title = c.get("title").and_then(Value::as_str).filter(|t| !t.is_empty())?;
        out.push(json!({"id": i, "start": start, "end": end, "title": title}));
    }
    Some(out)
}

/// Splits "Series Name #2" into name and number (`parseSeriesString.parse`).
pub fn parse_series_string(text: &str) -> Option<SeriesRef> {
    static SEQ: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" #([^#\s]+)$").unwrap());
    if text.is_empty() {
        return None;
    }
    match SEQ.captures(text) {
        Some(c) => Some(SeriesRef { name: text.replacen(c.get(0).unwrap().as_str(), "", 1), sequence: Some(c[1].to_string()) }),
        None => Some(SeriesRef { name: text.to_string(), sequence: None }),
    }
}

/// Applies `metadata.json` details to a book (`AbsMetadataFileScanner.scanBookMetadataFile`): missing and null
/// keys are skipped, and so are empty author, genre, tag and chapter lists.
pub fn apply_abs_json(abs: &Map<String, Value>, book: &mut BookMeta) {
    for (key, value) in abs {
        if value.is_null() {
            continue;
        }
        let empty = value.as_array().is_some_and(|l| l.is_empty());
        if empty && ["authors", "genres", "tags", "chapters"].contains(&key.as_str()) {
            continue;
        }
        match key.as_str() {
            "authors" => book.authors = str_list(value),
            "narrators" => book.narrators = str_list(value),
            "genres" => book.genres = str_list(value),
            "tags" => book.tags = str_list(value),
            "chapters" => book.chapters = value.as_array().cloned().unwrap_or_default(),
            "series" => {
                book.series = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| SeriesRef {
                        name: s["name"].as_str().unwrap_or("").to_string(),
                        sequence: s["sequence"].as_str().map(String::from),
                    })
                    .collect()
            }
            _ => book.set(key, value.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opf_like_the_original() {
        let xml = r##"<?xml version="1.0"?><package version="3.0" xmlns="http://www.idpf.org/2007/opf">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
<dc:title>All Systems Red</dc:title><dc:creator opf:role="aut" opf:file-as="Wells, Martha">Martha Wells</dc:creator>
<dc:creator>No Role</dc:creator><dc:creator id="n1">Kevin R. Free</dc:creator><meta refines="#n1" property="role">nrt</meta>
<dc:identifier opf:scheme="ISBN">9780765397522</dc:identifier><dc:date>2017-05-02</dc:date><dc:language>en</dc:language>
<dc:subject>Science Fiction</dc:subject><dc:subject>Science Fiction</dc:subject><dc:description>&lt;p&gt;A &lt;b&gt;murderbot&lt;/b&gt;&lt;/p&gt;</dc:description>
<meta name="calibre:series" content="The Murderbot Diaries"/><meta name="calibre:series_index" content="1"/></metadata></package>"##;
        let opf = opf_from_json(&xml::to_json(xml).unwrap()).unwrap();
        assert_eq!(opf.authors, vec!["Martha Wells".to_string()]);
        assert_eq!(opf.narrators, vec!["Kevin R. Free".to_string()]);
        assert_eq!(opf.genres, vec!["Science Fiction".to_string()]);
        assert_eq!(opf.series, vec![SeriesRef { name: "The Murderbot Diaries".into(), sequence: Some("1".into()) }]);
        let texts: std::collections::HashMap<_, _> = opf.texts.iter().cloned().collect();
        assert_eq!(texts["publishedYear"].as_deref(), Some("2017"));
        assert_eq!(texts["isbn"].as_deref(), Some("9780765397522"));
        assert_eq!(texts["description"].as_deref(), Some("A murderbot"));
    }

    #[test]
    fn nfo_like_the_original() {
        let m = parse_nfo("Title: Book: The Sub\r\nAuthor: A One, B Two\nCopyright: 1999 and 2001\nUnabridged: Yes\nBook Description\n=====\nLine one\nLine two\n");
        assert_eq!(m["title"], "Book");
        assert_eq!(m["subtitle"], "The Sub");
        assert_eq!(m["authors"], json!(["A One", "B Two"]));
        assert_eq!(m["publishedYear"], "2001");
        assert_eq!(m["abridged"], false);
        assert_eq!(m["description"], "Line one\nLine two");
    }

    #[test]
    fn metadata_json_like_the_original() {
        let m = parse_abs_json(r#"{"title": 12, "authors": [" A ", "A", 3, ""], "series": ["S #2", "T"], "explicit": "TRUE", "abridged": 1, "chapters": [{"start": 0, "end": "5", "title": "One"}]}"#).unwrap();
        assert_eq!(m["title"], "12");
        assert_eq!(m["authors"], json!(["A"]));
        assert_eq!(m["series"], json!([{"name": "S", "sequence": "2"}, {"name": "T", "sequence": null}]));
        assert_eq!(m["explicit"], true);
        assert!(!m.contains_key("abridged"));
        assert_eq!(m["chapters"], json!([{"id": 0, "start": 0, "end": "5", "title": "One"}]));
        assert!(parse_abs_json("nope").is_none());
    }
}
