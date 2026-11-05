//! Reading what other apps need from inside an EPUB: the reading order (the spine), so a place written by one
//! reader ("the 12th part, 40% in") can be found by another. Only the package files are read, never the
//! book's text.

use std::io::Read;
use std::path::Path;

/// The spine items' paths inside the EPUB, in reading order (`OEBPS/Text/chapter01.xhtml`).
pub fn spine(path: &Path) -> Option<Vec<String>> {
    let file = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let container = read(&mut zip, "META-INF/container.xml")?;
    let opf_path = tags(&container, "rootfile").into_iter().find_map(|t| attr(&t, "full-path"))?;
    let opf = read(&mut zip, &opf_path)?;
    let base = match opf_path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/"),
        None => String::new(),
    };
    let manifest: std::collections::HashMap<String, String> =
        tags(&opf, "item").into_iter().filter_map(|t| Some((attr(&t, "id")?, attr(&t, "href")?))).collect();
    let order: Vec<String> = tags(&opf, "itemref")
        .into_iter()
        .filter_map(|t| attr(&t, "idref"))
        .filter_map(|id| manifest.get(&id).map(|h| format!("{base}{}", percent_decode(h))))
        .collect();
    (!order.is_empty()).then_some(order)
}

/// One file inside the zip as text (at most 4 MB).
fn read(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.take(4 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

/// The opening tags named `name` (with or without a namespace prefix such as `opf:`), each as its text inside
/// `<` and `>`.
pub fn tags(xml: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[..end];
        let tname = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let local = tname.rsplit(':').next().unwrap_or(tname);
        if local == name {
            out.push(tag.to_string());
        }
        rest = &rest[end..];
    }
    out
}

/// The value of attribute `name` in a tag's text, with the XML entities read.
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().last();
        let after = &rest[i + name.len()..];
        rest = after;
        if !matches!(before, Some(c) if c.is_whitespace()) {
            continue;
        }
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let v = &after[1..];
        let end = v.find(quote)?;
        let v = &v[..end];
        return Some(v.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'"));
    }
    None
}

/// `%20` and the like turned back into the characters.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            let hex = |c: u8| (c as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(b[i + 1]) * 16 + hex(b[i + 2]));
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where `href` (as a reader wrote it: maybe relative, maybe with `#part`) is in `spine`, from 0.
pub fn spine_index(spine: &[String], href: &str) -> Option<usize> {
    let href = percent_decode(href.split('#').next().unwrap_or(href)).trim_start_matches('/').to_string();
    if href.is_empty() {
        return None;
    }
    spine
        .iter()
        .position(|s| s == &href)
        .or_else(|| spine.iter().position(|s| s.ends_with(&format!("/{href}")) || href.ends_with(&format!("/{s}"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A small real EPUB at `path`: three chapters in `OEBPS/`, the package file with `opf:` prefixes.
    pub fn write_epub(path: &Path) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("mimetype", o).unwrap();
        z.write_all(b"application/epub+zip").unwrap();
        z.start_file("META-INF/container.xml", o).unwrap();
        z.write_all(br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
        z.start_file("OEBPS/content.opf", o).unwrap();
        z.write_all(
            br#"<opf:package><opf:manifest><opf:item id="c2" href="Text/two.xhtml" media-type="application/xhtml+xml"/>
            <opf:item href="Text/one%20a.xhtml" id="c1" media-type="application/xhtml+xml"/><opf:item id="c3" href="three.xhtml"/></opf:manifest>
            <opf:spine><opf:itemref idref="c1"/><opf:itemref idref="c2"/><opf:itemref idref="c3"/></opf:spine></opf:package>"#,
        )
        .unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn the_spine_is_read_in_order() {
        let p = std::env::temp_dir().join(format!("folio-epub-spine-{}.epub", std::process::id()));
        write_epub(&p);
        let s = spine(&p).unwrap();
        assert_eq!(s, ["OEBPS/Text/one a.xhtml", "OEBPS/Text/two.xhtml", "OEBPS/three.xhtml"]);
        assert_eq!(spine_index(&s, "Text/two.xhtml#p3"), Some(1));
        assert_eq!(spine_index(&s, "OEBPS/Text/one%20a.xhtml"), Some(0));
        assert_eq!(spine_index(&s, "four.xhtml"), None);
        assert_eq!(spine(&std::env::temp_dir()), None);
    }

    #[test]
    fn attributes_are_found_by_whole_name() {
        assert_eq!(attr(r#"item xid="no" id='yes' href="a&amp;b""#, "id").as_deref(), Some("yes"));
        assert_eq!(attr(r#"item id = "x""#, "href"), None);
        assert_eq!(attr(r#"item href="a&amp;b""#, "href").as_deref(), Some("a&b"));
    }
}
