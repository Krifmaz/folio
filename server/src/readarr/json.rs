//! How Readarr stores and sends its values, shared by every Readarr module: times, the JSON documents in its
//! columns (ratings, images, links), objects without their empty fields, numbers the way .NET writes them, and
//! query strings the way ASP.NET reads them.

use serde_json::{json, Map, Value};

/// A stored time (`2025-05-01 09:30:00Z`, or with up to seven decimals) as Readarr sends it
/// (`2025-05-01T09:30:00Z`); nothing stays nothing.
pub fn date(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Value::String(s.replacen(' ', "T", 1)),
        _ => Value::Null,
    }
}

/// A time from a request (`2013-01-01T00:00:00Z`) as Readarr stores it (`2013-01-01 00:00:00Z`); nothing stays
/// nothing.
pub fn stored_date(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) if !s.trim().is_empty() => {
            let s = s.trim().replacen('T', " ", 1);
            Value::String(if s.ends_with('Z') { s } else { format!("{s}Z") })
        }
        _ => Value::Null,
    }
}

/// A JSON value to store in a document column: its compact text, or nothing.
pub fn stored_doc(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(v) => Value::String(v.to_string()),
    }
}

/// A stored time that .NET can't leave empty: nothing becomes `0001-01-01T00:00:00Z`.
pub fn date_or_min(v: Option<&Value>) -> Value {
    match date(v) {
        Value::Null => json!("0001-01-01T00:00:00Z"),
        d => d,
    }
}

/// Now as Readarr passes it to SQLite (`2026-11-05 18:38:30.686076Z`, up to seven decimals), for comparing with
/// stored times.
pub fn now_param() -> String {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = t.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let (y, m, d) = civil(days);
    // .NET's `FFFFFFF`: the fraction without its trailing zeros, and without the point when it is zero.
    let fraction = format!("{:07}", t.subsec_nanos() / 100);
    let fraction = fraction.trim_end_matches('0');
    let fraction = if fraction.is_empty() { String::new() } else { format!(".{fraction}") };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}{fraction}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The calendar date of a day number since 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A JSON document stored as text; nothing (or text that isn't JSON) stays nothing.
pub fn doc(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// A stored list, or an empty one when there is none (the models' constructors make empty lists).
pub fn list(v: Option<&Value>) -> Value {
    match doc(v) {
        Value::Array(a) => Value::Array(a),
        _ => json!([]),
    }
}

/// A number .NET wrote from a `double`: whole numbers without a decimal point.
pub fn double(x: f64) -> Value {
    if x.fract() == 0.0 && x.abs() < 9.0e15 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

/// Stored ratings as Readarr sends them: `{votes, value, popularity}`, where popularity is value × votes;
/// nothing becomes the empty ratings.
pub fn ratings(v: Option<&Value>) -> Value {
    let r = doc(v);
    let votes = r.get("votes").and_then(Value::as_i64).unwrap_or(0);
    let value = r.get("value").filter(|v| v.is_number()).cloned().unwrap_or(json!(0));
    let popularity = double(value.as_f64().unwrap_or(0.0) * votes as f64);
    json!({ "votes": votes, "value": value, "popularity": popularity })
}

/// A stored list of covers (`[{url, coverType, extension}]`), each with its extension (Readarr takes it from the
/// address when it has none).
pub fn images(v: Option<&Value>) -> Vec<Value> {
    let Value::Array(list) = doc(v) else { return Vec::new() };
    list.into_iter()
        .map(|mut i| {
            if i.get("extension").and_then(Value::as_str).is_none_or(str::is_empty) {
                let url = i.get("url").and_then(Value::as_str).unwrap_or_default();
                i["extension"] = json!(extension(url));
            }
            i
        })
        .collect()
}

/// The extension of the last part of an address or path (`.jpg`), as .NET's `Path.GetExtension` gives it.
pub fn extension(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() => name[i..].to_string(),
        _ => String::new(),
    }
}

/// Covers as Readarr sends them for an item it has (`entity` is `author` or `book`): the address of its own copy
/// (`/MediaCover/1/poster.jpg`, `/MediaCover/Books/1/cover.jpg`) and the original's as `remoteUrl`.
pub fn local_covers(images: Vec<Value>, entity: &str, id: i64) -> Vec<Value> {
    images
        .into_iter()
        .map(|mut i| {
            let kind = i.get("coverType").and_then(Value::as_str).unwrap_or("unknown").to_lowercase();
            if kind == "unknown" || id == 0 {
                return i;
            }
            let ext = if kind == "clearlogo" { ".png".to_string() } else { i["extension"].as_str().unwrap_or_default().to_string() };
            let remote = i.get("url").cloned().unwrap_or(Value::Null);
            let folder = if entity == "book" { "Books/" } else { "" };
            i["remoteUrl"] = remote;
            i["url"] = json!(format!("/MediaCover/{folder}{id}/{kind}{ext}"));
            i
        })
        .collect()
}

/// An object of the given fields without the empty ones (Readarr leaves out fields that are null).
pub fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    let mut m = Map::new();
    for (k, v) in fields {
        if !v.is_null() {
            m.insert(k.to_string(), v);
        }
    }
    Value::Object(m)
}

/// A percentage the way .NET's `decimal` writes `part / whole * 100`: a whole number when the division comes out
/// whole, otherwise with decimals; 0 when there is no whole.
pub fn percent(part: i64, whole: i64) -> Value {
    if whole == 0 {
        json!(0)
    } else if part % whole == 0 {
        json!(part / whole * 100)
    } else {
        json!(part as f64 / whole as f64 * 100.0)
    }
}

/// The pairs of a query string, decoded (`a=1&b=x%20y`).
pub fn query_pairs(q: Option<&str>) -> Vec<(String, String)> {
    serde_urlencoded::from_str::<Vec<(String, String)>>(q.unwrap_or_default()).unwrap_or_default()
}

/// A query string read the way ASP.NET binds it: names without regard to case.
pub struct Query(pub Vec<(String, String)>);

impl Query {
    /// The query of a request.
    pub fn of(q: Option<&str>) -> Query {
        Query(query_pairs(q))
    }

    /// The first value of `name`, if any.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// `name` as a number, if it is given and is one.
    pub fn int(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(|v| v.trim().parse().ok())
    }

    /// `name` as true or false, if it is given and is one.
    pub fn bool(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    }

    /// Every value of `name` that is a number (`bookIds=1&bookIds=2`).
    pub fn ints(&self, name: &str) -> Vec<i64> {
        self.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case(name)).filter_map(|(_, v)| v.trim().parse().ok()).collect()
    }
}

/// `?, ?, ?` for `n` values (an `IN (...)` list).
pub fn marks(n: usize) -> String {
    vec!["?"; n].join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_numbers_and_covers_come_out_like_readarr() {
        assert_eq!(date(Some(&json!("2025-05-01 09:30:00Z"))), json!("2025-05-01T09:30:00Z"));
        assert_eq!(date(Some(&json!("2026-11-05 18:38:30.6860766Z"))), json!("2026-11-05T18:38:30.6860766Z"));
        assert_eq!(date_or_min(None), json!("0001-01-01T00:00:00Z"));
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_397), (2025, 11, 5));
        assert!(now_param().ends_with('Z') && now_param().len() == 28, "{}", now_param());
        assert_eq!(percent(1, 2), json!(50.0));
        assert_eq!(percent(1, 1), json!(100));
        assert_eq!(percent(0, 0), json!(0));
        assert_eq!(ratings(Some(&json!(r#"{"votes":300000,"value":4.0}"#))), json!({"votes":300000,"value":4.0,"popularity":1200000}));
        assert_eq!(ratings(None), json!({"votes":0,"value":0,"popularity":0}));
        let covers = local_covers(images(Some(&json!(r#"[{"url":"https://x/a.jpg","coverType":"poster"}]"#))), "author", 1);
        assert_eq!(
            covers,
            vec![json!({"url":"/MediaCover/1/poster.jpg","coverType":"poster","extension":".jpg","remoteUrl":"https://x/a.jpg"})]
        );
        assert_eq!(extension("/a/b.epub"), ".epub");
        let q = Query::of(Some("bookIds=1&BOOKIDS=2&x=true"));
        assert_eq!(q.ints("bookIds"), vec![1, 2]);
        assert_eq!(q.bool("X"), Some(true));
    }
}
