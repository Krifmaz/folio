//! The port of `server/models/MediaProgress.js`: the JSON the apps get ([`old_json`], `getOldMediaProgress`), a
//! new progress from a request ([`from_payload`]) and an update to a stored one ([`apply_update`],
//! `applyProgressUpdate`).

use super::MediaProgress;
use serde_json::{json, Map, Value};

/// A stored time as the API shows it: milliseconds, or null.
fn time_ms(t: &Option<String>) -> Value {
    t.as_deref().and_then(super::date_ms).map(Value::from).unwrap_or(Value::Null)
}

/// A number as JavaScript's JSON writes it: whole numbers without a fraction (`30`, not `30.0`).
pub fn js_json(n: Option<f64>) -> Value {
    match n {
        None => Value::Null,
        Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
        Some(f) => json!(f),
    }
}

/// One progress as the apps know it (`getOldMediaProgress`).
pub fn old_json(mp: &MediaProgress) -> Value {
    let extra = mp.extra_data.clone().unwrap_or(Value::Null);
    let episode = mp.media_item_type.as_deref() == Some("podcastEpisode");
    let progress = extra.get("progress").filter(|p| crate::abs::settings::truthy(Some(p))).cloned().unwrap_or(json!(0));
    let progress = progress.as_f64().map(|p| js_json(Some(p))).unwrap_or(progress);
    json!({
        "id": mp.id,
        "userId": mp.user_id,
        "libraryItemId": extra.get("libraryItemId").filter(|v| crate::abs::settings::truthy(Some(v))).cloned().unwrap_or(Value::Null),
        "episodeId": if episode { json!(mp.media_item_id) } else { Value::Null },
        "mediaItemId": mp.media_item_id,
        "mediaItemType": mp.media_item_type,
        "duration": js_json(mp.duration),
        "progress": progress,
        "currentTime": js_json(mp.current_time),
        "isFinished": mp.is_finished.unwrap_or(false),
        "hideFromContinueListening": mp.hide_from_continue_listening.unwrap_or(false),
        "ebookLocation": mp.ebook_location,
        "ebookProgress": js_json(mp.ebook_progress),
        "lastUpdate": time_ms(&mp.updated_at),
        "startedAt": time_ms(&mp.created_at),
        "finishedAt": time_ms(&mp.finished_at)
    })
}

/// A JavaScript `Number(v)` of a request field, `None` where the original's `isNullOrNaN` is true (missing,
/// `null`, or not a number).
pub fn js_number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::String(s) if s.trim().is_empty() => Some(0.0),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| !n.is_nan()),
        _ => None,
    }
}

/// A JavaScript truthiness test of a request field (`!!v`).
pub fn js_truthy(v: Option<&Value>) -> bool {
    crate::abs::settings::truthy(v)
}

/// A time a request gives (milliseconds or a date text) as the original stores it; `None` when it isn't one.
fn js_date(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::Number(n) => n.as_f64().map(|ms| super::date_text(ms as i64)),
        Value::String(s) => super::date_ms(s).or_else(|| s.trim().parse::<f64>().ok().map(|n| n as i64)).map(super::date_text),
        _ => None,
    }
}

/// How far through the item, from the times (`get progress`): 0 without a length.
fn time_progress(mp: &MediaProgress) -> f64 {
    let duration = mp.duration.unwrap_or(0.0);
    if duration == 0.0 {
        return 0.0;
    }
    (mp.current_time.unwrap_or(0.0) / duration).clamp(0.0, 1.0)
}

/// Sets `extraData.progress`.
fn set_extra_progress(mp: &mut MediaProgress, p: f64) {
    let mut extra = mp.extra_data.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    extra["progress"] = json!(p);
    mp.extra_data = Some(extra);
}

/// A new progress from a request (`createUpdateMediaProgressFromPayload`, when there is none yet): the times,
/// the flags and the ebook place it gives, the library item and progress in `extraData`.
pub fn from_payload(user_id: &str, media_item_id: &str, podcast_id: Option<&str>, payload: &Map<String, Value>) -> MediaProgress {
    let episode = js_truthy(payload.get("episodeId"));
    let finished = js_truthy(payload.get("isFinished"));
    let now = super::date_now();
    let created_at = payload.get("createdAt").filter(|v| js_truthy(Some(v))).and_then(|v| js_date(Some(v))).unwrap_or_else(|| now.clone());
    let finished_at = if finished {
        payload.get("finishedAt").filter(|v| js_truthy(Some(v))).and_then(|v| js_date(Some(v))).or_else(|| Some(now.clone()))
    } else {
        None
    };
    let progress = if finished { 1.0 } else { js_number(payload.get("progress")).unwrap_or(0.0) };
    let ebook_location = payload.get("ebookLocation").filter(|v| js_truthy(Some(v))).map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    MediaProgress {
        id: Some(super::new_id()),
        media_item_id: Some(media_item_id.to_string()),
        media_item_type: Some(if episode { "podcastEpisode" } else { "book" }.into()),
        duration: Some(js_number(payload.get("duration")).unwrap_or(0.0)),
        current_time: Some(js_number(payload.get("currentTime")).unwrap_or(0.0)),
        is_finished: Some(finished),
        hide_from_continue_listening: Some(js_truthy(payload.get("hideFromContinueListening"))),
        ebook_location,
        ebook_progress: Some(js_number(payload.get("ebookProgress")).unwrap_or(0.0)),
        finished_at,
        extra_data: Some(json!({ "libraryItemId": payload.get("libraryItemId").cloned().unwrap_or(Value::Null), "progress": progress })),
        podcast_id: podcast_id.map(str::to_string),
        created_at: Some(created_at),
        updated_at: Some(now),
        user_id: Some(user_id.to_string()),
    }
}

/// Applies a request to a stored progress (`applyProgressUpdate`): finished or not, the progress, the fields it
/// sets, then whether it is now finished (`markAsFinishedPercentComplete`, else `markAsFinishedTimeRemaining`,
/// 10 seconds by default) and, for a sync from a phone, the time it was made (`lastUpdate`). Answers whether
/// anything changed (the original saves, and moves `updatedAt`, only then).
pub fn apply_update(mp: &mut MediaProgress, payload: &Map<String, Value>) -> bool {
    let before = mp.clone();
    let mut payload = payload.clone();
    let was_finished = mp.is_finished.unwrap_or(false);
    if payload.contains_key("isFinished") {
        let finished = js_truthy(payload.get("isFinished"));
        if finished && !was_finished {
            mp.finished_at =
                payload.get("finishedAt").filter(|v| js_truthy(Some(v))).and_then(|v| js_date(Some(v))).or_else(|| Some(super::date_now()));
            set_extra_progress(mp, 1.0);
            payload.remove("finishedAt");
        } else if !finished && was_finished {
            mp.finished_at = None;
            set_extra_progress(mp, 0.0);
            mp.current_time = Some(0.0);
            payload.remove("finishedAt");
            payload.remove("currentTime");
        }
    } else if let Some(p) = js_number(payload.get("progress")) {
        if p != time_progress(mp) {
            set_extra_progress(mp, p.clamp(0.0, 1.0));
        }
    }
    // `this.set(progressPayload)`: the fields of the progress the request gives
    if let Some(v) = payload.get("duration") {
        mp.duration = js_number(Some(v));
    }
    if let Some(v) = payload.get("currentTime") {
        mp.current_time = js_number(Some(v));
    }
    if let Some(v) = payload.get("isFinished") {
        mp.is_finished = Some(js_truthy(Some(v)));
    }
    if let Some(v) = payload.get("hideFromContinueListening") {
        mp.hide_from_continue_listening = Some(js_truthy(Some(v)));
    }
    if let Some(v) = payload.get("ebookLocation") {
        mp.ebook_location = match v {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        };
    }
    if let Some(v) = payload.get("ebookProgress") {
        mp.ebook_progress = js_number(Some(v));
    }
    if let Some(v) = payload.get("finishedAt") {
        mp.finished_at = js_date(Some(v));
    }
    let time_changed = mp.current_time != before.current_time;
    if time_changed && !js_truthy(payload.get("hideFromContinueListening")) {
        mp.hide_from_continue_listening = Some(false);
    }
    let duration = mp.duration.unwrap_or(0.0);
    let mut should_finish = false;
    if duration != 0.0 {
        match js_number(payload.get("markAsFinishedPercentComplete")).filter(|p| *p > 0.0) {
            Some(percent) => should_finish = percent / 100.0 < time_progress(mp),
            None => {
                let remaining = js_number(payload.get("markAsFinishedTimeRemaining")).unwrap_or(10.0);
                should_finish = duration - mp.current_time.unwrap_or(0.0) < remaining;
            }
        }
    }
    if !mp.is_finished.unwrap_or(false) && should_finish {
        mp.is_finished = Some(true);
        if mp.finished_at.is_none() {
            mp.finished_at = Some(super::date_now());
        }
        set_extra_progress(mp, 1.0);
    } else if mp.is_finished.unwrap_or(false) && time_changed && !should_finish {
        mp.is_finished = Some(false);
        mp.finished_at = None;
    }
    let changed = *mp != before;
    if changed {
        mp.updated_at = Some(super::date_now());
    }
    if let Some(at) = payload.get("lastUpdate").filter(|v| js_truthy(Some(v))).and_then(|v| js_date(Some(v))) {
        mp.updated_at = Some(at);
        return true;
    }
    changed
}
