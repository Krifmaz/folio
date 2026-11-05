//! Small pieces the library side shares: numbers and times as the original's JSON writes them, keys that
//! JavaScript leaves out when they are `undefined`, the server's sorting settings, and title prefixes.

use crate::abs::models::date_ms;
use crate::abs::settings::ServerSettings;
use serde_json::{Map, Value};

/// A number as JavaScript writes it in JSON: whole numbers without a fraction (`45`, not `45.0`), `NaN` and
/// infinities as `null`.
pub fn js_num(n: f64) -> Value {
    if !n.is_finite() {
        Value::Null
    } else if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

/// An optional number as the original writes it: `null` when missing.
pub fn js_num_opt(n: Option<f64>) -> Value {
    n.map_or(Value::Null, js_num)
}

/// A stored time as milliseconds since 1970 (`date.valueOf()`), `None` when there is no time.
pub fn ms(date: &Option<String>) -> Option<i64> {
    date.as_deref().and_then(date_ms)
}

/// A stored time as milliseconds, `0` when missing (for times the original always has, like `createdAt`).
pub fn ms0(date: &Option<String>) -> Value {
    Value::from(ms(date).unwrap_or(0))
}

/// Sets `key` only when there is a value: JavaScript drops keys whose value is `undefined` from JSON.
pub fn put_some(obj: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(v) = value {
        obj.insert(key.to_string(), v);
    }
}

/// A copy of `keys` from a stored object, leaving out the ones it doesn't have (`undefined` in the original).
pub fn pick(from: &Value, keys: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for k in keys {
        if let Some(v) = from.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    out
}

/// A stored JSON array, or an empty one (`[...(this.x || [])]`).
pub fn array(v: &Option<Value>) -> Value {
    match v {
        Some(Value::Array(a)) => Value::Array(a.clone()),
        _ => Value::Array(Vec::new()),
    }
}

/// A stored JSON value as written, `null` when missing (`structuredClone(this.x)`).
pub fn clone_or_null(v: &Option<Value>) -> Value {
    v.clone().unwrap_or(Value::Null)
}

/// An optional string as JSON, `null` when missing.
pub fn s(v: &Option<String>) -> Value {
    v.clone().map_or(Value::Null, Value::from)
}

/// An optional flag as JSON, `null` when missing.
pub fn b(v: Option<bool>) -> Value {
    v.map_or(Value::Null, Value::from)
}

/// The server settings that change how titles sort.
#[derive(Debug, Clone)]
pub struct Sorting {
    /// `sortingIgnorePrefix`: sort "The Book" under B.
    pub ignore_prefix: bool,
    /// `sortingPrefixes`: the words moved to the end, in lower case (default "the" and "a").
    pub prefixes: Vec<String>,
}

impl Default for Sorting {
    fn default() -> Self {
        Self { ignore_prefix: false, prefixes: vec!["the".into(), "a".into()] }
    }
}

impl Sorting {
    /// The sorting settings in effect (`global.ServerSettings`).
    pub fn of(settings: &ServerSettings) -> Sorting {
        let prefixes = match settings.get("sortingPrefixes").and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).map(String::from).collect(),
            None => vec!["the".into(), "a".into()],
        };
        Sorting { ignore_prefix: settings.get("sortingIgnorePrefix").and_then(Value::as_bool).unwrap_or(false), prefixes }
    }
}

/// A title split into the part to sort by and the prefix moved away (`getTitleParts`).
fn title_parts(title: &str, sorting: &Sorting) -> (String, Option<String>) {
    let lower = title.to_lowercase();
    for p in &sorting.prefixes {
        if lower.starts_with(&format!("{p} ")) {
            let rest: String = title.chars().skip(p.chars().count() + 1).collect();
            let mut cs = p.chars();
            let cap = cs.next().map(|c| c.to_uppercase().collect::<String>() + cs.as_str()).unwrap_or_default();
            return (rest, Some(cap));
        }
    }
    (title.to_string(), None)
}

/// "The Good Book" as "Good Book, The" (`getTitlePrefixAtEnd`).
pub fn title_prefix_at_end(title: &str, sorting: &Sorting) -> String {
    match title_parts(title, sorting) {
        (sort, Some(prefix)) => format!("{sort}, {prefix}"),
        _ => title.to_string(),
    }
}

/// "The Good Book" as "Good Book" (`getTitleIgnorePrefix`).
pub fn title_ignore_prefix(title: &str, sorting: &Sorting) -> String {
    title_parts(title, sorting).0
}

/// A filter value from a library page's `filter=group.<value>`: URI-decoded, then base64 (`libraryFilters.decode`).
/// `None` when it can't be read.
pub fn decode_filter(text: &str) -> Option<String> {
    use base64::Engine;
    let uri = percent_decode(text)?;
    let cleaned: String = uri.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '-' | '_')).collect();
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.replace('-', "+").replace('_', "/").as_bytes()).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `decodeURIComponent`: `None` for a broken `%` sequence.
pub fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The order of `new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })`, the original's
/// "natural sort": letter case and accents don't count, runs of digits compare as numbers, and spaces sort before
/// punctuation, punctuation before digits, digits before letters (the root collation's groups).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (ka, kb) = (collation_key(a), collation_key(b));
    let (mut i, mut j) = (0, 0);
    while i < ka.len() && j < kb.len() {
        let (ca, cb) = (ka[i], kb[j]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let (sa, sb) = (i, j);
            while i < ka.len() && ka[i].is_ascii_digit() {
                i += 1;
            }
            while j < kb.len() && kb[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = ka[sa..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = kb[sb..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if ord != Ordering::Equal {
                return ord;
            }
            continue;
        }
        let ord = (char_group(ca), ca).cmp(&(char_group(cb), cb));
        if ord != Ordering::Equal {
            return ord;
        }
        i += 1;
        j += 1;
    }
    (ka.len() - i.min(ka.len())).cmp(&(kb.len() - j.min(kb.len())))
}

/// The characters a string compares by: lower case, without accents.
fn collation_key(s: &str) -> Vec<char> {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// The collation group of a character: spaces, punctuation and symbols, digits, then letters.
fn char_group(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_ascii_digit() || c.is_numeric() {
        2
    } else if c.is_alphabetic() {
        3
    } else {
        1
    }
}

/// Sorts strings the original's natural way, keeping the order of equal ones (`naturalSort(list).asc()`).
pub fn natural_sort(list: &mut [String]) {
    list.sort_by(|a, b| natural_cmp(a, b));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_javascript() {
        assert_eq!(js_num(45.0), serde_json::json!(45));
        assert_eq!(js_num(15.5), serde_json::json!(15.5));
        assert_eq!(js_num(f64::NAN), Value::Null);
    }

    #[test]
    fn titles_and_prefixes() {
        let s = Sorting::default();
        assert_eq!(title_prefix_at_end("A Psalm for the Wild-Built", &s), "Psalm for the Wild-Built, A");
        assert_eq!(title_prefix_at_end("The Murderbot Diaries", &s), "Murderbot Diaries, The");
        assert_eq!(title_prefix_at_end("Ancillary Justice", &s), "Ancillary Justice");
        assert_eq!(title_ignore_prefix("The Murderbot Diaries", &s), "Murderbot Diaries");
    }

    #[test]
    fn natural_order() {
        let mut v: Vec<String> = ["b", "A", "a10", "a2", "Éclair", "eagle", "_x", "1"].iter().map(|s| s.to_string()).collect();
        natural_sort(&mut v);
        assert_eq!(v, vec!["_x", "1", "A", "a2", "a10", "b", "eagle", "Éclair"]);
    }

    #[test]
    fn filter_values_decode() {
        assert_eq!(decode_filter("U2NpZW5jZSBGaWN0aW9u").as_deref(), Some("Science Fiction"));
        assert_eq!(decode_filter("aW4tcHJvZ3Jlc3M%3D").as_deref(), Some("in-progress"));
    }
}
