//! Text rules for matching books across sources: the keys that say two titles or two author names are the same
//! book or person, language names and codes in one form, and years from release dates.

use unicode_normalization::UnicodeNormalization;

/// Lower case, accents removed, every run of other characters one space (`"Café: The Book!"` → `"cafe the book"`).
pub fn fold(s: &str) -> String {
    let plain: String = s.nfkd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).collect::<String>().to_lowercase();
    plain.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The key two titles of one work share: the title before a subtitle (`:`, ` - `, `(`), folded, without a
/// leading "the", "a" or "an". `"The Way of Kings: The Stormlight Archive, Book 1"` → `"way of kings"`.
pub fn title_key(title: &str) -> String {
    let cut = [":", " - ", " – ", "(", "["].iter().filter_map(|m| title.find(m)).filter(|&i| i > 0).min().unwrap_or(title.len());
    let f = fold(&title[..cut]);
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = f.strip_prefix(article) {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    f
}

/// The key two spellings of one person share: "Last, First" turned round, then only the letters and digits
/// (`"J. R. R. Tolkien"`, `"J.R.R. Tolkien"` and `"Tolkien, J.R.R."` → `"jrrtolkien"`).
pub fn person_key(name: &str) -> String {
    let name = match name.split_once(',') {
        Some((last, first)) if !first.trim().is_empty() && !last.contains(' ') => format!("{} {}", first.trim(), last.trim()),
        _ => name.to_string(),
    };
    fold(&name).replace(' ', "")
}

/// A person's name for sorting: "Last, First" (`"Brandon Sanderson"` → `"Sanderson, Brandon"`). One-word names
/// and names already with a comma stay as they are.
pub fn sort_name(name: &str) -> String {
    let name = name.trim();
    if name.contains(',') {
        return name.to_string();
    }
    match name.rsplit_once(' ') {
        Some((first, last)) => format!("{last}, {first}"),
        None => name.to_string(),
    }
}

/// Languages as two-letter codes (ISO 639-1), from names (`"english"`), two-letter and three-letter codes
/// (`"en"`, `"eng"`, `"/languages/eng"`) and regional forms (`"en-US"`, `"en_GB"`). A language not in the table
/// is kept folded, so it still compares equal to itself.
pub fn language(raw: &str) -> String {
    const TABLE: &[(&str, &str, &str, &str)] = &[
        ("en", "eng", "english", ""),
        ("de", "ger", "german", "deu deutsch"),
        ("fr", "fre", "french", "fra francais"),
        ("es", "spa", "spanish", "espanol castilian"),
        ("it", "ita", "italian", "italiano"),
        ("nl", "dut", "dutch", "nld nederlands"),
        ("pt", "por", "portuguese", "portugues"),
        ("sv", "swe", "swedish", "svenska"),
        ("da", "dan", "danish", "dansk"),
        ("no", "nor", "norwegian", "nob nno norsk bokmal"),
        ("fi", "fin", "finnish", "suomi"),
        ("pl", "pol", "polish", "polski"),
        ("ru", "rus", "russian", ""),
        ("ja", "jpn", "japanese", ""),
        ("zh", "chi", "chinese", "zho mandarin"),
        ("ko", "kor", "korean", ""),
        ("cs", "cze", "czech", "ces"),
        ("hu", "hun", "hungarian", ""),
        ("tr", "tur", "turkish", ""),
        ("el", "gre", "greek", "ell"),
        ("he", "heb", "hebrew", ""),
        ("ar", "ara", "arabic", ""),
        ("hi", "hin", "hindi", ""),
        ("uk", "ukr", "ukrainian", ""),
        ("ro", "rum", "romanian", "ron"),
        ("ca", "cat", "catalan", ""),
        ("af", "afr", "afrikaans", ""),
    ];
    let t = raw.trim().rsplit('/').next().unwrap_or("").to_lowercase();
    let t = t.split(['-', '_']).next().unwrap_or("").trim().to_string();
    let f = fold(&t);
    for (two, three, name, more) in TABLE {
        if f == *two || f == *three || f == *name || more.split(' ').any(|m| !m.is_empty() && m == f) {
            return two.to_string();
        }
    }
    f
}

/// The year at the start of a date (`"2020-11-17T00:00:00.000Z"`, `"2020"`) or anywhere in it (`"Nov 17, 2020"`).
pub fn year(date: &str) -> Option<i32> {
    let digits: Vec<&str> = date.split(|c: char| !c.is_ascii_digit()).filter(|w| w.len() == 4).collect();
    digits.first().and_then(|y| y.parse().ok()).filter(|y| (1000..3000).contains(y))
}

/// A release date as `YYYY-MM-DD` (or the year only) from the forms the sources use: ISO dates and times,
/// `"November 17, 2020"`, `"Nov 2020"`, `"2020"`.
pub fn date(raw: &str) -> String {
    let raw = raw.trim();
    if raw.len() >= 10 && raw.as_bytes()[4] == b'-' && raw.as_bytes()[7] == b'-' {
        return raw[..10].to_string();
    }
    if raw.len() == 7 && raw.as_bytes()[4] == b'-' {
        return raw.to_string();
    }
    year(raw).map(|y| y.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_share_a_key_without_subtitles_and_articles() {
        assert_eq!(title_key("The Way of Kings: The Stormlight Archive, Book 1"), "way of kings");
        assert_eq!(title_key("The Way of Kings (Stormlight Archive #1)"), "way of kings");
        assert_eq!(title_key("Way of Kings"), "way of kings");
        assert_eq!(title_key("Café Society!"), "cafe society");
        assert_eq!(title_key("The"), "the");
        assert_eq!(title_key("A"), "a");
    }

    #[test]
    fn people_share_a_key_however_written() {
        assert_eq!(person_key("J. R. R. Tolkien"), "jrrtolkien");
        assert_eq!(person_key("J.R.R. Tolkien"), "jrrtolkien");
        assert_eq!(person_key("Tolkien, J.R.R."), "jrrtolkien");
        assert_eq!(person_key("Gabriel García Márquez"), "gabrielgarciamarquez");
        assert_eq!(sort_name("Brandon Sanderson"), "Sanderson, Brandon");
        assert_eq!(sort_name("Plato"), "Plato");
    }

    #[test]
    fn languages_come_out_as_two_letters() {
        for (raw, want) in [
            ("english", "en"),
            ("English", "en"),
            ("eng", "en"),
            ("/languages/eng", "en"),
            ("en-US", "en"),
            ("en_GB", "en"),
            ("ger", "de"),
            ("deu", "de"),
            ("German", "de"),
            ("español", "es"),
            ("klingon", "klingon"),
        ] {
            assert_eq!(language(raw), want, "{raw}");
        }
    }

    #[test]
    fn dates_and_years() {
        assert_eq!(date("2020-11-17T00:00:00.000Z"), "2020-11-17");
        assert_eq!(date("November 17, 2020"), "2020");
        assert_eq!(date("2020"), "2020");
        assert_eq!(date("2020-11"), "2020-11");
        assert_eq!(date(""), "");
        assert_eq!(year("Nov 17, 2020"), Some(2020));
        assert_eq!(year("n.d."), None);
    }
}
