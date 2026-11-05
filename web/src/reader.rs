//! The EPUB reader (`#/read?id=<edition id>`) (the player and reader track, docs/TRACKS.md). A placeholder until
//! that track's first PR.

use leptos::prelude::*;

/// `#/read?id=<edition id>`: the reader.
#[component]
pub fn Reader(id: i64) -> impl IntoView {
    view! { <div class="empty"><p class="muted">{format!("The reader for edition {id} is on its way.")}</p></div> }
}
