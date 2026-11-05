//! Looking editions up: audiobooks by Audible ASIN on Audnexus, ebooks by ISBN on Open Library, Google Books
//! and Hardcover. Every source answers in its own shape; each module turns that into one [`EditionMeta`], and
//! [`merge`] joins the ebook sources so each fills what the others miss.
//!
//! The addresses, the Audible store and the tokens are settings (Settings → Metadata), so tests and people
//! behind a mirror can point them elsewhere.

pub mod audnexus;
pub mod googlebooks;
pub mod hardcover;
pub mod openlibrary;

use crate::library::{ids, text};
use kit_server_core::{err, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// What an edition is: listened to or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// an audiobook, known by its ASIN
    Audiobook,
    /// an ebook, known by its ISBN
    #[default]
    Ebook,
}

impl Format {
    /// The word kept in the database and sent to clients (`audiobook`, `ebook`).
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Audiobook => "audiobook",
            Format::Ebook => "ebook",
        }
    }

    /// The format a stored word names; `None` for anything else.
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "audiobook" => Some(Format::Audiobook),
            "ebook" => Some(Format::Ebook),
            _ => None,
        }
    }
}

/// A person a source names: an author, with the source's own id when it has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Person {
    /// the name as the source writes it
    pub name: String,
    /// Audible's author ASIN, when Audnexus gives one
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asin: String,
    /// Open Library's author key (`OL23919A`), when Open Library gives one
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ol_key: String,
}

/// A series an edition belongs to, and where in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SeriesRef {
    /// the series' name
    pub name: String,
    /// the place in the series as the source writes it (`"1"`, `"2.5"`, `"1-3"`); empty when unknown
    #[serde(default)]
    pub position: String,
    /// Audible's series ASIN, when Audnexus gives one
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asin: String,
}

/// One chapter of an audiobook, from Audnexus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Chapter {
    /// the chapter's title
    pub title: String,
    /// where it starts, in milliseconds from the start
    pub start_ms: u64,
    /// how long it lasts, in milliseconds
    pub length_ms: u64,
}

/// One edition as the sources describe it: everything Folio keeps about an exact edition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EditionMeta {
    /// audiobook or ebook
    pub format: Format,
    /// the Audible ASIN (audiobooks)
    #[serde(default)]
    pub asin: String,
    /// the ISBN-13 (ebooks; some audiobooks have one too)
    #[serde(default)]
    pub isbn13: String,
    /// the ISBN-10, when the ISBN-13 has one
    #[serde(default)]
    pub isbn10: String,
    /// the title without its subtitle
    pub title: String,
    /// the subtitle
    #[serde(default)]
    pub subtitle: String,
    /// the authors, in the source's order
    #[serde(default)]
    pub authors: Vec<Person>,
    /// the narrators (audiobooks)
    #[serde(default)]
    pub narrators: Vec<String>,
    /// the series, the main one first
    #[serde(default)]
    pub series: Vec<SeriesRef>,
    /// the language as a two-letter code (`en`)
    #[serde(default)]
    pub language: String,
    /// how long the audiobook runs, in minutes
    #[serde(default)]
    pub runtime_min: Option<u32>,
    /// whether the audiobook is abridged; `None` when the source does not say
    #[serde(default)]
    pub abridged: Option<bool>,
    /// how many pages the book has
    #[serde(default)]
    pub pages: Option<u32>,
    /// the publisher
    #[serde(default)]
    pub publisher: String,
    /// the release date, `YYYY-MM-DD` or the year
    #[serde(default)]
    pub release_date: String,
    /// the blurb, as plain text or simple HTML
    #[serde(default)]
    pub description: String,
    /// a cover picture's address
    #[serde(default)]
    pub cover_url: String,
    /// genres and subjects
    #[serde(default)]
    pub genres: Vec<String>,
    /// the audiobook's chapters (Audnexus)
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    /// Open Library's work key (`OL45883W`), which ties editions of one work together
    #[serde(default)]
    pub ol_work: String,
    /// the sources that answered (`audnexus`, `openlibrary`, `googlebooks`, `hardcover`)
    #[serde(default)]
    pub sources: Vec<String>,
}

impl EditionMeta {
    /// The first author's name, "" when there is none.
    pub fn author(&self) -> &str {
        self.authors.first().map(|a| a.name.as_str()).unwrap_or("")
    }

    /// The year of the release date.
    pub fn year(&self) -> Option<i32> {
        text::year(&self.release_date)
    }
}

/// Where the sources are and how to ask them: the settings in effect when a lookup starts.
#[derive(Debug, Clone)]
pub struct Providers {
    /// one HTTP client for every source
    pub http: reqwest::Client,
    /// Audnexus's address
    pub audnexus_url: String,
    /// the Audible store (`us`, `uk` …)
    pub region: String,
    /// Open Library's address
    pub openlibrary_url: String,
    /// Google Books' address
    pub googlebooks_url: String,
    /// Google Books' API key; empty: none
    pub google_key: String,
    /// Hardcover's GraphQL address
    pub hardcover_url: String,
    /// Hardcover's API token; empty: Hardcover is not asked
    pub hardcover_token: String,
}

/// The HTTP client every outside call uses: a 20-second limit and Folio's name and version.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("Folio/{} (+https://github.com/Krifmaz/folio)", crate::VERSION))
        .build()
        .unwrap_or_default()
}

impl Providers {
    /// The sources as the settings `s` say.
    pub fn from_settings(http: reqwest::Client, s: &kit_server_settings::Store) -> Providers {
        let url = |k: &str| s.str(k).trim().trim_end_matches('/').to_string();
        Providers {
            http,
            audnexus_url: url("audnexus_url"),
            region: s.str("audible_region"),
            openlibrary_url: url("openlibrary_url"),
            googlebooks_url: url("googlebooks_url"),
            google_key: s.str("google_books_key"),
            hardcover_url: url("hardcover_url"),
            hardcover_token: s.str("hardcover_token"),
        }
    }

    /// The audiobook with this ASIN, from Audnexus, with its chapters. 400 for something that is not an ASIN,
    /// 404 when Audnexus does not know it, 502 when Audnexus can't be reached.
    pub async fn audiobook(&self, raw_asin: &str) -> Result<EditionMeta> {
        let asin = ids::asin(raw_asin).ok_or_else(|| {
            err(400, format!("\"{}\" is not an Audible ASIN. An ASIN is 10 letters and digits, like B08G9PRS1K.", raw_asin.trim()))
        })?;
        audnexus::book(self, &asin).await
    }

    /// The ebook with this ISBN, from Open Library, Google Books and Hardcover (when a token is set), merged.
    /// 400 for something that is not an ISBN, 404 when no source knows it, 502 when none could be reached.
    pub async fn ebook(&self, raw_isbn: &str) -> Result<EditionMeta> {
        let isbn = ids::isbn13(raw_isbn).ok_or_else(|| {
            err(
                400,
                format!(
                    "\"{}\" is not a valid ISBN. Check the digits: an ISBN has 10 or 13, and the last one is a check digit.",
                    raw_isbn.trim()
                ),
            )
        })?;
        let (ol, gb, hc) = tokio::join!(openlibrary::edition(self, &isbn), googlebooks::edition(self, &isbn), async {
            if self.hardcover_token.trim().is_empty() {
                Ok(None)
            } else {
                hardcover::edition(self, &isbn).await
            }
        });
        let mut failures = Vec::new();
        let mut found = Vec::new();
        for r in [ol, hc, gb] {
            match r {
                Ok(Some(m)) => found.push(m),
                Ok(None) => {}
                Err(e) => failures.push(e.detail),
            }
        }
        if found.is_empty() {
            if !failures.is_empty() {
                return Err(err(502, format!("No book source could be asked about ISBN {isbn}: {}", failures.join(" "))));
            }
            return Err(err(404, format!("No book with ISBN {isbn} was found on Open Library, Google Books or Hardcover.")));
        }
        let mut m = merge(found);
        m.isbn13 = isbn.clone();
        m.isbn10 = ids::isbn10(&isbn).unwrap_or_default();
        Ok(m)
    }
}

/// The ebook sources' answers joined, the first answer first: each field takes the first source that has it,
/// except the blurb, which takes the longest (Open Library's is often missing or short). Authors come from the
/// first source that names any, with Open Library keys added where the names match.
pub fn merge(parts: Vec<EditionMeta>) -> EditionMeta {
    let mut out = EditionMeta { format: Format::Ebook, ..Default::default() };
    for p in &parts {
        let fill = |dst: &mut String, src: &str| {
            if dst.is_empty() && !src.trim().is_empty() {
                *dst = src.trim().to_string();
            }
        };
        fill(&mut out.title, &p.title);
        fill(&mut out.subtitle, &p.subtitle);
        fill(&mut out.language, &p.language);
        fill(&mut out.publisher, &p.publisher);
        fill(&mut out.release_date, &p.release_date);
        fill(&mut out.cover_url, &p.cover_url);
        fill(&mut out.ol_work, &p.ol_work);
        if p.description.trim().len() > out.description.len() {
            out.description = p.description.trim().to_string();
        }
        if out.authors.is_empty() {
            out.authors = p.authors.clone();
        } else {
            for a in &p.authors {
                if let Some(mine) = out.authors.iter_mut().find(|m| text::person_key(&m.name) == text::person_key(&a.name)) {
                    fill(&mut mine.ol_key, &a.ol_key);
                    fill(&mut mine.asin, &a.asin);
                }
            }
        }
        if out.series.is_empty() {
            out.series = p.series.clone();
        }
        if out.pages.is_none() {
            out.pages = p.pages;
        }
        for g in &p.genres {
            if !out.genres.iter().any(|x| x.eq_ignore_ascii_case(g)) {
                out.genres.push(g.clone());
            }
        }
        out.sources.extend(p.sources.iter().cloned());
    }
    out.genres.truncate(12);
    out
}

/// Asks `req` and reads its JSON answer: `None` for a 404, the source's name and a plain sentence for anything
/// else that went wrong.
pub(crate) async fn get_json(source: &str, req: reqwest::RequestBuilder) -> Result<Option<serde_json::Value>> {
    let resp = req.send().await.map_err(|e| err(502, format!("{source} can't be reached ({}).", short(&e))))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        let why = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            "is busy and asked Folio to wait".to_string()
        } else {
            format!("answered with an error ({})", status.as_u16())
        };
        return Err(err(502, format!("{source} {why}.")));
    }
    resp.json().await.map(Some).map_err(|e| err(502, format!("{source} sent an answer Folio can't read ({}).", short(&e))))
}

/// A short reason for an HTTP failure, without the address.
fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "it took too long to answer".into()
    } else if e.is_connect() {
        "the connection failed".into()
    } else if e.is_decode() {
        "the answer was not JSON".into()
    } else {
        std::error::Error::source(e).map(|s| s.to_string()).unwrap_or_else(|| "the request failed".into())
    }
}

/// A JSON string field, trimmed; "" when missing.
pub(crate) fn js(v: &serde_json::Value, k: &str) -> String {
    v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merging_fills_gaps_and_keeps_the_longest_blurb() {
        let ol = EditionMeta {
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), ol_key: "OL1A".into(), ..Default::default() }],
            pages: Some(496),
            description: "Short.".into(),
            ol_work: "OL1W".into(),
            sources: vec!["openlibrary".into()],
            ..Default::default()
        };
        let gb = EditionMeta {
            title: "Project Hail Mary (ignored)".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            language: "en".into(),
            description: "A much longer blurb about a lone astronaut.".into(),
            cover_url: "https://books.google/x.jpg".into(),
            genres: vec!["Fiction".into()],
            sources: vec!["googlebooks".into()],
            ..Default::default()
        };
        let m = merge(vec![ol, gb]);
        assert_eq!(m.title, "Project Hail Mary");
        assert_eq!(m.authors[0].ol_key, "OL1A");
        assert_eq!(m.language, "en");
        assert_eq!(m.pages, Some(496));
        assert!(m.description.starts_with("A much longer"));
        assert_eq!(m.sources, vec!["openlibrary", "googlebooks"]);
        assert_eq!(m.format, Format::Ebook);
    }
}
