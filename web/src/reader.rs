//! The EPUB reader (`#/read?id=<edition id>`).
//!
//! The book is fetched once (from this browser's offline copy when there is one, [`crate::offline_media`]) and
//! opened here: [`epub`] reads its zip, package, spine and table of contents. Each chapter is drawn in a sandboxed
//! frame that runs none of the book's scripts, with its pictures and styles from the book itself, in pages (CSS
//! columns) coloured by the app's theme. The reader has the table of contents, text size, bookmarks with notes,
//! page turns by buttons, keys and taps on the page's edges, and keeps the place on the server
//! (`{href, progression, total_progression, cfi}`, docs/TRACKS.md) so another device opens at the same spot.

use crate::offline_media::{self, now_s};
use kit_leptos::api;
use kit_leptos::rt;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;

/// Reading an EPUB: its zip, the package (OPF), the spine (the chapters in reading order), the table of contents
/// and where things are. Plain Rust, so it is tested without a browser.
pub mod epub {
    use std::collections::HashMap;
    use std::io::{Cursor, Read};

    /// One chapter file of the spine.
    #[derive(Debug, Clone, PartialEq)]
    pub struct SpineItem {
        /// its manifest id (the CFI names it)
        pub idref: String,
        /// its path in the zip
        pub href: String,
        /// its size in bytes, uncompressed: how much of the book it is
        pub size: u64,
    }

    /// One entry of the table of contents.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TocEntry {
        /// its words
        pub title: String,
        /// the chapter file's path in the zip
        pub href: String,
        /// the place in the chapter (`#id` without `#`), "" for its start
        pub fragment: String,
        /// how deep it is, from 0
        pub depth: usize,
    }

    /// An opened EPUB.
    pub struct Book {
        /// the title from the package
        pub title: String,
        /// the chapters in reading order
        pub spine: Vec<SpineItem>,
        /// the table of contents (from the EPUB 3 nav, else the EPUB 2 NCX)
        pub toc: Vec<TocEntry>,
        /// the type of each file of the manifest, by path
        pub types: HashMap<String, String>,
        /// the zip
        zip: zip::ZipArchive<Cursor<Vec<u8>>>,
    }

    /// Why a file is not an EPUB this reader can open, in plain words.
    fn broken(what: &str) -> String {
        format!("This ebook can't be opened: {what}. It may be damaged; search for another copy from the book's page.")
    }

    /// `rel` (an address inside the book, maybe with `../`, `%20` and `#place`) read from the file `base`: the path
    /// in the zip and the place after `#`.
    pub fn resolve(base: &str, rel: &str) -> (String, String) {
        let (path, frag) = rel.split_once('#').unwrap_or((rel, ""));
        let path = decode(path);
        let mut parts: Vec<&str> = if path.starts_with('/') { vec![] } else { base.split('/').collect() };
        parts.pop(); // the base's own file name
        for p in path.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        (parts.join("/"), frag.to_string())
    }

    /// `%XX` in an address read as the bytes they stand for.
    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// The local name of an XML element (`opf:item` → `item`).
    fn is(n: &roxmltree::Node, name: &str) -> bool {
        n.is_element() && n.tag_name().name() == name
    }

    impl Book {
        /// Opens the EPUB in `bytes`.
        pub fn open(bytes: Vec<u8>) -> Result<Book, String> {
            let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| broken("it is not a zip file"))?;
            let mut book = Book { title: String::new(), spine: vec![], toc: vec![], types: HashMap::new(), zip };
            let container = book.text("META-INF/container.xml").ok_or_else(|| broken("it has no META-INF/container.xml"))?;
            let doc = roxmltree::Document::parse(&container).map_err(|_| broken("its container.xml can't be read"))?;
            let opf = doc
                .descendants()
                .find(|n| is(n, "rootfile"))
                .and_then(|n| n.attribute("full-path"))
                .ok_or_else(|| broken("its container.xml names no package"))?
                .to_string();
            let text = book.text(&opf).ok_or_else(|| broken("its package file is missing"))?;
            let doc = roxmltree::Document::parse(&text).map_err(|_| broken("its package file can't be read"))?;
            book.title = doc.descendants().find(|n| is(n, "title")).and_then(|n| n.text()).unwrap_or("").trim().to_string();
            let mut items: HashMap<String, (String, String, String)> = HashMap::new();
            for n in doc.descendants().filter(|n| is(n, "item")) {
                let (Some(id), Some(href)) = (n.attribute("id"), n.attribute("href")) else { continue };
                let path = resolve(&opf, href).0;
                let mime = n.attribute("media-type").unwrap_or("").to_string();
                book.types.insert(path.clone(), mime.clone());
                items.insert(id.to_string(), (path, mime, n.attribute("properties").unwrap_or("").to_string()));
            }
            let spine_el = doc.descendants().find(|n| is(n, "spine")).ok_or_else(|| broken("its package has no spine"))?;
            for n in spine_el.children().filter(|n| is(n, "itemref")) {
                let Some(idref) = n.attribute("idref") else { continue };
                let Some((href, _, _)) = items.get(idref) else { continue };
                let size = book.zip.by_name(href).map(|f| f.size()).unwrap_or(0);
                book.spine.push(SpineItem { idref: idref.to_string(), href: href.clone(), size: size.max(1) });
            }
            if book.spine.is_empty() {
                return Err(broken("it has no chapters"));
            }
            let nav = items.values().find(|(_, _, props)| props.split_whitespace().any(|p| p == "nav")).map(|(p, _, _)| p.clone());
            let ncx = spine_el
                .attribute("toc")
                .and_then(|id| items.get(id))
                .map(|(p, _, _)| p.clone())
                .or_else(|| items.values().find(|(_, m, _)| m == "application/x-dtbncx+xml").map(|(p, _, _)| p.clone()));
            if let Some(nav) = nav {
                book.toc = book.text(&nav).map(|t| nav_toc(&nav, &t)).unwrap_or_default();
            }
            if book.toc.is_empty() {
                if let Some(ncx) = ncx {
                    book.toc = book.text(&ncx).map(|t| ncx_toc(&ncx, &t)).unwrap_or_default();
                }
            }
            Ok(book)
        }

        /// The bytes of the file `path` of the book.
        pub fn bytes(&mut self, path: &str) -> Option<Vec<u8>> {
            let mut f = self.zip.by_name(path).ok()?;
            let mut out = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut out).ok()?;
            Some(out)
        }

        /// The file `path` of the book as text.
        pub fn text(&mut self, path: &str) -> Option<String> {
            self.bytes(path).map(|b| String::from_utf8_lossy(&b).into_owned())
        }

        /// Whether the book has the file `path`.
        pub fn has(&self, path: &str) -> bool {
            self.zip.index_for_name(path).is_some()
        }

        /// The type of the file `path` (from the manifest, else from its name).
        pub fn mime(&self, path: &str) -> String {
            if let Some(m) = self.types.get(path).filter(|m| !m.is_empty()) {
                return m.clone();
            }
            let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            match ext.as_str() {
                "css" => "text/css",
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "gif" => "image/gif",
                "svg" => "image/svg+xml",
                "webp" => "image/webp",
                "xhtml" | "xht" => "application/xhtml+xml",
                "html" | "htm" => "text/html",
                "otf" => "font/otf",
                "ttf" => "font/ttf",
                "woff" => "font/woff",
                "woff2" => "font/woff2",
                _ => "application/octet-stream",
            }
            .to_string()
        }

        /// The spine index of the chapter file `href`.
        pub fn index_of(&self, href: &str) -> Option<usize> {
            self.spine.iter().position(|s| s.href == href)
        }

        /// How far through the whole book a place is: the chapter `index` and `progression` (0–1) through it,
        /// with each chapter counting by its size.
        pub fn total_progression(&self, index: usize, progression: f64) -> f64 {
            let all: u64 = self.spine.iter().map(|s| s.size).sum();
            let before: u64 = self.spine.iter().take(index).map(|s| s.size).sum();
            let here = self.spine.get(index).map(|s| s.size).unwrap_or(0) as f64 * progression.clamp(0.0, 1.0);
            if all == 0 {
                0.0
            } else {
                ((before as f64 + here) / all as f64).clamp(0.0, 1.0)
            }
        }

        /// The chapter and progression through it for a place `total` (0–1) through the whole book.
        pub fn at_total(&self, total: f64) -> (usize, f64) {
            let all: u64 = self.spine.iter().map(|s| s.size).sum();
            let target = total.clamp(0.0, 1.0) * all as f64;
            let mut before = 0.0;
            for (i, s) in self.spine.iter().enumerate() {
                let size = s.size as f64;
                if target < before + size || i + 1 == self.spine.len() {
                    return (i, ((target - before) / size).clamp(0.0, 1.0));
                }
                before += size;
            }
            (0, 0.0)
        }

        /// The title of the table of contents entry a chapter falls under ("" when none).
        pub fn chapter_title(&self, index: usize) -> String {
            let mut best: Option<(usize, &str)> = None;
            for e in &self.toc {
                let Some(i) = self.index_of(&e.href) else { continue };
                // the latest chapter at or before this one; its first entry (its start, not a place inside it)
                if i <= index && best.is_none_or(|(b, _)| i > b) {
                    best = Some((i, &e.title));
                }
            }
            best.map(|(_, t)| t.to_string()).unwrap_or_default()
        }
    }

    /// The table of contents of an EPUB 3 nav document at `path`.
    pub fn nav_toc(path: &str, text: &str) -> Vec<TocEntry> {
        let Ok(doc) = roxmltree::Document::parse(text) else { return vec![] };
        let toc = doc
            .descendants()
            .filter(|n| is(n, "nav"))
            .find(|n| n.attributes().any(|a| a.name() == "type" && a.value().split_whitespace().any(|v| v == "toc")))
            .or_else(|| doc.descendants().find(|n| is(n, "nav")));
        let Some(toc) = toc else { return vec![] };
        let mut out = vec![];
        if let Some(ol) = toc.children().find(|n| is(n, "ol")) {
            walk_ol(path, ol, 0, &mut out);
        }
        out
    }

    /// The entries of a nav `<ol>` and the lists inside it.
    fn walk_ol(path: &str, ol: roxmltree::Node, depth: usize, out: &mut Vec<TocEntry>) {
        for li in ol.children().filter(|n| is(n, "li")) {
            if let Some(a) = li.children().find(|n| is(n, "a")) {
                let title: String = a.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect::<String>();
                let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                let (href, fragment) = resolve(path, a.attribute("href").unwrap_or(""));
                out.push(TocEntry { title, href, fragment, depth });
            }
            if let Some(sub) = li.children().find(|n| is(n, "ol")) {
                walk_ol(path, sub, depth + 1, out);
            }
        }
    }

    /// The table of contents of an EPUB 2 NCX at `path`.
    pub fn ncx_toc(path: &str, text: &str) -> Vec<TocEntry> {
        let Ok(doc) = roxmltree::Document::parse(text) else { return vec![] };
        let Some(map) = doc.descendants().find(|n| is(n, "navMap")) else { return vec![] };
        let mut out = vec![];
        walk_points(path, map, 0, &mut out);
        out
    }

    /// The entries of NCX navPoints and the ones inside them.
    fn walk_points(path: &str, parent: roxmltree::Node, depth: usize, out: &mut Vec<TocEntry>) {
        for p in parent.children().filter(|n| is(n, "navPoint")) {
            let title = p
                .children()
                .find(|n| is(n, "navLabel"))
                .and_then(|l| l.descendants().find(|n| is(n, "text")))
                .and_then(|t| t.text())
                .unwrap_or("")
                .trim()
                .to_string();
            let src = p.children().find(|n| is(n, "content")).and_then(|c| c.attribute("src")).unwrap_or("");
            let (href, fragment) = resolve(path, src);
            out.push(TocEntry { title, href, fragment, depth });
            walk_points(path, p, depth + 1, out);
        }
    }

    /// An EPUB CFI for the element at `steps` (child element positions from the root element, from 0) of the
    /// spine item `index` (`idref`): `epubcfi(/6/4[ch1]!/4/2/6)`.
    pub fn cfi(index: usize, idref: &str, steps: &[usize]) -> String {
        let path: String = steps.iter().map(|s| format!("/{}", (s + 1) * 2)).collect();
        format!("epubcfi(/6/{}[{}]!{})", (index + 1) * 2, idref, path)
    }

    /// The spine item and element steps of a CFI made by [`cfi`] (assertions and character offsets left out):
    /// `None` when it isn't one.
    pub fn parse_cfi(cfi: &str) -> Option<(usize, Vec<usize>)> {
        let inner = cfi.strip_prefix("epubcfi(")?.strip_suffix(')')?;
        let (spine, path) = inner.split_once('!')?;
        let step = |t: &str| -> Option<usize> {
            let t = t.split(['[', ':', '~', '@']).next()?;
            t.parse::<usize>().ok()
        };
        let spine_steps: Vec<usize> = spine.split('/').filter(|t| !t.is_empty()).filter_map(step).collect();
        let index = spine_steps.get(1)?.checked_div(2)?.checked_sub(1)?;
        let steps: Vec<usize> =
            path.split('/').filter(|t| !t.is_empty()).filter_map(step).filter(|s| s % 2 == 0 && *s > 0).map(|s| s / 2 - 1).collect();
        Some((index, steps))
    }
}

/// The `localStorage` key of the text size (percent, one for every book).
const SIZE_KEY: &str = "folio.reader.size";

/// The text size last chosen in this browser, 100% when none.
fn saved_size() -> i32 {
    rt::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SIZE_KEY).ok().flatten())
        .and_then(|t| t.parse::<i32>().ok())
        .filter(|x| (70..=220).contains(x))
        .unwrap_or(100)
}

/// Keeps the text size for next time.
fn keep_size(x: i32) {
    if let Some(s) = rt::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(SIZE_KEY, &x.to_string());
    }
}

/// The rules a chapter's page runs under: only the book's own files (`blob:`, `data:`), no script of any kind.
pub const CSP: &str = "default-src 'none'; script-src 'none'; style-src blob: data: 'unsafe-inline'; img-src blob: data:; \
                       font-src blob: data:; media-src blob: data:";

/// The space at a page's sides, in pixels.
const PAD: i32 = 36;

/// The page rules put into each chapter: one page per screen (CSS columns), the app's colours, the chosen size.
pub fn page_css(bg: &str, text: &str, link: &str, size: i32) -> String {
    format!(
        "html{{height:100%;overflow:hidden;font-size:{size}%;background:{bg}!important;}}\
         body{{margin:0!important;box-sizing:border-box;height:100%;padding:{top}px {PAD}px!important;\
         column-width:calc(100vw - {two}px);column-gap:{two}px;column-fill:auto;\
         background:{bg}!important;color:{text}!important;line-height:1.6;overflow-wrap:break-word;}}\
         body *{{color:inherit;background-color:transparent!important;}}\
         a,a *{{color:{link}!important;}}\
         img,svg,video{{max-width:100%;max-height:calc(100vh - {img}px);object-fit:contain;break-inside:avoid;}}\
         table{{max-width:100%;}} pre{{white-space:pre-wrap;}}",
        top = PAD,
        two = PAD * 2,
        img = PAD * 2 + 8,
    )
}

/// A theme colour of the app (`--bg`), as the page shows it now.
fn token(name: &str) -> String {
    rt::window()
        .zip(rt::document().and_then(|d| d.document_element()))
        .and_then(|(w, root)| w.get_computed_style(&root).ok().flatten())
        .and_then(|st| st.get_property_value(name).ok())
        .map(|v| v.trim().to_string())
        .unwrap_or_default()
}

/// The book open in the reader, with the addresses made for its pictures and styles.
struct Open {
    /// the book
    book: epub::Book,
    /// `blob:` addresses of the book's files, by path
    urls: HashMap<String, String>,
}

impl Open {
    /// A `blob:` address for the file `path` of the book (made once); stylesheets have their own addresses
    /// rewritten first.
    fn url(&mut self, path: &str) -> Option<String> {
        if let Some(u) = self.urls.get(path) {
            return Some(u.clone());
        }
        let mime = self.book.mime(path);
        let bytes = if mime == "text/css" {
            let css = self.book.text(path)?;
            self.css(path, &css).into_bytes()
        } else {
            self.book.bytes(path)?
        };
        let u = blob_url(&bytes, &mime)?;
        self.urls.insert(path.to_string(), u.clone());
        Some(u)
    }

    /// The stylesheet `css` of the file `path` with its `url(...)`s pointing at the book's files.
    fn css(&mut self, path: &str, css: &str) -> String {
        let mut out = String::with_capacity(css.len());
        let mut rest = css;
        while let Some(i) = rest.find("url(") {
            out.push_str(&rest[..i + 4]);
            rest = &rest[i + 4..];
            let Some(end) = rest.find(')') else { break };
            let raw = rest[..end].trim().trim_matches(['"', '\'']);
            let (target, _) = epub::resolve(path, raw);
            match (raw.contains(':'), self.book.has(&target)) {
                (false, true) if target != path => {
                    let u = self.url(&target).unwrap_or_default();
                    out.push_str(&format!("\"{u}\""));
                }
                _ => out.push_str(&rest[..end]),
            }
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }

    /// The chapter `index` ready for the frame: its pictures, styles and fonts from the book, its links marked,
    /// the page rules `style` added and the book's scripts taken out; a `blob:` address.
    fn chapter(&mut self, index: usize, style: &str) -> Result<String, String> {
        let href = self.book.spine.get(index).map(|s| s.href.clone()).ok_or("That chapter isn't in this book.")?;
        let text = self.book.text(&href).ok_or("This chapter is missing from the ebook file.")?;
        let parser = web_sys::DomParser::new().map_err(|_| "This browser can't read the chapter.")?;
        let mut xml = true;
        let mut doc =
            parser.parse_from_string(&text, web_sys::SupportedType::ApplicationXhtmlXml).map_err(|_| "The chapter can't be read.")?;
        if doc.get_elements_by_tag_name("parsererror").length() > 0 {
            xml = false;
            doc = parser.parse_from_string(&text, web_sys::SupportedType::TextHtml).map_err(|_| "The chapter can't be read.")?;
        }
        // none of the book's own code runs: scripts, frames and plugins go, and so do inline handlers (onclick=)
        for tag in ["script", "iframe", "frame", "object", "embed"] {
            for el in to_vec(doc.get_elements_by_tag_name(tag)) {
                el.remove();
            }
        }
        let all = to_vec(doc.get_elements_by_tag_name("*"));
        for el in all {
            let names = el.get_attribute_names();
            for n in names.iter().filter_map(|n| n.as_string()) {
                if n.to_ascii_lowercase().starts_with("on") {
                    el.remove_attribute(&n).ok();
                }
            }
            if el.local_name().eq_ignore_ascii_case("a") && el.has_attribute("href") {
                let v = el.get_attribute("href").unwrap_or_default();
                if v.trim_start().to_ascii_lowercase().starts_with("javascript:") {
                    el.remove_attribute("href").ok();
                }
            }
            for attr in ["src", "href", "xlink:href", "poster"] {
                let Some(v) = el.get_attribute(attr) else { continue };
                if v.starts_with('#') && el.tag_name().eq_ignore_ascii_case("a") {
                    el.set_attribute("data-folio-href", &format!("{href}{v}")).ok();
                    continue;
                }
                if v.contains(':') && !v.starts_with("../") {
                    // an address outside the book: links open in a new tab, nothing else is fetched
                    if el.tag_name().eq_ignore_ascii_case("a") {
                        el.set_attribute("data-folio-out", &v).ok();
                    } else {
                        el.remove_attribute(attr).ok();
                    }
                    continue;
                }
                let (target, frag) = epub::resolve(&href, &v);
                let tag = el.local_name();
                if tag.eq_ignore_ascii_case("a") {
                    let to = if frag.is_empty() { target } else { format!("{target}#{frag}") };
                    el.set_attribute("data-folio-href", &to).ok();
                } else if let Some(u) = self.url(&target) {
                    el.set_attribute(attr, &u).ok();
                }
            }
            if let Some(st) = el.get_attribute("style").filter(|s| s.contains("url(")) {
                let fixed = self.css(&href, &st);
                el.set_attribute("style", &fixed).ok();
            }
        }
        for st in to_vec(doc.get_elements_by_tag_name("style")) {
            let fixed = self.css(&href, &st.text_content().unwrap_or_default());
            st.set_text_content(Some(&fixed));
        }
        let head = doc.get_elements_by_tag_name("head").item(0);
        let xhtml = Some("http://www.w3.org/1999/xhtml");
        let laid_out = "The chapter can't be laid out.";
        // the page may load only the book's own files (blob:) and run no script at all
        let csp = doc.create_element_ns(xhtml, "meta").map_err(|_| laid_out)?;
        csp.set_attribute("http-equiv", "Content-Security-Policy").ok();
        csp.set_attribute("content", CSP).ok();
        let ours = doc.create_element_ns(xhtml, "style").map_err(|_| laid_out)?;
        ours.set_text_content(Some(style));
        let parent = match head {
            Some(h) => h,
            None => doc.document_element().ok_or(laid_out)?,
        };
        parent.insert_before(&csp, parent.first_child().as_ref()).map_err(|_| laid_out)?;
        parent.append_child(&ours).map_err(|_| laid_out)?;
        let (markup, mime) = if xml {
            let ser = web_sys::XmlSerializer::new().map_err(|_| "The chapter can't be laid out.")?;
            (ser.serialize_to_string(&doc).map_err(|_| "The chapter can't be laid out.")?, "application/xhtml+xml")
        } else {
            (doc.document_element().map(|e| e.outer_html()).unwrap_or_default(), "text/html")
        };
        blob_url(markup.as_bytes(), mime).ok_or_else(|| "The chapter can't be shown.".to_string())
    }

    /// Lets go of every address made for the book.
    fn free(&mut self) {
        for (_, u) in self.urls.drain() {
            let _ = web_sys::Url::revoke_object_url(&u);
        }
    }
}

/// A list of elements as a `Vec`.
fn to_vec(list: web_sys::HtmlCollection) -> Vec<web_sys::Element> {
    (0..list.length()).filter_map(|i| list.item(i)).collect()
}

/// A `blob:` address for `bytes` of type `mime`.
fn blob_url(bytes: &[u8], mime: &str) -> Option<String> {
    let arr = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::of1(&arr);
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// Where the reader is: the chapter, the page in it and how many pages it has.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    /// the spine index
    pub index: usize,
    /// the page in the chapter, from 0
    pub page: i32,
    /// the chapter's pages
    pub pages: i32,
}

impl Spot {
    /// How far through the chapter the top of this page is, 0–1.
    pub fn progression(self) -> f64 {
        if self.pages <= 1 {
            0.0
        } else {
            (self.page as f64 / self.pages as f64).clamp(0.0, 1.0)
        }
    }
}

/// The page of a chapter of `pages` pages that a place `progression` (0–1) through it is on.
pub fn page_for(progression: f64, pages: i32) -> i32 {
    ((progression.clamp(0.0, 1.0) * pages as f64).floor() as i32).clamp(0, (pages - 1).max(0))
}

/// What to show once a chapter is drawn: a page, a place through it, an element (CFI steps) or an `id`.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// this page (`-1`: the last)
    Page(i32),
    /// this far through the chapter
    Progress(f64),
    /// the element at these steps
    Steps(Vec<usize>),
    /// the element with this id
    Id(String),
}

/// What the reader loaded.
#[derive(Clone)]
struct Loaded {
    /// the edition as the library answers it
    edition: Value,
    /// the opened book (shared with the page)
    open: Rc<std::cell::RefCell<Open>>,
    /// the person's place, the newer of the server's and one waiting in this browser
    place: Option<Value>,
    /// the person's bookmarks
    bookmarks: Vec<Value>,
    /// the edition's files as the server lists them
    files: Vec<Value>,
}

/// Loads the edition, its EPUB, the person's place and bookmarks.
async fn load(id: i64) -> Result<Loaded, String> {
    let edition = offline_media::json(&format!("/library/editions/{id}")).await?;
    if edition.get("format").and_then(Value::as_str) != Some("ebook") {
        return Err("This edition is an audiobook. Open it with Listen.".into());
    }
    let files = offline_media::json(&format!("/library/editions/{id}/files")).await?;
    let files = files.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let kind =
        |k: &str| files.iter().find(|f| f.get("kind").and_then(Value::as_str) == Some(k)).and_then(|f| f.get("id")).and_then(Value::as_i64);
    let Some(file) = kind("epub") else {
        if let Some(pdf) = kind("pdf") {
            return Err(format!("PDF:{pdf}"));
        }
        return Err("This ebook has no EPUB file in the library yet. Once it is downloaded and checked, it opens here.".into());
    };
    let bytes = offline_media::file_bytes(file).await?;
    let book = epub::Book::open(bytes)?;
    let server = offline_media::json(&format!("/progress/{id}")).await.ok();
    let local = offline_media::pending(id);
    let place = match (server, local) {
        (Some(s), Some(l)) => Some(if offline_media::newer(&l, Some(&s)) { l } else { s }),
        (s, l) => s.or(l),
    };
    let bookmarks = offline_media::json(&format!("/editions/{id}/bookmarks"))
        .await
        .ok()
        .and_then(|v| v.get("bookmarks").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    Ok(Loaded { edition, open: Rc::new(std::cell::RefCell::new(Open { book, urls: HashMap::new() })), place, bookmarks, files })
}

/// `#/read?id=<edition id>`: the reader.
#[component]
pub fn Reader(id: i64) -> impl IntoView {
    offline_media::watch();
    let data = LocalResource::new(move || load(id));
    move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span><p class="muted">"Opening the book…"</p></div> }.into_any(),
        Some(Err(e)) if e.starts_with("PDF:") => {
            let pdf = e.trim_start_matches("PDF:").to_string();
            view! {
                <div class="empty">
                    <p class="muted">"This ebook is a PDF. Your browser shows it best."</p>
                    <a class="btn primary" href=format!("/api/files/{pdf}") target="_blank" rel="noopener">"Open the PDF"</a>
                </div>
            }
            .into_any()
        }
        Some(Err(e)) => view! {
            <div class="error-box" id="reader-error">{e}</div>
            <a class="btn mt" href="javascript:history.back()">"Back"</a>
        }
        .into_any(),
        Some(Ok(l)) => view! { <Pages id=id loaded=l/> }.into_any(),
    }
}

/// The steps (child element positions, from 0) from the root element down to `el`.
fn steps_of(el: &web_sys::Element) -> Vec<usize> {
    let mut steps = vec![];
    let mut cur = el.clone();
    while let Some(parent) = cur.parent_element() {
        let mut i = 0;
        let mut sib = cur.previous_element_sibling();
        while let Some(s) = sib {
            i += 1;
            sib = s.previous_element_sibling();
        }
        steps.push(i);
        cur = parent;
    }
    steps.reverse();
    steps
}

/// The element at `steps` from the root element of `doc`, as deep as the steps still lead.
fn element_at(doc: &web_sys::Document, steps: &[usize]) -> Option<web_sys::Element> {
    let mut cur = doc.document_element()?;
    for s in steps {
        match cur.children().item(*s as u32) {
            Some(c) => cur = c,
            None => break,
        }
    }
    Some(cur)
}

/// The reader itself, once the book is open.
#[component]
fn Pages(id: i64, loaded: Loaded) -> impl IntoView {
    let Loaded { edition, open, place, bookmarks, files } = loaded;
    let edition_for_download = edition.clone();
    let open = StoredValue::new_local(open);
    let frame = NodeRef::<leptos::html::Iframe>::new();
    let (spot, set_spot) = signal(Spot { index: 0, page: 0, pages: 1 });
    let (size, set_size) = signal(saved_size());
    let (panel, set_panel) = signal("");
    let (marks, set_marks) = signal(bookmarks);
    let (busy, set_busy) = signal(true);
    let target = StoredValue::new(None::<Target>);
    let save_tick = StoredValue::new(0u32);
    let known_at = StoredValue::new(place.as_ref().and_then(|p| p.get("updated_at")).and_then(Value::as_f64).unwrap_or(0.0));
    let listeners = StoredValue::new_local(Vec::<rt::Listener>::new());

    // (timers may fire after the reader is closed: then there is no frame)
    let doc = move || frame.try_get_untracked().flatten().and_then(|f| f.content_document());
    let width = move || frame.try_get_untracked().flatten().map(|f| f.client_width()).unwrap_or(1).max(1);

    // the place now, as the server keeps it
    let locator = move || -> Value {
        let sp = spot.get_untracked();
        open.with_value(|o| {
            let o = o.borrow();
            let Some(item) = o.book.spine.get(sp.index) else { return Value::Null };
            let first = doc().and_then(|d| first_visible(&d, width()));
            let mut loc = json!({
                "href": item.href,
                "progression": sp.progression(),
                "total_progression": o.book.total_progression(sp.index, sp.progression()),
            });
            if let Some(el) = first {
                loc["cfi"] = json!(epub::cfi(sp.index, &item.idref, &steps_of(&el)));
            }
            loc
        })
    };
    // the place now, as a device sends it; kept as the latest place too
    let place_now = move || -> Option<Value> {
        let loc = locator();
        if loc.is_null() {
            return None;
        }
        let total = loc["total_progression"].as_f64().unwrap_or(0.0);
        let sp = spot.get_untracked();
        let last = open.with_value(|o| o.borrow().book.spine.len()) == sp.index + 1 && sp.page + 1 >= sp.pages;
        let place = json!({
            "locator": loc,
            "fraction": total,
            "finished": last,
            "device": offline_media::DEVICE,
            "updated_at": now_s(),
        });
        offline_media::remember(id, &place);
        Some(place)
    };
    let save_now = move || {
        let Some(place) = place_now() else { return };
        known_at.set_value(place["updated_at"].as_f64().unwrap_or(0.0));
        rt::spawn(async move {
            let _ = offline_media::save_progress(id, place).await;
        });
    };
    // the place goes a moment after the last page turn
    let save_soon = move || {
        let tick = save_tick.get_value().wrapping_add(1);
        save_tick.set_value(tick);
        rt::after(1500, move || {
            if save_tick.try_get_value() == Some(tick) {
                save_now();
            }
        });
    };

    // shows page `p` of the chapter drawn now
    let show_page = move |p: i32| {
        let Some(d) = doc() else { return };
        let w = width();
        let Some(root) = d.document_element() else { return };
        let pages = ((root.scroll_width() as f64) / w as f64).ceil().max(1.0) as i32;
        let p = if p < 0 { pages - 1 } else { p.min(pages - 1) };
        root.set_scroll_left(p * w);
        if let Some(b) = d.body() {
            b.set_scroll_left(p * w);
        }
        set_spot.update(|s| {
            s.page = p;
            s.pages = pages;
        });
        place_now();
    };

    // draws chapter `index` and then goes to `t`
    let draw = move |index: usize, t: Target| {
        let Some(f) = frame.get_untracked() else { return };
        let style = page_css(&token("--bg"), &token("--text"), &token("--accent-2"), size.get_untracked());
        let url = open.with_value(|o| o.borrow_mut().chapter(index, &style));
        match url {
            Ok(u) => {
                set_busy.set(true);
                target.set_value(Some(t));
                set_spot.update(|s| s.index = index);
                let old = f.src();
                f.set_src(&u);
                if old.starts_with("blob:") {
                    let _ = web_sys::Url::revoke_object_url(&old);
                }
            }
            Err(e) => toast(&e, "err"),
        }
    };

    let next = move || {
        let sp = spot.get_untracked();
        if sp.page + 1 < sp.pages {
            show_page(sp.page + 1);
            save_soon();
        } else if sp.index + 1 < open.with_value(|o| o.borrow().book.spine.len()) {
            draw(sp.index + 1, Target::Page(0));
            save_soon();
        }
    };
    let prev = move || {
        let sp = spot.get_untracked();
        if sp.page > 0 {
            show_page(sp.page - 1);
            save_soon();
        } else if sp.index > 0 {
            draw(sp.index - 1, Target::Page(-1));
            save_soon();
        }
    };
    // goes to a place inside the book (`path#id`)
    let go_href = move |to: &str| {
        let (path, frag) = to.split_once('#').map(|(p, f)| (p.to_string(), f.to_string())).unwrap_or((to.to_string(), String::new()));
        let Some(i) = open.with_value(|o| o.borrow().book.index_of(&path)) else { return };
        let t = if frag.is_empty() { Target::Page(0) } else { Target::Id(frag) };
        if i == spot.get_untracked().index && !busy.get_untracked() {
            go_target(doc(), t, width(), show_page);
        } else {
            draw(i, t);
        }
        save_soon();
    };
    // goes to a kept place (a locator)
    let go_locator = move |loc: &Value| {
        let href = loc.get("href").and_then(Value::as_str).unwrap_or("");
        let (index, t) = open.with_value(|o| {
            let o = o.borrow();
            if let Some((i, steps)) = loc.get("cfi").and_then(Value::as_str).and_then(epub::parse_cfi) {
                if o.book.spine.get(i).is_some_and(|s| s.href == href || href.is_empty()) && !steps.is_empty() {
                    return (i, Target::Steps(steps));
                }
            }
            match o.book.index_of(href) {
                Some(i) => (i, Target::Progress(loc.get("progression").and_then(Value::as_f64).unwrap_or(0.0))),
                None => {
                    let (i, p) = o.book.at_total(loc.get("total_progression").and_then(Value::as_f64).unwrap_or(0.0));
                    (i, Target::Progress(p))
                }
            }
        });
        draw(index, t);
    };

    // a chapter finished loading in the frame: listen to it and go to the place asked for
    let on_load = move |_| {
        let Some(d) = doc() else { return };
        let w = width();
        let t = target.get_value().unwrap_or(Target::Page(0));
        target.set_value(None);
        // the pages settle once fonts and pictures are in
        go_target(Some(d.clone()), t.clone(), w, show_page);
        rt::after(250, move || go_target(doc(), t, width(), show_page));
        set_busy.set(false);
        let mut ls = vec![];
        ls.push(rt::listen(&d, "click", move |e| {
            if let Some(a) = frame_target(e).and_then(|t| t.closest("a").ok().flatten()) {
                e.prevent_default();
                if let Some(to) = a.get_attribute("data-folio-href") {
                    go_href(&to);
                } else if let Some(out) = a.get_attribute("data-folio-out").filter(|u| u.starts_with("http")) {
                    if let Some(w) = rt::window() {
                        let _ = w.open_with_url_and_target_and_features(&out, "_blank", "noopener");
                    }
                }
                return;
            }
            let m = e.unchecked_ref::<web_sys::MouseEvent>();
            let w = width();
            if m.client_x() < w * 3 / 10 {
                prev();
            } else if m.client_x() > w * 7 / 10 {
                next();
            }
        }));
        ls.push(rt::listen(&d, "keydown", move |e| key(e, next, prev)));
        listeners.set_value(ls);
    };

    // keys on the app's page too
    let page_keys = rt::document().map(|d| rt::listen(&d, "keydown", move |e| key(e, next, prev)));
    let resize = rt::window().map(|w| {
        rt::listen(&w, "resize", move |_| {
            let steps = doc().and_then(|d| first_visible(&d, width())).map(|el| steps_of(&el));
            if let Some(s) = steps {
                go_target(doc(), Target::Steps(s), width(), show_page);
            }
        })
    });
    let hide = rt::document().map(|d| {
        rt::listen(&d, "visibilitychange", move |_| {
            if rt::document().map(|d| d.hidden()).unwrap_or(false) {
                save_now();
                return;
            }
            // back on the page: another device may have read on
            rt::spawn(async move {
                if let Ok(p) = api::get(&format!("/progress/{id}")).await {
                    let at = p.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0);
                    if at > known_at.get_value() + 1.0 {
                        known_at.set_value(at);
                        if let Some(loc) = p.get("locator").filter(|l| l.is_object()) {
                            go_locator(loc);
                            toast("Moved to where you were reading on another device.", "info");
                        }
                    }
                }
            });
        })
    });
    let page_listeners = StoredValue::new_local((page_keys, resize, hide));
    // leaving: only what is not reactive any more may be used here (parts of the page may be gone already)
    on_cleanup(move || {
        offline_media::send_latest(id);
        listeners.try_update_value(|l| l.clear());
        page_listeners.try_update_value(|l| *l = (None, None, None));
        open.try_with_value(|o| o.borrow_mut().free());
        if let Some(f) = frame.try_get_untracked().flatten() {
            let src = f.src();
            if src.starts_with("blob:") {
                let _ = web_sys::Url::revoke_object_url(&src);
            }
        }
    });

    // the first chapter, at the place kept
    let start = place.clone();
    Effect::new(move |done: Option<bool>| {
        if done == Some(true) || frame.get().is_none() {
            return done.unwrap_or(false);
        }
        match start
            .as_ref()
            .and_then(|p| p.get("locator"))
            .filter(|l| l.is_object() && start.as_ref().and_then(|p| p.get("finished")).and_then(Value::as_bool) != Some(true))
        {
            Some(loc) => go_locator(loc),
            None => draw(0, Target::Page(0)),
        }
        true
    });

    let resize_text = move |d: i32| {
        let n = (size.get_untracked() + d).clamp(70, 220);
        set_size.set(n);
        keep_size(n);
        let steps = doc().and_then(|dd| first_visible(&dd, width())).map(|el| steps_of(&el));
        draw(spot.get_untracked().index, steps.map(Target::Steps).unwrap_or(Target::Page(0)));
    };

    let add_mark = move |_| {
        let loc = locator();
        let snippet = doc().and_then(|d| first_visible(&d, width())).and_then(|e| e.text_content()).unwrap_or_default();
        let snippet: String = snippet.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect();
        let note =
            if snippet.is_empty() { open.with_value(|o| o.borrow().book.chapter_title(spot.get_untracked().index)) } else { snippet };
        rt::spawn(async move {
            match api::post(&format!("/editions/{id}/bookmarks"), json!({ "locator": loc, "note": note, "created_at": now_s() })).await {
                Ok(b) => {
                    set_marks.update(|m| {
                        m.push(b);
                        let far = |b: &Value| b["locator"]["total_progression"].as_f64().unwrap_or(0.0);
                        m.sort_by(|a, b| far(a).total_cmp(&far(b)));
                    });
                    toast("Bookmark added.", "ok");
                }
                Err(e) => toast(&format!("The bookmark wasn't kept: {}", e.message), "err"),
            }
        });
    };

    let title = edition.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let work_id = edition.get("work_id").and_then(Value::as_i64).unwrap_or(0);
    let toc = open.with_value(|o| o.borrow().book.toc.clone());
    let chapter = move || open.with_value(|o| o.borrow().book.chapter_title(spot.get().index));
    let percent = move || {
        let sp = spot.get();
        let t = open.with_value(|o| o.borrow().book.total_progression(sp.index, sp.progression()));
        format!("{}%", (t * 100.0).round() as i64)
    };
    let toggle_panel = move |p: &'static str| set_panel.update(|x| *x = if *x == p { "" } else { p });

    view! {
        <div class="reader" id="reader">
            <div class="reader-bar">
                <a class="btn ghost sm" href=format!("#/work?id={work_id}")>"Back"</a>
                <div class="grow" style="min-width:0">
                    <div class="ellipsis reader-title" id="reader-title">{title}</div>
                    <div class="small muted ellipsis" id="reader-chapter">{chapter}</div>
                </div>
                <button class="btn ghost sm" class:on=move || panel.get() == "toc" on:click=move |_| toggle_panel("toc")>"Contents"</button>
                <button class="btn ghost sm" class:on=move || panel.get() == "marks" on:click=move |_| toggle_panel("marks")>"Bookmarks"</button>
                <button class="btn ghost sm" title="Smaller text" aria-label="Smaller text" on:click=move |_| resize_text(-10)>"A−"</button>
                <button class="btn ghost sm" title="Larger text" aria-label="Larger text" on:click=move |_| resize_text(10)>"A+"</button>
                <button class="btn sm" id="reader-bookmark" on:click=add_mark>"Add bookmark"</button>
                <offline_media::DownloadButton edition=edition_for_download files=files/>
            </div>
            <div class="reader-body">
                <div class="reader-page">
                    <iframe node_ref=frame class="reader-frame" id="reader-frame" title="The book's page" sandbox="allow-same-origin allow-scripts" on:load=on_load></iframe>
                    {move || busy.get().then(|| view! { <div class="reader-busy"><span class="spinner"></span></div> })}
                </div>
                {move || match panel.get() {
                    "toc" => view! {
                        <aside class="card reader-panel" id="reader-toc">
                            <h2>"Contents"</h2>
                            <div class="list">
                                {toc.iter().map(|e| {
                                    let to = if e.fragment.is_empty() { e.href.clone() } else { format!("{}#{}", e.href, e.fragment) };
                                    let pad = format!("padding-left:{}px", 10 + e.depth * 14);
                                    view! { <button class="list-item player-item" style=pad on:click=move |_| { go_href(&to); set_panel.set(""); }>{e.title.clone()}</button> }
                                }).collect_view()}
                            </div>
                        </aside>
                    }.into_any(),
                    "marks" => view! {
                        <aside class="card reader-panel" id="reader-marks">
                            <h2>"Bookmarks"</h2>
                            {move || {
                                let list = marks.get();
                                if list.is_empty() {
                                    view! { <p class="small muted">"No bookmarks yet. Add one with Add bookmark while you read."</p> }.into_any()
                                } else {
                                    view! { <div class="list">{list.into_iter().map(|b| mark_row(b, go_locator, set_marks)).collect_view()}</div> }.into_any()
                                }
                            }}
                        </aside>
                    }.into_any(),
                    _ => ().into_any(),
                }}
            </div>
            <div class="reader-foot">
                <button class="btn ghost" id="reader-prev" on:click=move |_| prev()>"Previous page"</button>
                <span class="small muted" id="reader-where">{move || {
                    let sp = spot.get();
                    format!("Page {} of {} in this chapter · {}", sp.page + 1, sp.pages, percent())
                }}</span>
                <button class="btn ghost" id="reader-next" on:click=move |_| next()>"Next page"</button>
            </div>
        </div>
    }
}

/// Goes to `t` in the chapter drawn in `doc`, pages `w` pixels wide.
fn go_target(doc: Option<web_sys::Document>, t: Target, w: i32, show_page: impl Fn(i32)) {
    let Some(d) = doc else { return };
    let pages = d.document_element().map(|r| ((r.scroll_width() as f64) / w.max(1) as f64).ceil().max(1.0) as i32).unwrap_or(1);
    let page_of = |el: &web_sys::Element| {
        let x = el.get_bounding_client_rect().left() + d.document_element().map(|r| r.scroll_left()).unwrap_or(0) as f64;
        ((x.max(0.0)) / w.max(1) as f64).floor() as i32
    };
    let p = match t {
        Target::Page(p) => p,
        Target::Progress(f) => page_for(f, pages),
        Target::Steps(s) => element_at(&d, &s).map(|el| page_of(&el)).unwrap_or(0),
        Target::Id(id) => d.get_element_by_id(&id).map(|el| page_of(&el)).unwrap_or(0),
    };
    show_page(p);
}

/// The first element whose text shows on the page now (the place a bookmark and the CFI point at).
fn first_visible(doc: &web_sys::Document, w: i32) -> Option<web_sys::Element> {
    let body = doc.body()?;
    let all = body.get_elements_by_tag_name("*");
    for i in 0..all.length() {
        let Some(el) = all.item(i) else { continue };
        if el.child_element_count() > 0 && !el.text_content().unwrap_or_default().trim().is_empty() && !has_own_text(&el) {
            continue;
        }
        let r = el.get_bounding_client_rect();
        if r.width() > 0.0 && r.right() > 0.0 && r.left() < w as f64 && r.height() > 0.0 {
            return Some(el);
        }
    }
    None
}

/// Whether `el` holds text of its own (not only inside its children).
fn has_own_text(el: &web_sys::Element) -> bool {
    let kids = el.child_nodes();
    (0..kids.length())
        .filter_map(|i| kids.item(i))
        .any(|n| n.node_type() == web_sys::Node::TEXT_NODE && !n.text_content().unwrap_or_default().trim().is_empty())
}

/// The element an event happened on, also when it happened inside the frame (whose elements `instanceof` doesn't
/// know, being of the frame's own window).
fn frame_target(e: &web_sys::Event) -> Option<web_sys::Element> {
    let t = e.target()?;
    let is_element = js_sys::Reflect::get(&t, &"nodeType".into()).ok().and_then(|v| v.as_f64()) == Some(1.0);
    is_element.then(|| t.unchecked_into::<web_sys::Element>())
}

/// Page turns by keys: the arrows, Page Up and Down, space.
fn key(e: &web_sys::Event, next: impl Fn(), prev: impl Fn()) {
    // the page's events come from the frame's own window, which `instanceof` doesn't know: ask for the key itself
    if !js_sys::Reflect::has(e, &"key".into()).unwrap_or(false) {
        return;
    }
    let k = e.unchecked_ref::<web_sys::KeyboardEvent>();
    let typing = frame_target(e).is_some_and(|t| matches!(t.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"));
    if typing || k.ctrl_key() || k.meta_key() || k.alt_key() {
        return;
    }
    match k.key().as_str() {
        "ArrowRight" | "PageDown" | " " => {
            e.prevent_default();
            next();
        }
        "ArrowLeft" | "PageUp" => {
            e.prevent_default();
            prev();
        }
        _ => {}
    }
}

/// One bookmark: how far into the book and its note; a click goes there; the note can be changed; it can be deleted.
fn mark_row(b: Value, go: impl Fn(&Value) + Copy + 'static, set_marks: WriteSignal<Vec<Value>>) -> impl IntoView {
    let bid = b.get("id").and_then(Value::as_i64).unwrap_or(0);
    let loc = b.get("locator").cloned().unwrap_or(Value::Null);
    let pct = format!("{}%", (loc.get("total_progression").and_then(Value::as_f64).unwrap_or(0.0) * 100.0).round() as i64);
    let note = b.get("note").and_then(Value::as_str).unwrap_or("").to_string();
    let edit = move |e: leptos::ev::Event| {
        let note = event_target_value(&e);
        rt::spawn(async move {
            if let Err(e) = api::patch(&format!("/bookmarks/{bid}"), json!({ "note": note })).await {
                toast(&format!("The note wasn't kept: {}", e.message), "err");
            }
        });
    };
    let remove = move |_| {
        rt::spawn(async move {
            match api::del(&format!("/bookmarks/{bid}")).await {
                Ok(_) => set_marks.update(|m| m.retain(|x| x.get("id").and_then(Value::as_i64) != Some(bid))),
                Err(e) => toast(&format!("The bookmark wasn't deleted: {}", e.message), "err"),
            }
        });
    };
    view! {
        <div class="list-item player-item">
            <button class="btn ghost sm" title="Go to this bookmark" on:click=move |_| go(&loc)>{pct}</button>
            <input class="input grow" value=note placeholder="Note" aria-label="Note" on:change=edit/>
            <button class="btn ghost icon sm" title="Delete this bookmark" aria-label="Delete this bookmark" on:click=remove inner_html=kit_leptos::icons::icon("trash")></button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::epub::*;
    use super::*;
    use std::io::Write;

    /// A small EPUB 3 with a nav, two chapters, a picture and a stylesheet.
    pub fn sample() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            let mut add = |name: &str, body: &str, opts| {
                z.start_file(name, opts).unwrap();
                z.write_all(body.as_bytes()).unwrap();
            };
            add("mimetype", "application/epub+zip", stored);
            add(
                "META-INF/container.xml",
                r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
                deflated,
            );
            add(
                "OEBPS/content.opf",
                r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>The Test Book</dc:title></metadata><manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="c1" href="text/one.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="text/two%20b.xhtml" media-type="application/xhtml+xml"/><item id="css" href="style.css" media-type="text/css"/><item id="img" href="images/a.png" media-type="image/png"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine></package>"#,
                deflated,
            );
            add(
                "OEBPS/nav.xhtml",
                r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="text/one.xhtml">One</a><ol><li><a href="text/one.xhtml#mid">  The middle </a></li></ol></li><li><a href="text/two%20b.xhtml">Two</a></li></ol></nav></body></html>"#,
                deflated,
            );
            add("OEBPS/text/one.xhtml", &format!("<html><body><p>{}</p><p id=\"mid\">mid</p></body></html>", "x".repeat(300)), deflated);
            add("OEBPS/text/two b.xhtml", "<html><body><p>two</p></body></html>", deflated);
            add("OEBPS/style.css", "body{background:url(images/a.png)}", deflated);
            add("OEBPS/images/a.png", "PNG", deflated);
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn an_epub_opens_with_its_spine_and_contents() {
        let mut b = Book::open(sample()).unwrap();
        assert_eq!(b.title, "The Test Book");
        assert_eq!(b.spine.iter().map(|s| s.href.as_str()).collect::<Vec<_>>(), vec!["OEBPS/text/one.xhtml", "OEBPS/text/two b.xhtml"]);
        let toc: Vec<(&str, &str, &str, usize)> =
            b.toc.iter().map(|e| (e.title.as_str(), e.href.as_str(), e.fragment.as_str(), e.depth)).collect();
        assert_eq!(
            toc,
            vec![
                ("One", "OEBPS/text/one.xhtml", "", 0),
                ("The middle", "OEBPS/text/one.xhtml", "mid", 1),
                ("Two", "OEBPS/text/two b.xhtml", "", 0)
            ]
        );
        assert_eq!(b.text("OEBPS/text/two b.xhtml").as_deref(), Some("<html><body><p>two</p></body></html>"));
        assert!(b.has("OEBPS/images/a.png"));
        assert_eq!(b.mime("OEBPS/style.css"), "text/css");
        assert_eq!(b.mime("OEBPS/fonts/x.woff2"), "font/woff2");
        assert_eq!(b.chapter_title(1), "Two");
        assert_eq!(b.chapter_title(0), "One");
    }

    #[test]
    fn broken_files_say_so_in_plain_words() {
        let e = Book::open(b"not a zip".to_vec()).err().unwrap();
        assert!(e.starts_with("This ebook can't be opened: it is not a zip file."), "{e}");
    }

    #[test]
    fn addresses_inside_the_book_resolve() {
        assert_eq!(resolve("OEBPS/text/one.xhtml", "../images/a.png"), ("OEBPS/images/a.png".into(), String::new()));
        assert_eq!(resolve("OEBPS/text/one.xhtml", "two%20b.xhtml#p3"), ("OEBPS/text/two b.xhtml".into(), "p3".into()));
        assert_eq!(resolve("content.opf", "./a/b.xhtml"), ("a/b.xhtml".into(), String::new()));
        assert_eq!(resolve("OEBPS/x.opf", "/top.css"), ("top.css".into(), String::new()));
        assert_eq!(resolve("a/b.xhtml", "100%"), ("a/100%".into(), String::new()));
    }

    #[test]
    fn places_count_by_chapter_size() {
        let b = Book::open(sample()).unwrap();
        let t = b.total_progression(1, 0.0);
        assert!(t > 0.5 && t < 1.0, "{t}");
        assert_eq!(b.total_progression(0, 0.0), 0.0);
        assert_eq!(b.total_progression(1, 1.0), 1.0);
        let (i, p) = b.at_total(t);
        assert_eq!(i, 1);
        assert!(p < 0.01);
        assert_eq!(b.at_total(0.0), (0, 0.0));
    }

    #[test]
    fn cfis_go_there_and_back() {
        let c = cfi(1, "c2", &[1, 0, 2]);
        assert_eq!(c, "epubcfi(/6/4[c2]!/4/2/6)");
        assert_eq!(parse_cfi(&c), Some((1, vec![1, 0, 2])));
        assert_eq!(parse_cfi("epubcfi(/6/14[chap05ref]!/4[body01]/10/2:3)"), Some((6, vec![1, 4, 0])));
        assert_eq!(parse_cfi("page 4"), None);
    }

    #[test]
    fn pages_and_progress() {
        assert_eq!(page_for(0.0, 10), 0);
        assert_eq!(page_for(0.55, 10), 5);
        assert_eq!(page_for(1.0, 10), 9);
        assert_eq!(page_for(0.5, 0), 0);
        assert_eq!(Spot { index: 0, page: 3, pages: 4 }.progression(), 0.75);
        assert_eq!(Spot { index: 0, page: 0, pages: 1 }.progression(), 0.0);
        let css = page_css("#000", "#fff", "#ccc", 120);
        assert!(css.contains("font-size:120%") && css.contains("column-width:calc(100vw - 72px)"));
    }
}
