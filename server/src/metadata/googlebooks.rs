//! Google Books: `GET /volumes?q=isbn:{isbn}` gives title, subtitle, authors, publisher, date, blurb, pages,
//! categories, cover and language of the volume with that ISBN.

use super::{get_json, js, EditionMeta, Format, Person, Providers};
use crate::library::{ids, text};
use kit_server_core::Result;
use serde_json::Value;

/// The volume with this ISBN-13; `None` when Google Books has none.
pub async fn edition(p: &Providers, isbn: &str) -> Result<Option<EditionMeta>> {
    let mut req = p.http.get(format!("{}/volumes", p.googlebooks_url)).query(&[("q", format!("isbn:{isbn}"))]);
    if !p.google_key.trim().is_empty() {
        req = req.query(&[("key", p.google_key.trim())]);
    }
    let Some(v) = get_json("Google Books", req).await? else { return Ok(None) };
    Ok(parse(&v, isbn))
}

/// The volume of a search answer whose ISBN is `isbn` (Google sometimes adds near matches), as an edition.
pub fn parse(v: &Value, isbn: &str) -> Option<EditionMeta> {
    let items = v.get("items").and_then(Value::as_array)?;
    let info = items.iter().filter_map(|i| i.get("volumeInfo")).find(|vi| {
        vi.get("industryIdentifiers")
            .and_then(Value::as_array)
            .is_some_and(|ids_| ids_.iter().any(|x| ids::isbn13(&js(x, "identifier")).as_deref() == Some(isbn)))
    })?;
    let title = js(info, "title");
    if title.is_empty() {
        return None;
    }
    let strings = |k: &str| -> Vec<String> {
        info.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let cover = info.get("imageLinks").map(|l| {
        let u = ["extraLarge", "large", "medium", "thumbnail"].iter().map(|k| js(l, k)).find(|u| !u.is_empty()).unwrap_or_default();
        u.replacen("http://", "https://", 1)
    });
    Some(EditionMeta {
        format: Format::Ebook,
        title,
        subtitle: js(info, "subtitle"),
        authors: strings("authors").into_iter().map(|name| Person { name, ..Default::default() }).collect(),
        language: text::language(&js(info, "language")),
        pages: info.get("pageCount").and_then(Value::as_u64).map(|n| n as u32),
        publisher: js(info, "publisher"),
        release_date: text::date(&js(info, "publishedDate")),
        description: js(info, "description"),
        cover_url: cover.unwrap_or_default(),
        genres: strings("categories"),
        sources: vec!["googlebooks".into()],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_with_the_isbn_is_read() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/gb_volumes.json")).unwrap();
        let m = parse(&v, "9780593135204").unwrap();
        assert_eq!((m.title.as_str(), m.subtitle.as_str()), ("Project Hail Mary", "A Novel"));
        assert_eq!(m.author(), "Andy Weir");
        assert_eq!(m.language, "en");
        assert_eq!(m.release_date, "2021-05-04");
        assert!(m.cover_url.starts_with("https://books.google.com/"));
        assert!(parse(&v, "9780000000002").is_none(), "another ISBN's volume is not taken");
    }
}
