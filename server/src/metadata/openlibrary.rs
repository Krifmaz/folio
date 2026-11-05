//! Open Library (<https://openlibrary.org>): `GET /isbn/{isbn}.json` gives the edition (pages, publisher, date,
//! language, cover, its work and authors), `GET /works/{key}.json` the work's blurb and subjects, and
//! `GET /authors/{key}.json` each author's name.

use super::{get_json, js, EditionMeta, Format, Person, Providers};
use crate::library::text;
use kit_server_core::Result;
use serde_json::Value;

/// The edition with this ISBN-13; `None` when Open Library does not know it.
pub async fn edition(p: &Providers, isbn: &str) -> Result<Option<EditionMeta>> {
    let base = &p.openlibrary_url;
    let Some(ed) = get_json("Open Library", p.http.get(format!("{base}/isbn/{isbn}.json"))).await? else { return Ok(None) };
    let work_key = ed.get("works").and_then(Value::as_array).and_then(|w| w.first()).map(|w| key(&js(w, "key"))).unwrap_or_default();
    let work = if work_key.is_empty() {
        None
    } else {
        get_json("Open Library", p.http.get(format!("{base}/works/{work_key}.json"))).await.ok().flatten()
    };
    // the edition names its authors; older records only name them on the work
    let mut author_keys = author_keys(ed.get("authors"));
    if author_keys.is_empty() {
        author_keys = work.as_ref().map(|w| author_keys_of_work(w.get("authors"))).unwrap_or_default();
    }
    let mut authors = Vec::new();
    for k in author_keys.iter().take(6) {
        if let Ok(Some(a)) = get_json("Open Library", p.http.get(format!("{base}/authors/{k}.json"))).await {
            let name = js(&a, "name");
            if !name.is_empty() {
                authors.push(Person { name, asin: String::new(), ol_key: k.clone() });
            }
        }
    }
    Ok(parse(&ed, work.as_ref(), authors))
}

/// The last part of an Open Library path (`/works/OL45883W` → `OL45883W`).
fn key(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("").to_string()
}

/// The author keys of an edition's `authors` (`[{key}]`).
fn author_keys(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().map(|x| key(&js(x, "key"))).filter(|k| !k.is_empty()).collect()).unwrap_or_default()
}

/// The author keys of a work's `authors` (`[{author: {key}}]`).
fn author_keys_of_work(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("author")).map(|x| key(&js(x, "key"))).filter(|k| !k.is_empty()).collect())
        .unwrap_or_default()
}

/// Open Library's edition, work and authors as one edition; `None` when the edition has no title.
pub fn parse(ed: &Value, work: Option<&Value>, authors: Vec<Person>) -> Option<EditionMeta> {
    let title = js(ed, "title");
    if title.is_empty() {
        return None;
    }
    let description = work
        .and_then(|w| w.get("description"))
        .or_else(|| ed.get("description"))
        .map(|d| d.as_str().map(str::to_string).unwrap_or_else(|| js(d, "value")))
        .unwrap_or_default();
    let language =
        ed.get("languages").and_then(Value::as_array).and_then(|l| l.first()).map(|l| text::language(&js(l, "key"))).unwrap_or_default();
    let cover = ed.get("covers").and_then(Value::as_array).and_then(|c| c.iter().filter_map(Value::as_i64).find(|&id| id > 0));
    let genres = work
        .and_then(|w| w.get("subjects"))
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(Value::as_str).take(8).map(str::to_string).collect())
        .unwrap_or_default();
    let first = |k: &str| ed.get(k).and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).unwrap_or("").to_string();
    Some(EditionMeta {
        format: Format::Ebook,
        title,
        subtitle: js(ed, "subtitle"),
        authors,
        language,
        pages: ed.get("number_of_pages").and_then(Value::as_u64).map(|n| n as u32),
        publisher: first("publishers"),
        release_date: text::date(&js(ed, "publish_date")),
        description,
        cover_url: cover.map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg")).unwrap_or_default(),
        genres,
        ol_work: ed.get("works").and_then(Value::as_array).and_then(|w| w.first()).map(|w| key(&js(w, "key"))).unwrap_or_default(),
        sources: vec!["openlibrary".into()],
        ..Default::default()
    })
}

/// One edition of a work as Open Library lists it: enough to pick the ebook to want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditionRef {
    /// the ISBN-13
    pub isbn13: String,
    /// the language, two letters ("" when not said)
    pub language: String,
    /// whether Open Library calls it an ebook
    pub ebook: bool,
}

/// The works of the author `key` (`OL7234434A`): `(work key, title)`, up to 100.
pub async fn author_works(p: &Providers, key: &str) -> Result<Vec<(String, String)>> {
    let v = get_json("Open Library", p.http.get(format!("{}/authors/{key}/works.json", p.openlibrary_url)).query(&[("limit", "100")]))
        .await?
        .unwrap_or(Value::Null);
    Ok(works_of(&v))
}

/// The works of an author's works answer.
pub fn works_of(v: &Value) -> Vec<(String, String)> {
    v.get("entries")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|w| (key(&js(w, "key")), js(w, "title"))).filter(|(k, t)| !k.is_empty() && !t.is_empty()).collect())
        .unwrap_or_default()
}

/// The editions of the work `key` that have an ISBN-13, up to 100.
pub async fn work_editions(p: &Providers, key: &str) -> Result<Vec<EditionRef>> {
    let v = get_json("Open Library", p.http.get(format!("{}/works/{key}/editions.json", p.openlibrary_url)).query(&[("limit", "100")]))
        .await?
        .unwrap_or(Value::Null);
    Ok(editions_of(&v))
}

/// The editions of a work's editions answer.
pub fn editions_of(v: &Value) -> Vec<EditionRef> {
    v.get("entries")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let isbn = e
                        .get("isbn_13")
                        .and_then(Value::as_array)
                        .and_then(|i| i.iter().filter_map(Value::as_str).find_map(crate::library::ids::isbn13))?;
                    let language = e
                        .get("languages")
                        .and_then(Value::as_array)
                        .and_then(|l| l.first())
                        .map(|l| text::language(&js(l, "key")))
                        .unwrap_or_default();
                    let fmt = js(e, "physical_format").to_lowercase();
                    Some(EditionRef {
                        isbn13: isbn,
                        language,
                        ebook: fmt.contains("ebook") || fmt.contains("electronic") || fmt.contains("epub") || fmt.contains("kindle"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edition_with_its_work_becomes_one_edition() {
        let ed: Value = serde_json::from_str(include_str!("../../tests/fixtures/ol_isbn.json")).unwrap();
        let work: Value = serde_json::from_str(include_str!("../../tests/fixtures/ol_work.json")).unwrap();
        assert_eq!(author_keys(ed.get("authors")), vec!["OL7234434A"]);
        assert_eq!(author_keys_of_work(work.get("authors")), vec!["OL7234434A"]);
        let m =
            parse(&ed, Some(&work), vec![Person { name: "Andy Weir".into(), ol_key: "OL7234434A".into(), ..Default::default() }]).unwrap();
        assert_eq!(m.title, "Project Hail Mary");
        assert_eq!(m.language, "en");
        assert_eq!(m.pages, Some(496));
        assert_eq!(m.publisher, "Ballantine Books");
        assert_eq!(m.release_date, "2021");
        assert_eq!(m.ol_work, "OL21745884W");
        assert_eq!(m.cover_url, "https://covers.openlibrary.org/b/id/10521270-L.jpg");
        assert!(m.description.starts_with("A lone astronaut"));
        assert_eq!(m.genres.len(), 3);
    }

    #[test]
    fn author_works_and_work_editions_are_listed() {
        let w: Value = serde_json::from_str(include_str!("../../tests/fixtures/ol_author_works.json")).unwrap();
        assert_eq!(works_of(&w)[1], ("OL17091839W".to_string(), "The Martian".to_string()));
        let e: Value = serde_json::from_str(include_str!("../../tests/fixtures/ol_work_editions.json")).unwrap();
        let e = editions_of(&e);
        assert_eq!(e.len(), 3);
        assert_eq!((e[0].language.as_str(), e[2].ebook), ("de", true));
    }
}
