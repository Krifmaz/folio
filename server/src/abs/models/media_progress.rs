//! The port of `server/models/MediaProgress.js`. For now only the JSON the apps get inside a user
//! ([`old_json`], `getOldMediaProgress`); track A ports the rest with playback (docs/TRACKS.md).

use super::MediaProgress;
use serde_json::{json, Value};

/// A stored time as the API shows it: milliseconds, or null.
fn time_ms(t: &Option<String>) -> Value {
    t.as_deref().and_then(super::date_ms).map(Value::from).unwrap_or(Value::Null)
}

/// One progress as the apps know it (`getOldMediaProgress`).
pub fn old_json(mp: &MediaProgress) -> Value {
    let extra = mp.extra_data.clone().unwrap_or(Value::Null);
    let episode = mp.media_item_type.as_deref() == Some("podcastEpisode");
    let progress = extra.get("progress").filter(|p| crate::abs::settings::truthy(Some(p))).cloned().unwrap_or(json!(0));
    json!({
        "id": mp.id,
        "userId": mp.user_id,
        "libraryItemId": extra.get("libraryItemId").filter(|v| crate::abs::settings::truthy(Some(v))).cloned().unwrap_or(Value::Null),
        "episodeId": if episode { json!(mp.media_item_id) } else { Value::Null },
        "mediaItemId": mp.media_item_id,
        "mediaItemType": mp.media_item_type,
        "duration": mp.duration,
        "progress": progress,
        "currentTime": mp.current_time,
        "isFinished": mp.is_finished.unwrap_or(false),
        "hideFromContinueListening": mp.hide_from_continue_listening.unwrap_or(false),
        "ebookLocation": mp.ebook_location,
        "ebookProgress": mp.ebook_progress,
        "lastUpdate": time_ms(&mp.updated_at),
        "startedAt": time_ms(&mp.created_at),
        "finishedAt": time_ms(&mp.finished_at)
    })
}
