//! Readarr's qualities (`NzbDrone.Core.Qualities`): their names and default weights, a stored quality as the API
//! sends it, the quality a file's extension gives, and whether a file meets its quality profile's cutoff.

use kit_server_core::{Db, RowExt, SqliteDb};
use serde_json::{json, Value};

/// Every quality: id, name and its default weight (`Quality.DefaultQualityDefinitions`).
pub const QUALITIES: &[(i64, &str, i64)] = &[
    (0, "Unknown Text", 1),
    (1, "PDF", 5),
    (2, "MOBI", 10),
    (3, "EPUB", 11),
    (4, "AZW3", 12),
    (13, "Unknown Audio", 50),
    (10, "MP3", 100),
    (12, "M4B", 105),
    (11, "FLAC", 110),
];

/// The name of quality `id` (`Unknown Text` for one Readarr doesn't know).
pub fn name(id: i64) -> &'static str {
    QUALITIES.iter().find(|q| q.0 == id).map(|q| q.1).unwrap_or("Unknown Text")
}

/// A stored quality model (`{"quality": 3, "revision": {...}}`) as the API sends it, with the quality's name.
pub fn model(stored: &Value) -> Value {
    let id = stored.get("quality").and_then(Value::as_i64).unwrap_or(0);
    let revision = stored.get("revision").cloned().unwrap_or_else(|| json!({ "version": 1, "real": 0, "isRepack": false }));
    json!({ "quality": { "id": id, "name": name(id) }, "revision": revision })
}

/// The weight the API gives a file's quality: the quality's weight, plus ten for each `real` and one for each
/// `version` of its revision.
pub fn weight(stored: &Value) -> i64 {
    let id = stored.get("quality").and_then(Value::as_i64).unwrap_or(0);
    let base = QUALITIES.iter().find(|q| q.0 == id).map(|q| q.2).unwrap_or(0);
    let rev = stored.get("revision").cloned().unwrap_or(Value::Null);
    base + rev.get("real").and_then(Value::as_i64).unwrap_or(0) * 10 + rev.get("version").and_then(Value::as_i64).unwrap_or(1)
}

/// The quality of a file from its extension, as Readarr's parser gives it for ebooks and audio.
pub fn from_extension(path: &str) -> Value {
    let id = match super::json::extension(path).to_lowercase().as_str() {
        ".pdf" => 1,
        ".mobi" => 2,
        ".epub" | ".kepub" => 3,
        ".azw3" | ".azw" => 4,
        ".mp3" => 10,
        ".flac" => 11,
        ".m4b" | ".m4a" => 12,
        ".aac" | ".ogg" | ".opus" | ".wma" | ".wav" | ".ape" => 13,
        _ => 0,
    };
    model(&json!({ "quality": id, "revision": { "version": 1, "real": 0, "isRepack": false } }))
}

/// Where a quality sits in a profile's list (a group counts as one place); -1 when it isn't there.
fn index(items: &[Value], id: i64) -> i64 {
    for (i, item) in items.iter().enumerate() {
        if item.get("quality").and_then(Value::as_i64) == Some(id) && item.get("quality").is_some_and(|q| !q.is_null()) {
            return i as i64;
        }
        if item.get("id").and_then(Value::as_i64).is_some_and(|g| g > 0 && g == id) {
            return i as i64;
        }
        let group = item.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        if group.iter().any(|g| g.get("quality").and_then(Value::as_i64) == Some(id)) {
            return i as i64;
        }
    }
    -1
}

/// Whether a file of the stored quality `stored` is below the cutoff of quality profile `profile_id`
/// (`UpgradableSpecification.QualityCutoffNotMet`): the cutoff is the profile's own when upgrades are allowed,
/// else its first allowed quality.
pub fn cutoff_not_met(db: &SqliteDb, profile_id: i64, stored: &Value) -> kit_server_core::Result<bool> {
    let Some(p) = db.one("SELECT Cutoff, Items, UpgradeAllowed FROM QualityProfiles WHERE Id = ?", vec![json!(profile_id)])? else {
        return Ok(false);
    };
    let items = match super::json::doc(p.get("Items")) {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let cutoff = if p.i("UpgradeAllowed") != 0 {
        p.i("Cutoff")
    } else {
        let Some(first) = items.iter().find(|i| i.get("allowed").and_then(Value::as_bool) == Some(true)) else { return Ok(false) };
        match first.get("quality").and_then(Value::as_i64) {
            Some(q) => q,
            None => first["items"][0]["quality"].as_i64().unwrap_or(0),
        }
    };
    let have = stored.get("quality").and_then(Value::as_i64).unwrap_or(0);
    Ok(index(&items, have) < index(&items, cutoff))
}
