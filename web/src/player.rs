//! The audiobook player (`#/play?id=<edition id>`) and the Listen and Read buttons on an edition (the player and
//! reader track, docs/TRACKS.md). A placeholder until that track's first PR.

use leptos::prelude::*;
use serde_json::Value;

/// The buttons an edition's row shows to play or read it (none yet).
pub fn edition_actions(_edition: &Value) -> AnyView {
    ().into_any()
}

/// `#/play?id=<edition id>`: the player.
#[component]
pub fn Player(id: i64) -> impl IntoView {
    view! { <div class="empty"><p class="muted">{format!("The player for edition {id} is on its way.")}</p></div> }
}
