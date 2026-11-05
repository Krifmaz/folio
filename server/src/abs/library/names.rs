//! People's names, the way the original reads them: `parseFullName` (a copy of the parse-full-name package in
//! `server/utils/parsers/parseFullName.js`) and `parseNameString` (`server/utils/parsers/parseNameString.js`),
//! which turns an author or narrator field into separate names and gives "Last, First" for sorting.
//!
//! The rules are kept exactly, odd corners included, so that names sort and match the same way.

/// A name split into its parts, as `parseFullName` answers it. Every part is empty when not found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FullName {
    /// A title before the name, such as "Dr".
    pub title: String,
    /// The first name.
    pub first: String,
    /// Every name between the first and the last.
    pub middle: String,
    /// The last name, with its prefixes ("van der Berg").
    pub last: String,
    /// A nickname written in quotes or brackets.
    pub nick: String,
    /// A suffix such as "Jr." or "PhD".
    pub suffix: String,
}

/// Joining words that hold two names together ("Bill and Melinda").
const CONJUNCTIONS: &[&str] = &["&", "and", "et", "e", "of", "the", "und", "y"];
/// Suffixes recognised after a name (the short list).
const SUFFIXES: &[&str] = &[
    "esq", "esquire", "jr", "jnr", "sr", "snr", "2", "ii", "iii", "iv", "md", "phd", "j.d.", "ll.m.", "m.d.", "d.o.", "d.c.", "p.c.",
    "ph.d.",
];
/// Words that belong to the following last name (the short list).
const PREFIXES: &[&str] = &[
    "ab", "bar", "bin", "da", "dal", "de", "de la", "del", "della", "der", "di", "du", "ibn", "l'", "la", "le", "san", "st", "st.", "ste",
    "ter", "van", "van de", "van der", "van den", "vel", "ver", "vere", "von",
];
/// Titles recognised before a name (the short list).
const TITLES: &[&str] = &[
    "dr",
    "miss",
    "mr",
    "mrs",
    "ms",
    "prof",
    "sir",
    "frau",
    "herr",
    "hr",
    "monsieur",
    "captain",
    "doctor",
    "judge",
    "officer",
    "professor",
    "ind",
    "misc",
    "mx",
];
/// Words whose letter case is always written this way when a name's case is fixed.
const FORCE_CASE: &[&str] = &[
    "e", "y", "av", "af", "da", "dal", "de", "del", "der", "di", "la", "le", "van", "der", "den", "vel", "von", "II", "III", "IV", "J.D.",
    "LL.M.", "M.D.", "D.O.", "D.C.", "Ph.D.",
];

/// The first `n` characters of a string.
fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Everything after the first `n` characters.
fn tail(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

/// The string without its last `n` characters (JavaScript's `slice(0, -n)`).
fn drop_last(s: &str, n: usize) -> String {
    let len = s.chars().count();
    s.chars().take(len.saturating_sub(n)).collect()
}

/// The last character, or an empty string.
fn last_char(s: &str) -> String {
    s.chars().last().map(String::from).unwrap_or_default()
}

/// The nickname matches of `regex = /\s(?:‘..’|“..”|\[..\]|\(..\)),?\s/g` on a string, as whole matches.
fn nickname_matches(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = nickname_at(&chars, i) {
            found.push(chars[i..end].iter().collect());
            i = end;
        } else {
            i += 1;
        }
    }
    found
}

/// Where a nickname match starting at `i` ends (exclusive), if one starts there.
fn nickname_at(chars: &[char], i: usize) -> Option<usize> {
    if !chars.get(i)?.is_whitespace() {
        return None;
    }
    let open = *chars.get(i + 1)?;
    let closes: &[char] = match open {
        '‘' | '’' | '\'' => &['‘', '’', '\''],
        '“' | '”' | '"' => &['“', '”', '"'],
        '[' => &[']'],
        '(' => &[')'],
        _ => return None,
    };
    let mut j = i + 2;
    while j < chars.len() && !closes.contains(&chars[j]) {
        j += 1;
    }
    if j == i + 2 || j >= chars.len() {
        return None;
    }
    let mut k = j + 1;
    if chars.get(k) == Some(&',') && chars.get(k + 1).is_some_and(|c| c.is_whitespace()) {
        k += 1;
    }
    chars.get(k).filter(|c| c.is_whitespace())?;
    Some(k + 1)
}

/// Fixes the letter case of every part, as the original does for names written all in upper or lower case.
fn fix_case(name: &mut FullName) {
    let labels = ["title", "first", "middle", "last", "nick", "suffix"];
    let parts: [&mut String; 6] = [&mut name.title, &mut name.first, &mut name.middle, &mut name.last, &mut name.nick, &mut name.suffix];
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let mut words: Vec<String> = part.split(' ').map(String::from).collect();
        for (j, word) in words.iter_mut().enumerate() {
            let lower = word.to_lowercase();
            let len = word.chars().count();
            let (c0, c1, rest) = (head(word, 1), tail(&head(word, 2), 1), tail(word, 2));
            if let Some(forced) = FORCE_CASE.iter().find(|v| v.to_lowercase() == lower) {
                *word = (*forced).to_string();
            } else if len == 1 {
                *word = word.to_uppercase();
            } else if len > 2 && c0 == c0.to_uppercase() && c1 == c1.to_lowercase() && rest == rest.to_uppercase() {
                *word = format!("{}{}", head(word, 3), tail(word, 3).to_lowercase());
            } else if labels.get(j) == Some(&"suffix") && last_char(word) != "." && SUFFIXES.iter().position(|s| *s == lower) == Some(0) {
                if *word == lower {
                    *word = word.to_uppercase();
                }
            } else {
                *word = format!("{}{}", c0.to_uppercase(), tail(word, 1).to_lowercase());
            }
        }
        *part = words.join(" ");
    }
}

/// Splits a full name into title, first, middle, last, nickname and suffix (`parseFullName(name)` with its
/// defaults: case fixed only for names written all in one case, errors kept quiet, the short lists).
pub fn parse_full_name(name: &str) -> FullName {
    let mut parsed = FullName::default();
    let mut text = name.trim().to_string();
    if name.is_empty() {
        return parsed;
    }
    let fix = text == text.to_uppercase() || text == text.to_lowercase();
    let done = |mut p: FullName| {
        if fix {
            fix_case(&mut p);
        }
        p
    };

    // Nicknames: parts in quotes or brackets.
    let padded = format!(" {text} ");
    let found = nickname_matches(&padded);
    if found.len() == 1 {
        let mut nick = drop_last(&tail(&found[0], 2), 2);
        if last_char(&nick) == "," {
            nick = drop_last(&nick, 1);
        }
        parsed.nick = nick;
        text = padded.replacen(&found[0], " ", 1).trim().to_string();
    } else if found.len() > 1 {
        let mut nicks = Vec::new();
        for f in &found {
            text = format!(" {text} ").replacen(f, " ", 1).trim().to_string();
            let mut n = drop_last(&tail(f, 2), 2);
            if last_char(&n) == "," {
                n = drop_last(&n, 1);
            }
            nicks.push(n);
        }
        parsed.nick = nicks.join(", ");
    }
    if text.trim().is_empty() {
        return done(parsed);
    }

    // The words, each with the comma written after the one before it.
    let mut parts: Vec<String> = Vec::new();
    let mut commas: Vec<Option<char>> = vec![None];
    for word in text.split(' ') {
        if last_char(word) == "," {
            parts.push(drop_last(word, 1));
            commas.push(Some(','));
        } else {
            parts.push(word.to_string());
            commas.push(None);
        }
    }
    let plain = |w: &str| if last_char(w) == "." { drop_last(w, 1).to_lowercase() } else { w.to_lowercase() };
    let listed = |list: &[&str], w: &str| list.contains(&w) || list.contains(&format!("{w}.").as_str());

    // Suffixes, from the end (never the first word).
    let mut taken: Vec<String> = Vec::new();
    let mut i = parts.len();
    while i > 1 {
        i -= 1;
        if listed(SUFFIXES, &plain(&parts[i])) {
            taken.insert(0, parts.remove(i));
            if commas[i] == Some(',') {
                commas.remove(i + 1);
            } else {
                commas.remove(i);
            }
        }
    }
    if !taken.is_empty() {
        parsed.suffix = taken.join(", ");
        taken.clear();
    }
    if parts.is_empty() {
        return done(parsed);
    }

    // Titles, from the end.
    let mut i = parts.len();
    while i > 0 {
        i -= 1;
        if listed(TITLES, &plain(&parts[i])) {
            taken.insert(0, parts.remove(i));
            if commas[i] == Some(',') {
                commas.remove(i + 1);
            } else {
                commas.remove(i);
            }
        }
    }
    if !taken.is_empty() {
        parsed.title = taken.join(", ");
        taken.clear();
    }
    if parts.is_empty() {
        return done(parsed);
    }

    // Prefixes join the name after them.
    if parts.len() > 1 {
        let mut i = parts.len() - 1;
        while i > 0 {
            i -= 1;
            if PREFIXES.contains(&parts[i].to_lowercase().as_str()) {
                let next = parts.remove(i + 1);
                parts[i] = format!("{} {next}", parts[i]);
                commas.remove(i + 1);
            }
        }
    }

    // Conjunctions join the names around them.
    if parts.len() > 2 {
        let mut i = parts.len() as isize - 3;
        while i >= 0 {
            let u = i as usize;
            if u + 2 < parts.len() && CONJUNCTIONS.contains(&parts[u + 1].to_lowercase().as_str()) {
                let after = parts.remove(u + 2);
                let and = parts.remove(u + 1);
                parts[u] = format!("{} {and} {after}", parts[u]);
                commas.drain(u + 1..(u + 3).min(commas.len()));
                i -= 1;
            }
            i -= 1;
        }
    }

    // Words after extra commas are suffixes.
    commas.pop();
    let first_comma = commas.iter().position(|c| c.is_some()).map_or(-1, |p| p as isize);
    let mut remaining = commas.iter().filter(|c| c.is_some()).count();
    if first_comma > 1 || remaining > 1 {
        let mut i = parts.len();
        while i > 2 {
            i -= 1;
            if commas.get(i) == Some(&Some(',')) {
                taken.insert(0, parts.remove(i));
                commas.remove(i);
                remaining -= 1;
            } else {
                break;
            }
        }
    }
    if !taken.is_empty() {
        if !parsed.suffix.is_empty() {
            taken.insert(0, parsed.suffix.clone());
        }
        parsed.suffix = taken.join(", ");
    }

    // The last name: everything before the first comma, or else the last word.
    if remaining > 0 {
        if let Some(at) = commas.iter().position(|c| c.is_some()).filter(|at| *at != 0) {
            let at = at.min(parts.len());
            parsed.last = parts.drain(..at).collect::<Vec<_>>().join(" ");
            commas.drain(..at.min(commas.len()));
        }
    } else {
        parsed.last = parts.pop().unwrap_or_default();
    }
    if parts.is_empty() {
        return done(parsed);
    }

    parsed.first = parts.remove(0);
    if parts.is_empty() {
        return done(parsed);
    }
    parsed.middle = parts.join(" ");
    done(parsed)
}

/// A name as first and last, the middle names joined to the first (`parseName` in parseNameString.js).
fn parse_name(name: &str) -> (String, String) {
    let parts = parse_full_name(name);
    let mut first = parts.first;
    if !first.is_empty() && !parts.middle.is_empty() {
        first = format!("{first} {}", parts.middle);
    }
    (first, parts.last)
}

/// Whether a piece of a comma-separated list is a last name on its own ("Smith" in "Smith, John").
fn is_last_name(name: &str) -> bool {
    !name.contains(' ') || parse_full_name(name).first.is_empty()
}

/// "First Last" as "Last, First" (`nameToLastFirst`).
pub fn name_to_last_first(first_last: &str) -> String {
    let (first, last) = parse_name(first_last);
    if last.is_empty() {
        first
    } else if first.is_empty() {
        last
    } else {
        format!("{last}, {first}")
    }
}

/// Whether a character is Chinese, Japanese or Korean script, whose names are kept as written.
fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3040}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}')
}

/// The separate names in an author or narrator field, each as "First Last", without repeats (`parse`).
/// Handles "A & B", "A and B", "A; B", "Last, First, Last, First" and "First Last, First Last".
pub fn parse_names(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let comma_list = text.contains(',');
    let split_on = |sep: &str| text.split(sep).flat_map(|p| p.split(',')).map(String::from).collect::<Vec<_>>();
    let mut split: Vec<String> = if text.contains('&') {
        split_on("&")
    } else if text.contains(" and ") {
        split_on(" and ")
    } else if text.contains(';') {
        split_on(";")
    } else {
        text.split(',').map(String::from).collect()
    };
    split = split.iter().map(|s| s.trim().to_string()).collect();
    if split.first().is_some_and(|s| s.chars().any(is_cjk)) {
        return split;
    }
    let mut names: Vec<(String, String)> = Vec::new();
    if split.len() == 1 {
        names.push(parse_name(text));
    } else {
        let last_first = comma_list && is_last_name(&split[0]);
        if split.len() % 2 == 1 && last_first {
            split.pop();
        }
        if last_first {
            for pair in split.chunks(2) {
                names.push((pair.get(1).cloned().unwrap_or_default(), pair[0].clone()));
            }
        } else {
            names.extend(split.iter().map(|s| parse_name(s)));
        }
    }
    let mut out: Vec<String> = Vec::new();
    for (first, last) in names.into_iter().filter(|(f, l)| !f.is_empty() || !l.is_empty()) {
        let full = if first.is_empty() { last } else { format!("{first} {last}") };
        if !out.contains(&full) {
            out.push(full);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_first_like_the_original() {
        assert_eq!(name_to_last_first("Becky Chambers"), "Chambers, Becky");
        assert_eq!(name_to_last_first("Ann Leckie"), "Leckie, Ann");
        assert_eq!(name_to_last_first("Ursula K. Le Guin"), "Le Guin, Ursula K.");
        assert_eq!(name_to_last_first("Ludwig van Beethoven"), "van Beethoven, Ludwig");
        assert_eq!(name_to_last_first("Plato"), "Plato");
        assert_eq!(name_to_last_first("Martin Luther King Jr."), "King, Martin Luther");
        assert_eq!(name_to_last_first("Dr. John Smith"), "Smith, John");
        assert_eq!(name_to_last_first("Smith, John"), "Smith, John");
        assert_eq!(name_to_last_first("JOHN SMITH"), "Smith, John");
        assert_eq!(name_to_last_first("Bill and Melinda Gates"), "Gates, Bill and Melinda");
        assert_eq!(name_to_last_first("Robert 'Bob' Jones"), "Jones, Robert");
    }

    #[test]
    fn full_name_parts() {
        let n = parse_full_name("Dr. John 'Jack' Q. Public, Jr.");
        assert_eq!((n.title.as_str(), n.first.as_str(), n.middle.as_str(), n.last.as_str()), ("Dr.", "John", "Q.", "Public"));
        assert_eq!((n.nick.as_str(), n.suffix.as_str()), ("Jack", "Jr."));
        assert_eq!(parse_full_name(""), FullName::default());
    }

    #[test]
    fn name_lists() {
        assert_eq!(parse_names("Jon Smith & Jane Smith"), vec!["Jon Smith", "Jane Smith"]);
        assert_eq!(parse_names("Smith, Jon, Doe, Jane"), vec!["Jon Smith", "Jane Doe"]);
        assert_eq!(parse_names("Jon Smith, Jane Doe"), vec!["Jon Smith", "Jane Doe"]);
        assert_eq!(parse_names("Jon Smith and Jane Doe"), vec!["Jon Smith", "Jane Doe"]);
        assert_eq!(parse_names("Adjoa Andoh"), vec!["Adjoa Andoh"]);
        assert_eq!(parse_names("村上春樹, 柴田元幸"), vec!["村上春樹", "柴田元幸"]);
        assert!(parse_names("").is_empty());
    }
}
