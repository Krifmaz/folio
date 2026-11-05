//! Readarr's text helpers for names and titles, ported as they are: spaces (`CleanSpaces`), "Last, First"
//! (`ToLastFirst`, itself ported from calibre), bracketed text, accents, the clean names Readarr matches by
//! (`Parser.CleanAuthorName`) and the title forms its naming tokens use (`FileNameBuilder.CleanTitle`,
//! `TitleThe`).

use fancy_regex::Regex as FancyRegex;
use regex::Regex;
use std::sync::LazyLock;

/// `\s+`, which `CleanSpaces` collapses to one space.
static COLLAPSE_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("a valid pattern"));

/// The words and connectors `CleanAuthorName` leaves out, and every character that isn't a letter or digit
/// (`Parser.NormalizeRegex`).
static NORMALIZE: LazyLock<FancyRegex> =
    LazyLock::new(|| FancyRegex::new(r"(?i)((?:\b|_)(?<!^)(a(?!$)|an|the|and|or|of)(?!$)(?:\b|_))|\W|_").expect("a valid pattern"));

/// The characters `CleanTitle` removes (`FileNameBuilder.ScenifyRemoveChars`); `$` and `^` in the list are
/// anchors there, as in .NET.
static SCENIFY_REMOVE: LazyLock<FancyRegex> = LazyLock::new(|| {
    FancyRegex::new(
        r#"(?i)(?<=\s)(,|<|>|\/|\\|;|:|'|"|\||`|~|!|\?|@|$|%|^|\*|-|_|=){1}(?=\s)|('|:|\?|,)(?=(?:(?:s|m)\s)|\s|$)|(\(|\)|\[|\]|\{|\})"#,
    )
    .expect("a valid pattern")
});

/// A leading "The", "An" or "A" and what follows it, with any bracketed endings (`TitlePrefixRegex`).
static TITLE_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(The|An|A) (.*?)((?: *\([^)]+\))*)$").expect("a valid pattern"));

/// Words that mark a name as a company or group, which `ToLastFirst` leaves as it is.
const COPYWORDS: &[&str] =
    &["agency", "corporation", "company", "co.", "council", "committee", "inc.", "institute", "national", "society", "club", "team"];

/// Surname prefixes that belong to the last name ("van Gogh").
const SURNAME_PREFIXES: &[&str] = &["da", "de", "di", "la", "le", "van", "von"];

/// Titles before a name.
const PREFIXES: &[&str] = &["mr", "mr.", "mrs", "mrs.", "ms", "ms.", "dr", "dr.", "prof", "prof."];

/// Endings after a name.
const SUFFIXES: &[&str] = &["jr", "sr", "inc", "ph.d", "phd", "md", "m.d", "i", "ii", "iii", "iv", "junior", "senior"];

/// Runs of white space become one space, and the ends are trimmed (`CleanSpaces`).
pub fn clean_spaces(text: &str) -> String {
    COLLAPSE_SPACE.replace_all(text, " ").trim().to_string()
}

/// The text without anything in brackets, `()`, `[]` or `{}`, nested ones too (`RemoveBracketedText`). A closing
/// bracket without its opening one is dropped.
pub fn remove_bracketed_text(input: &str) -> String {
    let mut counts = [0i32; 3];
    let mut total = 0;
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        let open = ['(', '[', '{'].iter().position(|&b| b == c);
        let close = [')', ']', '}'].iter().position(|&b| b == c);
        if let Some(i) = open {
            counts[i] += 1;
            total += 1;
        } else if let Some(i) = close {
            if counts[i] > 0 {
                counts[i] -= 1;
                total -= 1;
            }
        } else if total < 1 {
            out.push(c);
        }
    }
    out
}

/// A name as "Last, First" (`ToLastFirst`, from calibre's `author_to_author_sort`): "Terry Pratchett" gives
/// "Pratchett, Terry", "Ursula K. Le Guin" gives "Le Guin, Ursula K."; one-word names and company names stay as
/// they are.
pub fn to_last_first(author: &str) -> String {
    let sauthor = remove_bracketed_text(author);
    let sauthor = sauthor.trim();
    // .NET's `Split()` splits at every white-space character and keeps the empty parts.
    let mut tokens: Vec<String> = sauthor.split(char::is_whitespace).map(str::to_string).collect();
    if tokens.len() < 2 {
        return author.to_string();
    }
    let lower = |s: &str| s.to_lowercase();
    if tokens.iter().any(|t| COPYWORDS.contains(&lower(t).as_str())) {
        return author.to_string();
    }
    if tokens.len() == 2 && SURNAME_PREFIXES.contains(&lower(&tokens[0]).as_str()) {
        return author.to_string();
    }
    let Some(first) = tokens.iter().position(|t| !PREFIXES.contains(&lower(t).as_str())) else {
        return author.to_string();
    };
    let Some(mut last) = (first..tokens.len()).rev().find(|&i| !SUFFIXES.contains(&lower(&tokens[i]).as_str())) else {
        return author.to_string();
    };
    let suffix = tokens[last + 1..].join(" ");
    if last > first && SURNAME_PREFIXES.contains(&lower(&tokens[last - 1]).as_str()) {
        tokens[last - 1] = format!("{} {}", tokens[last - 1], tokens[last]);
        last -= 1;
    }
    let mut atokens = vec![tokens[last].clone()];
    atokens.extend(tokens[first..last].iter().cloned());
    let add_comma = atokens.len() > 1;
    if !suffix.trim().is_empty() {
        atokens.push(suffix);
    }
    if add_comma {
        atokens[0].push(',');
    }
    atokens.join(" ")
}

/// The text without accents (`RemoveAccent`): decomposed, the combining marks dropped, composed again.
/// Readarr drops only non-spacing marks; this drops every combining mark, which differs only for the spacing and
/// enclosing marks of a few scripts.
pub fn remove_accent(text: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    text.nfd().filter(|c| !is_combining_mark(*c)).nfc().collect()
}

/// `(?<=\b\d+)%` replaced by "percent": a percent sign right after a number that starts a word.
fn percent_words(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '%' && i > 0 && chars[i - 1].is_numeric() {
            let mut start = i - 1;
            while start > 0 && chars[start - 1].is_numeric() {
                start -= 1;
            }
            if start == 0 || !(chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
                out.push_str("percent");
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// The clean name Readarr matches authors and books by (`Parser.CleanAuthorName`): lower case, without accents,
/// spaces, punctuation and the small words "a", "an", "the", "and", "or" and "of" inside it. A name that is a
/// number stays as it is; nothing gives nothing.
pub fn clean_author_name(name: &str) -> String {
    if name.trim().is_empty() {
        return String::new();
    }
    if name.parse::<i64>().is_ok() {
        return name.to_string();
    }
    let name = percent_words(name);
    let cleaned = NORMALIZE.replace_all(&name, "");
    remove_accent(&cleaned.to_lowercase())
}

/// A title for a file name the scene way (`FileNameBuilder.CleanTitle`): "&" becomes "and", "/" becomes a space
/// and stray punctuation and brackets are removed.
pub fn clean_title(title: &str) -> String {
    let title = title.replace('&', "and").replace('/', " ");
    SCENIFY_REMOVE.replace_all(&title, "").into_owned()
}

/// A title with a leading "The", "An" or "A" moved to the end (`TitleThe`): "The Colour of Magic" gives
/// "Colour of Magic, The".
pub fn title_the(title: &str) -> String {
    TITLE_PREFIX.replace(title, "$2, $1$3").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_out_like_readarr() {
        assert_eq!(clean_spaces("  Terry  Pratchett \t"), "Terry Pratchett");
        assert_eq!(remove_bracketed_text("Mort (Discworld, #4) [x]"), "Mort  ");
        assert_eq!(to_last_first("Terry Pratchett"), "Pratchett, Terry");
        assert_eq!(to_last_first("Ursula K. Le Guin"), "Le Guin, Ursula K.");
        assert_eq!(to_last_first("Dr. Martin Luther King Jr"), "King, Martin Luther Jr");
        assert_eq!(to_last_first("Plato"), "Plato");
        assert_eq!(to_last_first("Van Gogh"), "Van Gogh");
        assert_eq!(to_last_first("The Folio Society"), "The Folio Society");
        assert_eq!(clean_author_name("Terry Pratchett"), "terrypratchett");
        assert_eq!(clean_author_name("Ursula K. Le Guin"), "ursulakleguin");
        assert_eq!(clean_author_name("The Lord of the Rings"), "thelordrings");
        assert_eq!(clean_author_name("Guards! Guards!"), "guardsguards");
        assert_eq!(clean_author_name("100% Brontë"), "100percentbronte");
        assert_eq!(clean_author_name("1984"), "1984");
        assert_eq!(clean_author_name(" "), "");
        assert_eq!(clean_title("Tom & Jerry: The (Movie)"), "Tom and Jerry The Movie");
        assert_eq!(title_the("The Colour of Magic (Discworld)"), "Colour of Magic, The (Discworld)");
        assert_eq!(title_the("Mort"), "Mort");
    }
}
