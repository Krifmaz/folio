//! Rewriting an EPUB's metadata to the edition's: title, authors, language, ISBN and series in its OPF, and
//! its cover when it has none. Every other member of the archive is copied across byte for byte, in order (so
//! `mimetype` stays first and stored).

use crate::metadata::EditionMeta;
use kit_server_core::{err, Result};
use std::io::{Read, Write};
use std::path::Path;

/// The Dublin Core namespace.
const DC: &str = "http://purl.org/dc/elements/1.1/";

/// Text safe inside XML.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The OPF `opf` with the edition's metadata; `cover_href` (relative to the OPF) is added as the cover when given.
pub fn rewrite_opf(opf: &str, m: &EditionMeta, cover_href: Option<&str>) -> std::result::Result<String, String> {
    let doc = roxmltree::Document::parse(opf).map_err(|e| format!("its package document can't be read ({e})"))?;
    let meta = doc.descendants().find(|n| n.tag_name().name() == "metadata").ok_or("its package document has no metadata")?;
    let dc = |name: &'static str| {
        meta.children().filter(move |n| n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(DC))
    };
    // the prefix the book uses for Dublin Core (dc: almost always)
    let prefix = meta.lookup_prefix(DC).unwrap_or("dc").to_string();
    let el = |name: &str, text: &str| -> String { format!("<{prefix}:{name}>{}</{prefix}:{name}>", xml(text)) };

    // (start, end, replacement), applied from the end so earlier ranges stay right
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut added = String::new();
    let replace_first = |name: &'static str, value: &str, edits: &mut Vec<(usize, usize, String)>, added: &mut String| {
        if value.trim().is_empty() {
            return;
        }
        match dc(name).next() {
            Some(n) => edits.push((n.range().start, n.range().end, el(name, value))),
            None => added.push_str(&format!("\n    {}", el(name, value))),
        }
    };
    replace_first("title", &m.title, &mut edits, &mut added);
    replace_first("language", &m.language, &mut edits, &mut added);
    if !m.authors.is_empty() {
        let creators: Vec<roxmltree::Node> = dc("creator").collect();
        let all = m.authors.iter().map(|a| el("creator", &a.name)).collect::<Vec<_>>().join("\n    ");
        match creators.split_first() {
            Some((first, rest)) => {
                edits.push((first.range().start, first.range().end, all));
                for n in rest {
                    edits.push((n.range().start, n.range().end, String::new()));
                }
            }
            None => added.push_str(&format!("\n    {all}")),
        }
    }
    if !m.isbn13.is_empty() && !dc("identifier").any(|n| n.text().unwrap_or("").replace('-', "").contains(&m.isbn13)) {
        added.push_str(&format!("\n    {}", el("identifier", &format!("urn:isbn:{}", m.isbn13))));
    }
    if let Some(s) = m.series.first().filter(|s| !s.name.is_empty()) {
        let has = meta.children().any(|n| n.attribute("name") == Some("calibre:series"));
        if !has {
            added.push_str(&format!("\n    <meta name=\"calibre:series\" content=\"{}\"/>", xml(&s.name)));
            if !s.position.is_empty() {
                added.push_str(&format!("\n    <meta name=\"calibre:series_index\" content=\"{}\"/>", xml(&s.position)));
            }
        }
    }
    let mut manifest_add = String::new();
    if let Some(href) = cover_href {
        added.push_str("\n    <meta name=\"cover\" content=\"folio-cover\"/>");
        manifest_add =
            format!("\n    <item id=\"folio-cover\" href=\"{}\" media-type=\"image/jpeg\" properties=\"cover-image\"/>", xml(href));
    }
    // the metadata's closing tag: just before its end
    let meta_src = &opf[meta.range()];
    let close = meta.range().start + meta_src.rfind("</").ok_or("its metadata isn't closed")?;
    if !added.is_empty() {
        edits.push((close, close, format!("{added}\n  ")));
    }
    if !manifest_add.is_empty() {
        let man = doc.descendants().find(|n| n.tag_name().name() == "manifest").ok_or("its package document has no manifest")?;
        let src = &opf[man.range()];
        let at = man.range().start + src.rfind("</").ok_or("its manifest isn't closed")?;
        edits.push((at, at, format!("{manifest_add}\n  ")));
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = opf.to_string();
    for (s, e, r) in edits {
        out.replace_range(s..e, &r);
    }
    Ok(out)
}

/// Writes `dst`: the EPUB `src` with the edition's metadata, and `cover` (JPEG bytes) when the book has no cover
/// of its own.
pub fn rewrite(src: &Path, dst: &Path, m: &EditionMeta, cover: Option<&[u8]>) -> Result<()> {
    let bad = |why: String| err(422, format!("{} can't be rewritten: {why}.", src.display()));
    let mut z = zip::ZipArchive::new(std::fs::File::open(src).map_err(|e| bad(e.to_string()))?).map_err(|e| bad(e.to_string()))?;
    let mut container = String::new();
    z.by_name("META-INF/container.xml")
        .map_err(|_| bad("container.xml is missing".into()))?
        .read_to_string(&mut container)
        .map_err(|e| bad(e.to_string()))?;
    let cdoc = roxmltree::Document::parse(&container).map_err(|e| bad(e.to_string()))?;
    let opf_path = cdoc
        .descendants()
        .find(|n| n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| bad("container.xml names no package document".into()))?
        .to_string();
    let mut opf = String::new();
    z.by_name(&opf_path).map_err(|_| bad(format!("{opf_path} is missing")))?.read_to_string(&mut opf).map_err(|e| bad(e.to_string()))?;
    let has_cover = opf.contains("cover-image") || opf.contains("name=\"cover\"");
    let base = opf_path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let cover = cover.filter(|_| !has_cover);
    let new_opf = rewrite_opf(&opf, m, cover.map(|_| "folio-cover.jpg")).map_err(|e| bad(e.to_string()))?;

    let tmp = dst.with_extension("epub.part");
    let file = std::fs::File::create(&tmp).map_err(|e| err(500, format!("{} can't be written: {e}", tmp.display())))?;
    let mut w = zip::ZipWriter::new(file);
    let write_err = |e: &dyn std::fmt::Display| err(500, format!("{} can't be written: {e}", dst.display()));
    for i in 0..z.len() {
        let f = z.by_index_raw(i).map_err(|e| bad(e.to_string()))?;
        if f.name() == opf_path {
            drop(f);
            w.start_file(opf_path.as_str(), zip::write::SimpleFileOptions::default()).map_err(|e| write_err(&e))?;
            w.write_all(new_opf.as_bytes()).map_err(|e| write_err(&e))?;
        } else {
            w.raw_copy_file(f).map_err(|e| write_err(&e))?;
        }
    }
    if let Some(c) = cover {
        w.start_file(
            format!("{base}folio-cover.jpg"),
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )
        .map_err(|e| write_err(&e))?;
        w.write_all(c).map_err(|e| write_err(&e))?;
    }
    w.finish().map_err(|e| write_err(&e))?;
    std::fs::rename(&tmp, dst).map_err(|e| write_err(&e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Person, SeriesRef};

    const OPF: &str = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">uuid:1</dc:identifier><dc:title>project hail mary (retail)</dc:title><dc:creator>Weir, A.</dc:creator><dc:creator>Someone</dc:creator></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;

    #[test]
    fn the_opf_gets_the_editions_metadata() {
        let m = EditionMeta {
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            language: "en".into(),
            isbn13: "9780593135204".into(),
            series: vec![SeriesRef { name: "Saga & Co".into(), position: "2".into(), asin: String::new() }],
            ..Default::default()
        };
        let out = rewrite_opf(OPF, &m, Some("folio-cover.jpg")).unwrap();
        let d = roxmltree::Document::parse(&out).expect("still valid XML");
        let texts = |name: &str| d.descendants().filter(|n| n.tag_name().name() == name).filter_map(|n| n.text()).collect::<Vec<_>>();
        assert_eq!(texts("title"), ["Project Hail Mary"]);
        assert_eq!(texts("creator"), ["Andy Weir"]);
        assert_eq!(texts("language"), ["en"]);
        assert_eq!(texts("identifier"), ["uuid:1", "urn:isbn:9780593135204"]);
        assert!(out.contains(r#"<meta name="calibre:series" content="Saga &amp; Co"/>"#), "{out}");
        assert!(out.contains(r#"href="folio-cover.jpg""#) && out.contains(r#"<meta name="cover" content="folio-cover"/>"#), "{out}");
    }
}
