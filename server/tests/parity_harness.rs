//! Tests of the parity harness itself: every recorded fixture loads, and the comparison accepts the original's
//! answer with other ids and times but catches a changed value, a broken link, a missing or extra key.

mod parity;

use parity::{assert_like, dir, fixture, is_uuid, Matcher};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The same answer with every id replaced (consistently) and every time moved, as another run would give.
fn another_run(v: &Value, ids: &mut HashMap<String, String>) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    let moved = match x {
                        Value::Number(n) if parity::VOLATILE.contains(&k.as_str()) => json!(n.as_f64().unwrap_or(0.0) + 1000.0),
                        _ => another_run(x, ids),
                    };
                    (k.clone(), moved)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| another_run(x, ids)).collect()),
        Value::String(s) if is_uuid(s) => {
            let n = ids.len();
            json!(ids.entry(s.clone()).or_insert_with(|| format!("{n:08x}-0000-4000-8000-000000000000")).clone())
        }
        other => other.clone(),
    }
}

#[test]
fn every_fixture_loads_and_matches_another_run_of_itself() {
    let mut names: Vec<String> = std::fs::read_dir(dir("abs"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".json") && !n.starts_with('_'))
        .map(|n| n.trim_end_matches(".json").to_string())
        .collect();
    names.sort();
    assert!(names.len() >= 60, "only {} fixtures", names.len());
    for name in names {
        let fx = fixture("abs", &name);
        assert!(fx.status >= 200, "{name} has no status");
        let other = another_run(&fx.body, &mut HashMap::new());
        assert_like(fx.status, &other, &fx);
    }
}

#[test]
fn a_changed_value_a_broken_link_and_missing_keys_are_caught() {
    let a = "11111111-1111-4111-8111-111111111111";
    let b = "22222222-2222-4222-8222-222222222222";
    let original = json!({"id": a, "items": [{"libraryItemId": a, "title": "T"}], "updatedAt": 5});
    let mut m = Matcher::default();
    assert!(m.diff(&json!({"id": b, "items": [{"libraryItemId": b, "title": "T"}], "updatedAt": 9}), &original).is_empty());
    let mut m = Matcher::default();
    let broken = m.diff(&json!({"id": b, "items": [{"libraryItemId": a, "title": "T"}], "updatedAt": 9}), &original);
    assert_eq!(broken.len(), 1, "{broken:?}");
    let mut m = Matcher::default();
    let changed = m.diff(&json!({"id": b, "items": [{"libraryItemId": b, "title": "U"}], "updatedAt": 9}), &original);
    assert_eq!(changed, vec![r#"$.items[0].title: "U" where the original has "T""#]);
    let mut m = Matcher::default();
    let keys = m.diff(&json!({"id": b, "extra": 1, "updatedAt": "late"}), &original);
    assert!(keys.iter().any(|d| d.contains("items: missing")), "{keys:?}");
    assert!(keys.iter().any(|d| d.contains("extra: not in the original")), "{keys:?}");
    assert!(keys.iter().any(|d| d.contains("updatedAt")), "a time must stay a number: {keys:?}");
}
