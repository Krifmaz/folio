//! Audnexus (<https://audnex.us>): Audible's audiobook data by ASIN. `GET /books/{asin}?region=…` gives the
//! edition (narrators, runtime, abridged or not, language, series and place), `GET /books/{asin}/chapters?region=…`
//! its chapters.

use super::{get_json, js, Chapter, EditionMeta, Format, Person, Providers, SeriesRef};
use crate::library::{ids, text};
use kit_server_core::{err, Result};
use serde_json::Value;

/// The audiobook `asin` (already clean) with its chapters. A book without chapter data still comes back, with
/// no chapters.
pub async fn book(p: &Providers, asin: &str) -> Result<EditionMeta> {
    let url = format!("{}/books/{asin}", p.audnexus_url);
    let book = get_json("Audnexus", p.http.get(&url).query(&[("region", p.region.as_str())]))
        .await?
        .ok_or_else(|| err(404, format!("Audnexus doesn't know the ASIN {asin} in the {} Audible store. Check the ASIN, or pick the store it is from under Settings → Metadata.", p.region.to_uppercase())))?;
    let mut m = parse_book(&book).ok_or_else(|| err(502, format!("Audnexus's answer for {asin} has no title.")))?;
    let chapters = get_json("Audnexus", p.http.get(format!("{url}/chapters")).query(&[("region", p.region.as_str())])).await;
    if let Ok(Some(c)) = chapters {
        m.chapters = parse_chapters(&c);
    }
    Ok(m)
}

/// An Audnexus book answer as an edition; `None` when it has no title.
pub fn parse_book(b: &Value) -> Option<EditionMeta> {
    let title = js(b, "title");
    if title.is_empty() {
        return None;
    }
    let people = |k: &str| -> Vec<Person> {
        b.get(k)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|x| Person { name: js(x, "name"), asin: js(x, "asin"), ol_key: String::new() })
                    .filter(|x| !x.name.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let series = ["seriesPrimary", "seriesSecondary"]
        .iter()
        .filter_map(|k| b.get(*k))
        .map(|s| SeriesRef { name: js(s, "name"), position: clean_position(&js(s, "position")), asin: js(s, "asin") })
        .filter(|s| !s.name.is_empty())
        .collect();
    let format_type = js(b, "formatType").to_lowercase();
    let abridged = match format_type.as_str() {
        "abridged" => Some(true),
        "unabridged" => Some(false),
        _ => None,
    };
    let genres = b
        .get("genres")
        .and_then(Value::as_array)
        .map(|g| g.iter().map(|x| js(x, "name")).filter(|n| !n.is_empty()).collect())
        .unwrap_or_default();
    let description = [js(b, "summary"), js(b, "description")].into_iter().max_by_key(String::len).unwrap_or_default();
    Some(EditionMeta {
        format: Format::Audiobook,
        asin: ids::asin(&js(b, "asin")).unwrap_or_default(),
        isbn13: ids::isbn13(&js(b, "isbn")).unwrap_or_default(),
        isbn10: String::new(),
        title,
        subtitle: js(b, "subtitle"),
        authors: people("authors"),
        narrators: people("narrators").into_iter().map(|n| n.name).collect(),
        series,
        language: text::language(&js(b, "language")),
        runtime_min: b.get("runtimeLengthMin").and_then(Value::as_u64).map(|m| m as u32),
        abridged,
        pages: None,
        publisher: js(b, "publisherName"),
        release_date: text::date(&js(b, "releaseDate")),
        description,
        cover_url: js(b, "image"),
        genres,
        chapters: Vec::new(),
        ol_work: String::new(),
        sources: vec!["audnexus".into()],
    })
}

/// An Audnexus chapters answer as chapters (Audible's brand intro and outro stay part of the first and last).
pub fn parse_chapters(c: &Value) -> Vec<Chapter> {
    c.get("chapters")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| Chapter {
                    title: js(x, "title"),
                    start_ms: x.get("startOffsetMs").and_then(Value::as_u64).unwrap_or(0),
                    length_ms: x.get("lengthMs").and_then(Value::as_u64).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A series position as Audible writes it, without words (`"Book 1"` → `"1"`, `"1-3"` stays).
fn clean_position(p: &str) -> String {
    let p = p.trim();
    let start = p.find(|c: char| c.is_ascii_digit()).unwrap_or(p.len());
    p[start..].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_book_answer_becomes_an_edition() {
        let b: Value = serde_json::from_str(include_str!("../../tests/fixtures/audnexus_book.json")).unwrap();
        let m = parse_book(&b).unwrap();
        assert_eq!(m.asin, "B08G9PRS1K");
        assert_eq!(m.title, "Project Hail Mary");
        assert_eq!(m.author(), "Andy Weir");
        assert_eq!(m.narrators, vec!["Ray Porter"]);
        assert_eq!(m.runtime_min, Some(970));
        assert_eq!(m.abridged, Some(false));
        assert_eq!(m.language, "en");
        assert_eq!(m.release_date, "2021-05-04");
        assert!(m.series.is_empty());
        let c: Value = serde_json::from_str(include_str!("../../tests/fixtures/audnexus_chapters.json")).unwrap();
        let ch = parse_chapters(&c);
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[1].start_ms, 1_000_000);
        assert_eq!(clean_position("Book 2.5"), "2.5");
    }
}
