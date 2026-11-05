//! Hardcover (<https://hardcover.app>): its GraphQL API, with the owner's API token, gives the edition with an
//! ISBN and its book's authors, blurb and series.

use super::{get_json, js, EditionMeta, Format, Person, Providers, SeriesRef};
use crate::library::text;
use kit_server_core::{err, Result};
use serde_json::{json, Value};

/// The query: the edition with the ISBN-13 or ISBN-10, and its book.
const QUERY: &str = "query Edition($isbn13: String!, $isbn10: String!) {
  editions(where: {_or: [{isbn_13: {_eq: $isbn13}}, {isbn_10: {_eq: $isbn10}}]}, limit: 1) {
    title subtitle pages release_date isbn_13 isbn_10
    language { language }
    publisher { name }
    image { url }
    book { title description contributions { author { name } } book_series { position series { name } } }
  }
}";

/// The edition with this ISBN-13; `None` when Hardcover has none.
pub async fn edition(p: &Providers, isbn: &str) -> Result<Option<EditionMeta>> {
    let token = p.hardcover_token.trim();
    let auth = if token.to_ascii_lowercase().starts_with("bearer ") { token.to_string() } else { format!("Bearer {token}") };
    let isbn10 = crate::library::ids::isbn10(isbn).unwrap_or_default();
    let body = json!({ "query": QUERY, "variables": { "isbn13": isbn, "isbn10": isbn10 } });
    let Some(v) = get_json("Hardcover", p.http.post(&p.hardcover_url).header("authorization", auth).json(&body)).await? else {
        return Ok(None);
    };
    if let Some(e) = v.get("errors").and_then(Value::as_array).and_then(|e| e.first()) {
        return Err(err(
            502,
            format!("Hardcover refused the lookup: {}. Check the Hardcover API token under Settings → Metadata.", js(e, "message")),
        ));
    }
    Ok(parse(&v))
}

/// A Hardcover answer as an edition; `None` when it found no edition.
pub fn parse(v: &Value) -> Option<EditionMeta> {
    let ed = v.pointer("/data/editions/0")?;
    let book = ed.get("book").cloned().unwrap_or(Value::Null);
    let title = [js(ed, "title"), js(&book, "title")].into_iter().find(|t| !t.is_empty())?;
    let authors = book
        .get("contributions")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|x| x.pointer("/author/name").and_then(Value::as_str))
                .map(|n| Person { name: n.to_string(), ..Default::default() })
                .collect()
        })
        .unwrap_or_default();
    let series = book
        .get("book_series")
        .and_then(Value::as_array)
        .map(|s| {
            s.iter()
                .map(|x| SeriesRef {
                    name: x.pointer("/series/name").and_then(Value::as_str).unwrap_or("").to_string(),
                    position: x
                        .get("position")
                        .map(|p| p.as_str().map(str::to_string).unwrap_or_else(|| if p.is_null() { String::new() } else { p.to_string() }))
                        .unwrap_or_default(),
                    asin: String::new(),
                })
                .filter(|s| !s.name.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Some(EditionMeta {
        format: Format::Ebook,
        title,
        subtitle: js(ed, "subtitle"),
        authors,
        series,
        language: ed.pointer("/language/language").and_then(Value::as_str).map(text::language).unwrap_or_default(),
        pages: ed.get("pages").and_then(Value::as_u64).map(|n| n as u32),
        publisher: ed.pointer("/publisher/name").and_then(Value::as_str).unwrap_or("").to_string(),
        release_date: text::date(&js(ed, "release_date")),
        description: js(&book, "description"),
        cover_url: ed.pointer("/image/url").and_then(Value::as_str).unwrap_or("").to_string(),
        sources: vec!["hardcover".into()],
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edition_answer_is_read() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/hardcover_edition.json")).unwrap();
        let m = parse(&v).unwrap();
        assert_eq!(m.title, "Project Hail Mary");
        assert_eq!(m.author(), "Andy Weir");
        assert_eq!(m.language, "en");
        assert_eq!(m.pages, Some(496));
        assert!(parse(&json!({"data": {"editions": []}})).is_none());
    }
}
