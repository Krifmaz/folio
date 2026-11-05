//! Searching for an exact edition: the queries built from it, and every result scored on all of its facts.
//!
//! A result's score (0–100) adds up what its name and size say: the title and every author word (needed),
//! the narrator, the ASIN or ISBN, a file format the preferences take (better ones score more), the language,
//! a size that fits the runtime or the kind of book, and seeders. A result is refused outright — whatever its
//! score — when it is a sample, an excerpt or a summary, abridged when only unabridged will do, in a file
//! format the preferences don't take, or in another language. Every point and every refusal comes with its
//! reason in plain words.

use crate::library::text;
use crate::metadata::{EditionMeta, Format};
use crate::wanted::store::{language_name, Profile};
use serde::Serialize;

/// One search to send to Lantern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchQuery {
    /// the words (`Andy Weir Project Hail Mary`)
    pub text: String,
    /// for a book search: the author
    pub author: String,
    /// for a book search: the title
    pub title: String,
}

/// Lantern's (Newznab) categories for a format: Audio/Audiobook, or Books/EBook and Books.
pub fn categories(f: Format) -> &'static [u32] {
    match f {
        Format::Audiobook => &[3030],
        Format::Ebook => &[7020, 7000],
    }
}

/// The searches for an edition, most exact first: its ASIN or ISBN, author and title, title and narrator.
pub fn queries(m: &EditionMeta) -> Vec<SearchQuery> {
    let author = m.author().to_string();
    let title = short_title(&m.title);
    let mut out = Vec::new();
    let mut push = |text: String, author: &str, title: &str| {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() && !out.iter().any(|q: &SearchQuery| q.text == text) {
            out.push(SearchQuery { text, author: author.to_string(), title: title.to_string() });
        }
    };
    match m.format {
        Format::Audiobook if !m.asin.is_empty() => push(m.asin.clone(), "", ""),
        Format::Ebook if !m.isbn13.is_empty() => push(m.isbn13.clone(), "", ""),
        _ => {}
    }
    push(format!("{author} {title}"), &author, &title);
    if let Some(n) = m.narrators.first() {
        push(format!("{title} {n}"), "", "");
    }
    push(title.clone(), "", "");
    out
}

/// The title without its subtitle and without characters that trip indexers' searches.
fn short_title(t: &str) -> String {
    let cut = [":", " - ", "(", "["].iter().filter_map(|m| t.find(m)).filter(|&i| i > 0).min().unwrap_or(t.len());
    t[..cut]
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() || c == '\'' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A search result as Lantern sends it (the Prowlarr release fields Folio reads).
#[derive(Debug, Clone, PartialEq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// the result's id at its indexer
    #[serde(default)]
    pub guid: String,
    /// the result's name
    #[serde(default)]
    pub title: String,
    /// the indexer's id in Lantern
    #[serde(default)]
    pub indexer_id: i64,
    /// the indexer's name
    #[serde(default)]
    pub indexer: String,
    /// bytes
    #[serde(default)]
    pub size: u64,
    /// seeders (torrents)
    #[serde(default)]
    pub seeders: Option<i64>,
    /// `torrent` or `usenet`
    #[serde(default)]
    pub protocol: String,
    /// the download link through Lantern
    #[serde(default)]
    pub download_url: Option<String>,
    /// the release's page
    #[serde(default)]
    pub info_url: Option<String>,
    /// when it was published
    #[serde(default)]
    pub publish_date: Option<String>,
}

/// A result scored against the wanted edition.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scored {
    /// the result
    pub release: Release,
    /// 0–100; refused results score 0
    pub score: u32,
    /// why it won't do, in plain words; `None` when it may be taken
    pub refused: Option<String>,
    /// what counted, in plain words (`"The title matches."`)
    pub reasons: Vec<String>,
    /// the file format its name says (`m4b`), "" when it doesn't say
    pub file_format: String,
}

/// The words of a name, folded (`"Andy.Weir-Project_Hail.Mary.M4B"` → `andy weir project hail mary m4b`).
fn words(s: &str) -> Vec<String> {
    text::fold(s).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// Whether every word of `needle` (2 letters or more) is in `hay`.
fn has_all(hay: &[String], needle: &str) -> bool {
    let n: Vec<String> = words(needle).into_iter().filter(|w| w.len() > 1).collect();
    !n.is_empty() && n.iter().all(|w| hay.contains(w))
}

/// Whether `hay` has the words of `phrase` one after the other.
fn has_phrase(hay: &[String], phrase: &str) -> bool {
    let p = words(phrase);
    !p.is_empty() && hay.windows(p.len()).any(|w| w == p.as_slice())
}

/// Words that mark a result as not the whole book.
const NOT_THE_BOOK: &[&str] = &["sample", "excerpt", "preview", "summary", "teaser", "trailer", "dramatized", "dramatised"];

/// Languages a release name may say, with the two-letter code.
const LANGUAGE_WORDS: &[(&str, &str)] = &[
    ("english", "en"),
    ("german", "de"),
    ("deutsch", "de"),
    ("french", "fr"),
    ("francais", "fr"),
    ("spanish", "es"),
    ("espanol", "es"),
    ("italian", "it"),
    ("italiano", "it"),
    ("dutch", "nl"),
    ("nederlands", "nl"),
    ("portuguese", "pt"),
    ("swedish", "sv"),
    ("polish", "pl"),
    ("russian", "ru"),
    ("japanese", "ja"),
];

/// The file format a release name says, from the profile's known ones (`.m4b`, `M4B`, `[EPUB]`).
fn file_format_of(w: &[String], f: Format) -> String {
    crate::wanted::store::known_file_formats(f).iter().find(|k| w.iter().any(|x| x == *k)).map(|k| k.to_string()).unwrap_or_default()
}

/// Scores one result against the wanted edition `m` under the preferences `p`.
pub fn score(m: &EditionMeta, p: &Profile, r: &Release) -> Scored {
    let w = words(&r.title);
    let mut reasons = Vec::new();
    let mut points: i64 = 0;
    let file_format = file_format_of(&w, m.format);
    let refuse = |why: String, reasons: Vec<String>, file_format: String| Scored {
        release: r.clone(),
        score: 0,
        refused: Some(why),
        reasons,
        file_format,
    };

    // what makes it the wrong thing, whatever else matches
    if let Some(bad) = NOT_THE_BOOK.iter().find(|b| w.iter().any(|x| x == *b)) {
        return refuse(format!("Its name says \"{bad}\": it isn't the whole book."), reasons, file_format);
    }
    if m.format == Format::Audiobook {
        let abridged = w.iter().any(|x| x == "abridged") && !w.iter().any(|x| x == "unabridged");
        if abridged && p.unabridged_only {
            return refuse("Its name says it is abridged, and only unabridged audiobooks will do.".into(), reasons, file_format);
        }
    }
    if !file_format.is_empty() && p.file_rank(&file_format).is_none() {
        return refuse(format!("It is {}, which the {} preferences don't take.", file_format.to_uppercase(), p.name), reasons, file_format);
    }
    if let Some((word, code)) = LANGUAGE_WORDS.iter().find(|(word, _)| w.iter().any(|x| x == word)) {
        if !p.languages.is_empty() && !p.languages.iter().any(|l| l == code) {
            return refuse(
                format!(
                    "Its name says it is in {} (\"{word}\"), and the {} preferences want {}.",
                    language_name(code),
                    p.name,
                    p.languages.iter().map(|l| language_name(l)).collect::<Vec<_>>().join(" or ")
                ),
                reasons,
                file_format,
            );
        }
        if m.language == *code {
            points += 5;
            reasons.push(format!("It says it is in {}.", language_name(code)));
        }
    }

    // what makes it this edition
    let id = if m.format == Format::Audiobook { &m.asin } else { &m.isbn13 };
    let id_found = !id.is_empty()
        && (w.iter().any(|x| x.eq_ignore_ascii_case(id))
            || (m.format == Format::Ebook && !m.isbn10.is_empty() && w.iter().any(|x| x.eq_ignore_ascii_case(&m.isbn10))));
    if id_found {
        points += 30;
        reasons.push(format!("It names the {} {id}.", if m.format == Format::Audiobook { "ASIN" } else { "ISBN" }));
    }
    let title = short_title(&m.title);
    if has_phrase(&w, &title) {
        points += 30;
        reasons.push("The title matches.".into());
    } else if has_all(&w, &title) {
        points += 20;
        reasons.push("Every word of the title is there.".into());
    } else if !id_found {
        return refuse(format!("Its name doesn't have the title \"{title}\"."), reasons, file_format);
    }
    let authors: Vec<&str> = m.authors.iter().map(|a| a.name.as_str()).collect();
    if authors.iter().any(|a| has_all(&w, a)) {
        points += 20;
        reasons.push("The author matches.".into());
    } else if authors.iter().any(|a| words(a).last().is_some_and(|last| last.len() > 2 && w.contains(last))) {
        points += 12;
        reasons.push("The author's last name is there.".into());
    } else if !id_found {
        return refuse(
            format!("Its name doesn't have the author {}.", authors.first().copied().unwrap_or("the edition names")),
            reasons,
            file_format,
        );
    }
    if m.format == Format::Audiobook {
        if m.narrators.iter().any(|n| has_all(&w, n)) {
            points += 10;
            reasons.push("The narrator matches.".into());
        }
        if w.iter().any(|x| x == "unabridged") {
            points += 3;
            reasons.push("It says it is unabridged.".into());
        }
    }
    if let Some(rank) = (!file_format.is_empty()).then(|| p.file_rank(&file_format)).flatten() {
        let bonus = 10i64.saturating_sub(rank as i64 * 4).max(1);
        points += bonus;
        reasons.push(format!(
            "It is {}, {} in the preferences.",
            file_format.to_uppercase(),
            if rank == 0 { "the best choice".to_string() } else { format!("choice {}", rank + 1) }
        ));
    }

    // a size that fits
    match size_fits(m, r.size) {
        Some(true) => {
            points += 5;
            reasons.push("Its size fits.".into());
        }
        Some(false) => {
            return refuse(size_problem(m, r.size), reasons, file_format);
        }
        None => {}
    }
    if r.protocol == "torrent" {
        match r.seeders.unwrap_or(0) {
            0 => {
                points -= 15;
                reasons.push("Nobody is sharing it right now.".into());
            }
            s if s >= 5 => points += 2,
            _ => {}
        }
    }
    Scored { release: r.clone(), score: points.clamp(0, 100) as u32, refused: None, reasons, file_format }
}

/// Whether `bytes` fit the edition: an audiobook's 16–400 kbit/s over its runtime, an ebook's 20 KB to 300 MB.
/// `None` when the size or the runtime is unknown.
pub fn size_fits(m: &EditionMeta, bytes: u64) -> Option<bool> {
    if bytes == 0 {
        return None;
    }
    match m.format {
        Format::Audiobook => {
            let secs = u64::from(m.runtime_min?) * 60;
            let kbps = bytes * 8 / 1000 / secs.max(1);
            Some((16..=400).contains(&kbps))
        }
        Format::Ebook => Some((20_000..=300_000_000).contains(&bytes)),
    }
}

/// Why a size doesn't fit, in plain words.
fn size_problem(m: &EditionMeta, bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    match m.format {
        Format::Audiobook => format!(
            "It is {mb:.0} MB, which doesn't fit a {} audiobook (a whole one is about {:.0}–{:.0} MB).",
            crate::lantern::hours(m.runtime_min.unwrap_or(0)),
            f64::from(m.runtime_min.unwrap_or(0)) * 60.0 * 16.0 / 8.0 / 1000.0,
            f64::from(m.runtime_min.unwrap_or(0)) * 60.0 * 400.0 / 8.0 / 1000.0
        ),
        Format::Ebook => format!("It is {mb:.1} MB, which doesn't fit an ebook."),
    }
}

/// The results scored, best first; refused ones last.
pub fn rank(m: &EditionMeta, p: &Profile, releases: Vec<Release>) -> Vec<Scored> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Scored> =
        releases.into_iter().filter(|r| seen.insert((r.indexer_id, r.guid.clone()))).map(|r| score(m, p, &r)).collect();
    out.sort_by(|a, b| {
        a.refused
            .is_some()
            .cmp(&b.refused.is_some())
            .then(b.score.cmp(&a.score))
            .then(b.release.seeders.unwrap_or(0).cmp(&a.release.seeders.unwrap_or(0)))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Person;

    fn audio() -> EditionMeta {
        EditionMeta {
            format: Format::Audiobook,
            asin: "B08G9PRS1K".into(),
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            narrators: vec!["Ray Porter".into()],
            runtime_min: Some(970),
            abridged: Some(false),
            language: "en".into(),
            ..Default::default()
        }
    }

    fn ebook() -> EditionMeta {
        EditionMeta {
            format: Format::Ebook,
            isbn13: "9780593135204".into(),
            isbn10: "0593135202".into(),
            title: "Project Hail Mary: A Novel".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            language: "en".into(),
            ..Default::default()
        }
    }

    fn profile(f: Format) -> Profile {
        match f {
            Format::Audiobook => Profile {
                id: 1,
                format: f,
                name: "Audiobooks".into(),
                file_formats: vec!["m4b".into(), "mp3".into()],
                languages: vec!["en".into()],
                unabridged_only: true,
            },
            Format::Ebook => Profile {
                id: 2,
                format: f,
                name: "Ebooks".into(),
                file_formats: vec!["epub".into(), "azw3".into(), "pdf".into()],
                languages: vec!["en".into()],
                unabridged_only: false,
            },
        }
    }

    fn rel(title: &str, mb: u64) -> Release {
        Release {
            guid: title.into(),
            title: title.into(),
            indexer_id: 1,
            size: mb * 1_000_000,
            seeders: Some(10),
            protocol: "torrent".into(),
            ..Default::default()
        }
    }

    #[test]
    fn queries_go_from_exact_to_loose() {
        let q: Vec<String> = queries(&audio()).into_iter().map(|q| q.text).collect();
        assert_eq!(q, vec!["B08G9PRS1K", "Andy Weir Project Hail Mary", "Project Hail Mary Ray Porter", "Project Hail Mary"]);
        let q: Vec<String> = queries(&ebook()).into_iter().map(|q| q.text).collect();
        assert_eq!(q, vec!["9780593135204", "Andy Weir Project Hail Mary", "Project Hail Mary"]);
        assert_eq!(categories(Format::Ebook), &[7020, 7000]);
    }

    #[test]
    fn the_right_audiobook_scores_high_and_the_best_format_wins() {
        let (m, p) = (audio(), profile(Format::Audiobook));
        let m4b = score(&m, &p, &rel("Andy Weir - Project Hail Mary (Unabridged) [Ray Porter] B08G9PRS1K M4B", 500));
        assert!(m4b.refused.is_none(), "{m4b:?}");
        assert!(m4b.score >= 90, "{m4b:?}");
        assert_eq!(m4b.file_format, "m4b");
        let mp3 = score(&m, &p, &rel("Andy Weir - Project Hail Mary MP3 Ray Porter", 700));
        assert!(mp3.refused.is_none() && mp3.score < m4b.score, "{mp3:?}");
        let ranked = rank(
            &m,
            &p,
            vec![
                rel("Andy Weir - Project Hail Mary MP3 Ray Porter", 700),
                rel("Andy Weir - Project Hail Mary (Unabridged) [Ray Porter] B08G9PRS1K M4B", 500),
            ],
        );
        assert_eq!(ranked[0].file_format, "m4b");
    }

    #[test]
    fn samples_abridged_wrong_formats_languages_and_sizes_are_refused() {
        let (m, p) = (audio(), profile(Format::Audiobook));
        let why = |t: &str, mb: u64| score(&m, &p, &rel(t, mb)).refused.unwrap_or_default();
        assert_eq!(why("Andy Weir - Project Hail Mary SAMPLE m4b", 5), "Its name says \"sample\": it isn't the whole book.");
        assert_eq!(
            why("Andy Weir - Project Hail Mary (Abridged) mp3", 300),
            "Its name says it is abridged, and only unabridged audiobooks will do."
        );
        assert_eq!(why("Andy Weir - Project Hail Mary FLAC", 3000), "It is FLAC, which the Audiobooks preferences don't take.");
        assert_eq!(
            why("Andy Weir - Project Hail Mary German m4b", 500),
            "Its name says it is in German (\"german\"), and the Audiobooks preferences want English."
        );
        assert!(why("Andy Weir - Project Hail Mary m4b", 20).starts_with("It is 20 MB, which doesn't fit a 16 h 10 min audiobook"));
        assert_eq!(why("Andy Weir - The Martian m4b", 500), "Its name doesn't have the title \"Project Hail Mary\".");
        assert_eq!(why("Someone Else - Project Hail Mary m4b", 500), "Its name doesn't have the author Andy Weir.");
    }

    #[test]
    fn ebooks_are_scored_on_isbn_title_author_and_format() {
        let (m, p) = (ebook(), profile(Format::Ebook));
        let epub = score(&m, &p, &rel("Andy Weir - Project Hail Mary (2021) 9780593135204 epub", 3));
        assert!(epub.refused.is_none() && epub.score >= 90, "{epub:?}");
        let pdf = score(&m, &p, &rel("Andy Weir - Project Hail Mary pdf", 12));
        assert!(pdf.refused.is_none() && pdf.score < epub.score);
        let mobi = score(&m, &p, &rel("Andy Weir - Project Hail Mary mobi", 3));
        assert!(mobi.refused.is_some());
        let tiny = score(&m, &p, &rel("Andy Weir - Project Hail Mary epub", 0));
        assert!(tiny.refused.is_none(), "unknown size is not held against it");
        let only_isbn = score(&m, &p, &rel("9780593135204", 3));
        assert!(only_isbn.refused.is_none(), "{only_isbn:?}");
    }
}
