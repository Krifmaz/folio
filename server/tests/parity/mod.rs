//! The parity harness: answers recorded from the pinned original (`fixtures/<origin>/<name>.json`, made by
//! `abs/record.py`, see `record.md`) and [`assert_like`], which checks that Folio answers the same way.
//!
//! What must be equal: every key, every type, every value, list lengths and order. What may differ between two
//! runs, and is compared by shape only: ids (a UUID must be answered by a UUID, and the same original id always by
//! the same Folio id, so the links between objects hold), times (any number where the original has a time),
//! inode numbers and tokens.

#![allow(dead_code)]

use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

/// Keys whose values are times or run-specific numbers and strings: compared by type only.
pub const VOLATILE: &[&str] = &[
    "addedAt",
    "birthtimeMs",
    "createdAt",
    "ctimeMs",
    "lastScan",
    "lastUpdate",
    "loadedAt",
    "mtimeMs",
    "progressLastUpdate",
    "startedAt",
    "updatedAt",
    "lastSeen",
    "finishedAt",
    "ino",
    "token",
    "accessToken",
    "refreshToken",
];

/// One recorded exchange with the original.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// The request as recorded (`method`, `path` with `{placeholders}`, `body`).
    pub request: Value,
    /// The original's HTTP status.
    pub status: u16,
    /// The original's content type, without parameters.
    pub content_type: String,
    /// The original's answer: JSON, or a string for text answers.
    pub body: Value,
}

/// The fixtures folder of an origin (`abs`).
pub fn dir(origin: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity/fixtures").join(origin)
}

/// A recorded exchange by name, e.g. `fixture("abs", "library_items")`.
pub fn fixture(origin: &str, name: &str) -> Fixture {
    let path = dir(origin).join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("no fixture {}: {e}", path.display()));
    let v: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("fixture {} is not JSON: {e}", path.display()));
    Fixture {
        request: v["request"].clone(),
        status: v["status"].as_u64().unwrap_or(0) as u16,
        content_type: v["content_type"].as_str().unwrap_or_default().to_string(),
        body: v["body"].clone(),
    }
}

/// Whether a string is a UUID (the original's ids).
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// Compares Folio's answer with the original's; ids are matched up across the whole comparison.
#[derive(Default)]
pub struct Matcher {
    ids: HashMap<String, String>,
    /// Extra keys compared by type only, for one test.
    pub volatile: Vec<String>,
}

impl Matcher {
    /// Every difference between `actual` (Folio) and `expected` (the original), as `path: what` lines.
    pub fn diff(&mut self, actual: &Value, expected: &Value) -> Vec<String> {
        let mut out = Vec::new();
        self.walk("$", None, actual, expected, &mut out);
        out
    }

    fn walk(&mut self, path: &str, key: Option<&str>, a: &Value, e: &Value, out: &mut Vec<String>) {
        let volatile = key.is_some_and(|k| VOLATILE.contains(&k) || self.volatile.iter().any(|v| v == k));
        match (a, e) {
            (Value::Object(am), Value::Object(em)) => {
                for k in em.keys().filter(|k| !am.contains_key(*k)) {
                    out.push(format!("{path}.{k}: missing"));
                }
                for k in am.keys().filter(|k| !em.contains_key(*k)) {
                    out.push(format!("{path}.{k}: not in the original"));
                }
                for (k, ev) in em {
                    if let Some(av) = am.get(k) {
                        self.walk(&format!("{path}.{k}"), Some(k), av, ev, out);
                    }
                }
            }
            (Value::Array(aa), Value::Array(ea)) => {
                if aa.len() != ea.len() {
                    out.push(format!("{path}: {} items, the original has {}", aa.len(), ea.len()));
                }
                for (i, (av, ev)) in aa.iter().zip(ea).enumerate() {
                    self.walk(&format!("{path}[{i}]"), key, av, ev, out);
                }
            }
            (Value::String(as_), Value::String(es)) if is_uuid(es) => {
                if !is_uuid(as_) {
                    out.push(format!("{path}: {as_:?} is not an id like the original's"));
                } else if let Some(known) = self.ids.get(es) {
                    if known != as_ {
                        out.push(format!("{path}: id {as_} where the original links to the object answered as {known}"));
                    }
                } else {
                    self.ids.insert(es.clone(), as_.clone());
                }
            }
            (Value::Number(_), Value::Number(_)) | (Value::String(_), Value::String(_)) if volatile => {}
            _ if volatile && a.is_null() == e.is_null() && std::mem::discriminant(a) == std::mem::discriminant(e) => {}
            _ if a == e => {}
            _ => out.push(format!("{path}: {a} where the original has {e}")),
        }
    }
}

/// Panics with every difference when Folio's `status` and `body` don't answer like the fixture.
pub fn assert_like(status: u16, body: &Value, fx: &Fixture) {
    let mut m = Matcher::default();
    assert_like_with(&mut m, status, body, fx);
}

/// [`assert_like`] with a matcher kept across several answers (so ids must stay linked between them).
pub fn assert_like_with(m: &mut Matcher, status: u16, body: &Value, fx: &Fixture) {
    let mut diffs = Vec::new();
    if status != fx.status {
        diffs.push(format!("status {status} where the original answers {}", fx.status));
    }
    diffs.extend(m.diff(body, &fx.body));
    assert!(diffs.is_empty(), "Folio doesn't answer like the original ({}):\n{}", fx.request["path"], diffs.join("\n"));
}
