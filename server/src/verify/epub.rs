//! Ebook checks: an EPUB is opened as the ZIP it is, its OPF package metadata (ISBN, title, authors, language)
//! is read, encryption is looked for, and its text is counted, then all of it is compared with the edition.

use super::Check;
use crate::library::{ids, text};
use crate::metadata::EditionMeta;
use crate::wanted::store::{language_name, Profile};
use std::io::Read;
use std::path::Path;

/// The ebook file types Folio takes, by extension (only EPUB can be checked).
pub const EXTENSIONS: &[&str] = &["epub", "kepub", "azw3", "azw", "mobi", "pdf"];

/// Encryption that is only font obfuscation, which EPUB allows without DRM.
const FONT_OBFUSCATION: &[&str] = &["http://www.idpf.org/2008/embedding", "http://ns.adobe.com/pdf/enc#RC"];

/// Words in a printed page, to compare a book's length with its page count.
pub const WORDS_PER_PAGE: u64 = 250;

/// Fewer words than this is not a whole book.
pub const FEWEST_WORDS: u64 = 5_000;

/// What an EPUB says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EbookFacts {
    /// whether it opens as an EPUB with a package document
    pub valid: bool,
    /// why it isn't valid, in words
    pub problem: String,
    /// whether its content is encrypted (DRM); font obfuscation doesn't count
    pub drm: bool,
    /// the ISBN-13 in its identifiers, "" when it gives none
    pub isbn13: String,
    /// its title
    pub title: String,
    /// its authors (`dc:creator`)
    pub authors: Vec<String>,
    /// its language, two letters
    pub language: String,
    /// the words in its reading order
    pub words: u64,
}

/// The local name of an XML node (`dc:title` → `title`).
fn local<'a>(n: &roxmltree::Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

/// The text of an XML node, trimmed.
fn text_of(n: &roxmltree::Node) -> String {
    n.descendants().filter(|d| d.is_text()).filter_map(|d| d.text()).collect::<String>().trim().to_string()
}

/// The words of an XHTML document: tags, scripts and styles left out.
pub fn count_words(xhtml: &str) -> u64 {
    let body = xhtml.find("<body").map(|i| &xhtml[i..]).unwrap_or(xhtml);
    let mut words = 0u64;
    let mut in_tag = false;
    let mut in_word = false;
    let mut skip: Option<&str> = None;
    let lower = body.to_ascii_lowercase();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = skip {
            match lower[i..].find(end) {
                Some(j) => {
                    i += j + end.len();
                    skip = None;
                }
                None => break,
            }
            continue;
        }
        let c = bytes[i];
        if c == b'<' {
            if lower[i..].starts_with("<script") {
                skip = Some("</script>");
            } else if lower[i..].starts_with("<style") {
                skip = Some("</style>");
            }
            in_tag = true;
            if in_word {
                words += 1;
                in_word = false;
            }
        } else if c == b'>' {
            in_tag = false;
        } else if !in_tag {
            let space = c.is_ascii_whitespace();
            if space && in_word {
                words += 1;
                in_word = false;
            } else if !space && c != b'&' {
                in_word = true;
            }
        }
        i += 1;
    }
    words + in_word as u64
}

/// Opens `path` as an EPUB and reads what it says. A file that isn't one says why in `problem`.
pub fn inspect(path: &Path) -> EbookFacts {
    match read(path) {
        Ok(f) => f,
        Err(problem) => EbookFacts { problem, ..Default::default() },
    }
}

/// The member `name` of the archive as text.
fn member(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = z.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

/// [`inspect`]'s work; `Err` with the reason when it isn't an EPUB.
fn read(path: &Path) -> Result<EbookFacts, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{} can't be opened: {e}.", path.display()))?;
    let mut z = zip::ZipArchive::new(file).map_err(|_| "It isn't an EPUB: the file isn't a ZIP archive.".to_string())?;
    let container = member(&mut z, "META-INF/container.xml").ok_or("It isn't an EPUB: META-INF/container.xml is missing.")?;
    let doc = roxmltree::Document::parse(&container).map_err(|e| format!("It isn't a valid EPUB: container.xml can't be read ({e})."))?;
    let opf_path = doc
        .descendants()
        .find(|n| local(n) == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or("It isn't a valid EPUB: container.xml names no package document.")?
        .to_string();
    let opf = member(&mut z, &opf_path).ok_or_else(|| format!("It isn't a valid EPUB: the package document {opf_path} is missing."))?;
    let pkg = roxmltree::Document::parse(&opf).map_err(|e| format!("It isn't a valid EPUB: {opf_path} can't be read ({e})."))?;

    let mut f = EbookFacts { valid: true, ..Default::default() };
    // DRM: encryption.xml that encrypts more than fonts, or Adobe's rights.xml
    if let Some(enc) = member(&mut z, "META-INF/encryption.xml") {
        if let Ok(d) = roxmltree::Document::parse(&enc) {
            f.drm = d
                .descendants()
                .filter(|n| local(n) == "EncryptionMethod")
                .filter_map(|n| n.attribute("Algorithm"))
                .any(|a| !FONT_OBFUSCATION.contains(&a));
        }
    }
    if z.by_name("META-INF/rights.xml").is_ok() {
        f.drm = true;
    }

    let meta = pkg.descendants().find(|n| local(n) == "metadata");
    if let Some(meta) = meta {
        for n in meta.children().filter(|n| n.is_element()) {
            let t = text_of(&n);
            match local(&n) {
                "title" if f.title.is_empty() => f.title = t,
                "creator" if !t.is_empty() => f.authors.push(t),
                "language" if f.language.is_empty() => f.language = text::language(&t),
                "identifier" if f.isbn13.is_empty() => {
                    let raw = t.trim_start_matches("urn:isbn:").trim_start_matches("isbn:");
                    if let Some(i) = ids::isbn13(raw) {
                        f.isbn13 = i;
                    }
                }
                _ => {}
            }
        }
    }

    // the words in reading order: the spine's items, found in the manifest, relative to the package document
    let base = opf_path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let manifest: std::collections::HashMap<&str, &str> =
        pkg.descendants().filter(|n| local(n) == "item").filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?))).collect();
    let spine: Vec<String> = pkg
        .descendants()
        .filter(|n| local(n) == "itemref")
        .filter_map(|n| n.attribute("idref"))
        .filter_map(|id| manifest.get(id))
        .map(|h| h.to_string())
        .collect();
    for href in spine {
        let href = href.split('#').next().unwrap_or("").replace("%20", " ");
        if let Some(x) = member(&mut z, &format!("{base}{href}")) {
            f.words += count_words(&x);
        }
    }
    Ok(f)
}

/// Whether one of `names` is in `hay` with all its words.
fn names_in(hay: &str, names: &[String]) -> bool {
    let h: Vec<String> = text::fold(hay).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
    names.iter().any(|n| {
        let w: Vec<String> = text::fold(n).split(' ').filter(|w| w.len() > 1).map(str::to_string).collect();
        !w.is_empty() && w.iter().all(|x| h.contains(x))
    })
}

/// Every check of an ebook download (`path`) against the edition `m` and its preferences `pr`.
pub fn checks(m: &EditionMeta, pr: &Profile, path: &str, f: &EbookFacts) -> Vec<Check> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext != "epub" && ext != "kepub" {
        return vec![Check::new("File", 100, "EPUB", ext.to_uppercase())
            .unknown(format!("Folio can check only EPUB files, and this one is {}. Look at it yourself.", ext.to_uppercase()))];
    }
    let mut out = vec![Check::new("Valid EPUB", 0, "a valid EPUB", if f.valid { "valid" } else { "not valid" }).hard()];
    out[0] = if f.valid { out[0].clone().pass("It is a valid EPUB.") } else { out[0].clone().fail(f.problem.clone()) };
    if !f.valid {
        return out;
    }
    let c = Check::new("No DRM", 0, "no DRM", if f.drm { "DRM" } else { "no DRM" }).hard();
    out.push(if f.drm {
        c.fail("It is locked with DRM, so it can't be read in Folio or on your devices.")
    } else {
        c.pass("It has no DRM.")
    });

    let c = Check::new("ISBN", 30, m.isbn13.clone(), f.isbn13.clone());
    out.push(if f.isbn13.is_empty() || m.isbn13.is_empty() {
        c.unknown("The book doesn't give its ISBN.")
    } else if f.isbn13 == m.isbn13 {
        c.pass("The ISBN matches.")
    } else {
        c.fail(format!("The book says ISBN {}; the edition is {}. It may be another edition of the same book.", f.isbn13, m.isbn13))
    });

    let short = m.title.split(':').next().unwrap_or(&m.title).to_string();
    let c = Check::new("Title", 25, m.title.clone(), f.title.clone());
    out.push(if f.title.is_empty() {
        c.unknown("The book doesn't give its title.")
    } else if names_in(&f.title, std::slice::from_ref(&short)) {
        c.pass("The title matches.")
    } else {
        c.fail(format!("The book is titled \"{}\", not \"{}\".", f.title, m.title))
    });

    let want: Vec<String> = m.authors.iter().map(|a| a.name.clone()).collect();
    let c = Check::new("Author", 20, want.join(", "), f.authors.join(", "));
    out.push(if f.authors.is_empty() || want.is_empty() {
        c.unknown("The book doesn't say who wrote it.")
    } else if names_in(&f.authors.join(" "), &want) {
        c.pass(format!("By {}.", f.authors.join(", ")))
    } else {
        c.fail(format!("The book says it is by {}, not {}.", f.authors.join(", "), want.join(", ")))
    });

    let wanted_lang = if m.language.is_empty() { pr.languages.first().cloned().unwrap_or_default() } else { m.language.clone() };
    let c = Check::new("Language", 10, language_name(&wanted_lang), language_name(&f.language)).hard();
    out.push(if f.language.is_empty() || wanted_lang.is_empty() {
        c.unknown("The book doesn't say its language.")
    } else if f.language == wanted_lang {
        c.pass(format!("In {}.", language_name(&f.language)))
    } else {
        c.fail(format!("It is in {}; the edition is in {}.", language_name(&f.language), language_name(&wanted_lang)))
    });

    let pages = m.pages.unwrap_or(0) as u64;
    let c = Check::new("Length", 15, if pages > 0 { format!("{pages} pages") } else { String::new() }, format!("{} words", f.words));
    out.push(if f.words < FEWEST_WORDS {
        c.fail(format!("It has only {} words: a sample or an excerpt, not the whole book.", f.words)).hard()
    } else if pages == 0 {
        if f.words >= 20_000 {
            c.pass(format!("{} words, the length of a book.", f.words))
        } else {
            c.unknown("The edition doesn't give its pages, and the book is short.")
        }
    } else {
        let ratio = f.words as f64 / (pages * WORDS_PER_PAGE) as f64;
        if (0.4..=2.5).contains(&ratio) {
            c.pass(format!("{} words fits {pages} pages.", f.words))
        } else {
            c.fail(format!("{} words doesn't fit the edition's {pages} pages (about {} words).", f.words, pages * WORDS_PER_PAGE))
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_leave_out_tags_scripts_and_styles() {
        let x = "<html><head><style>p { x: y }</style></head><body><p>One two</p><script>var a = 1;</script><p>three&nbsp;four</p></body></html>";
        assert_eq!(count_words(x), 3);
        assert_eq!(count_words("<body>Hello <b>big</b> world</body>"), 3);
    }
}
