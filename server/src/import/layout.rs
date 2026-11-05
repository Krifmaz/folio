//! Where a book goes in the library: `Author/Series/NN - Title (Year)/`, or `Author/Title (Year)/` outside a
//! series, with names safe on every file system.

use crate::library::text;
use crate::metadata::EditionMeta;
use std::path::PathBuf;

/// Longest folder or file name kept (bytes, before the extension).
const MAX_NAME: usize = 120;

/// A name safe as one folder or file name everywhere: no slashes or characters Windows refuses, no control
/// characters, no leading or trailing dots and spaces, at most [`MAX_NAME`] bytes. "Unknown" when nothing is left.
pub fn safe(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '|' => '-',
            ':' => ' ',
            '*' | '?' | '"' | '<' | '>' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    while s.len() > MAX_NAME {
        s.pop();
    }
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if s.is_empty() {
        "Unknown".into()
    } else {
        s
    }
}

/// A place in a series as the folder starts with it: two digits for a whole number (`"1"` → `01`), the
/// decimal kept (`"2.5"` → `02.5`), anything else as written.
pub fn position(p: &str) -> String {
    let p = p.trim();
    let (whole, rest) = p.split_once('.').map(|(a, b)| (a, format!(".{b}"))).unwrap_or((p, String::new()));
    match whole.parse::<u32>() {
        Ok(n) if rest.is_empty() || rest[1..].chars().all(|c| c.is_ascii_digit()) => format!("{n:02}{rest}"),
        _ => p.to_string(),
    }
}

/// The book's folder under the library, and the name (without extension) of its main file.
pub fn place(m: &EditionMeta) -> (PathBuf, String) {
    let author = m.authors.first().map(|a| a.name.clone()).unwrap_or_default();
    let title = m.title.trim().to_string();
    let year = text::year(&m.release_date).map(|y| format!(" ({y})")).unwrap_or_default();
    let mut dir = PathBuf::from(safe(if author.is_empty() { "Unknown author" } else { &author }));
    let book = match m.series.first().filter(|s| !s.name.trim().is_empty()) {
        Some(s) => {
            dir.push(safe(&s.name));
            let pos = position(&s.position);
            if pos.is_empty() {
                format!("{title}{year}")
            } else {
                format!("{pos} - {title}{year}")
            }
        }
        None => format!("{title}{year}"),
    };
    dir.push(safe(&book));
    (dir, safe(&title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Person, SeriesRef};

    fn book(series: Option<(&str, &str)>) -> EditionMeta {
        EditionMeta {
            title: "Project Hail Mary".into(),
            authors: vec![Person { name: "Andy Weir".into(), ..Default::default() }],
            release_date: "2021-05-04".into(),
            series: series.map(|(n, p)| vec![SeriesRef { name: n.into(), position: p.into(), asin: String::new() }]).unwrap_or_default(),
            ..Default::default()
        }
    }

    #[test]
    fn books_are_filed_by_author_series_and_place() {
        assert_eq!(place(&book(None)), (PathBuf::from("Andy Weir/Project Hail Mary (2021)"), "Project Hail Mary".into()));
        assert_eq!(place(&book(Some(("The Expanse", "2")))).0, PathBuf::from("Andy Weir/The Expanse/02 - Project Hail Mary (2021)"));
        assert_eq!(place(&book(Some(("Saga", "")))).0, PathBuf::from("Andy Weir/Saga/Project Hail Mary (2021)"));
    }

    #[test]
    fn names_are_safe_everywhere() {
        assert_eq!(safe("AC/DC: Live? <1990>"), "AC-DC Live 1990");
        assert_eq!(safe("..."), "Unknown");
        assert_eq!(safe(" Dune. "), "Dune");
        assert!(safe(&"x".repeat(300)).len() <= MAX_NAME);
        assert_eq!(position("1"), "01");
        assert_eq!(position("2.5"), "02.5");
        assert_eq!(position("1-3"), "1-3");
        assert_eq!(position("12"), "12");
    }
}
