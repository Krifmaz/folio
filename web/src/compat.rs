//! `#/apps`, Other apps: the addresses to give e-readers and listening apps (OPDS, Audiobookshelf apps, KOReader,
//! Kobo), each with Copy, the KOReader sync password, and the Kobo sync address. How to set each app up is in
//! docs/COMPAT.md; the page says it in short.

use kit_leptos::api;
use kit_leptos::clipboard::copy_text;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// An address shown in full with a Copy button.
#[component]
fn Address(value: String) -> impl IntoView {
    let copy = value.clone();
    view! {
        <div class="row" style="gap:8px;margin-top:8px">
            <input class="input mono grow" readonly prop:value=value.clone() aria-label="Address"/>
            <button class="btn sm" type="button" on:click=move |_| copy_text(&copy)>"Copy"</button>
        </div>
    }
}

/// The KOReader card: its address and the sync password form.
#[component]
fn KoreaderCard(address: String, username: String, set: bool, reload: Callback<()>) -> impl IntoView {
    let (password, set_password) = signal(String::new());
    let save = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let pw = password.get_untracked();
        kit_leptos::rt::spawn(async move {
            match api::put("/apps/koreader", json!({ "password": pw })).await {
                Ok(_) => {
                    toast("KOReader sync password saved. Use Login in KOReader now.", "ok");
                    set_password.set(String::new());
                    reload.run(());
                }
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    view! {
        <div class="card mt" id="koreader-card">
            <h2>"KOReader progress sync"</h2>
            <p class="muted">"In KOReader: Tools → Progress sync → Custom sync server, then Login with your username "<b>{username}</b>" and the password below."</p>
            <Address value=address/>
            <p class="small faint mt">{if set { "A sync password is set. Saving a new one replaces it." } else { "No sync password yet. KOReader can't sync until you set one." }}</p>
            <form class="row wrap" style="gap:8px" on:submit=save>
                <input class="input grow" type="password" autocomplete="new-password" placeholder="At least 8 characters" prop:value=password on:input=move |e| set_password.set(event_target_value(&e)) aria-label="KOReader sync password"/>
                <button class="btn primary" type="submit">"Save password"</button>
            </form>
        </div>
    }
}

/// The Kobo card: the sync address, made on request, and a way to replace it.
#[component]
fn KoboCard(address: Option<String>, reload: Callback<()>) -> impl IntoView {
    let make = move |replace: bool| {
        kit_leptos::rt::spawn(async move {
            if replace {
                let q = kit_rules::dialogs::Confirm::new("Make a new Kobo sync address?")
                    .text("The old address stops working at once. Put the new one into every Kobo that syncs with Folio.")
                    .ok("Make a new address")
                    .danger(true);
                if !kit_leptos::modal::confirm(q).await {
                    return;
                }
            }
            let answer = if replace { api::post_empty("/apps/kobo").await } else { api::get("/apps/kobo").await };
            match answer {
                Ok(_) => reload.run(()),
                Err(e) => toast(&e.message, "err"),
            }
        });
    };
    let body = match address {
        Some(a) => view! {
            <Address value=a/>
            <p class="small muted mt">"On the Kobo (by USB): open .kobo/Kobo/Kobo eReader.conf, set api_endpoint= under [OneStoreServices] to this address, eject and tap Sync."</p>
            <button class="btn sm mt" type="button" on:click=move |_| make(true)>"Make a new address"</button>
        }
        .into_any(),
        None => view! { <button class="btn primary mt" type="button" on:click=move |_| make(false)>"Make my Kobo sync address"</button> }.into_any(),
    };
    view! {
        <div class="card mt" id="kobo-card">
            <h2>"Kobo e-readers"</h2>
            <p class="muted">"A Kobo syncs your ebooks over Wi-Fi and keeps its place in step with Folio. While it syncs with Folio, books bought in the Kobo store don't sync to it."</p>
            {body}
        </div>
    }
}

/// `#/apps`: Other apps.
#[component]
pub fn OtherApps() -> impl IntoView {
    let data = LocalResource::new(|| async { api::get("/apps").await.map_err(|e| e.message) });
    let reload = Callback::new(move |_| data.refetch());
    let body = move || {
        match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => view! {
            <div class="card mt" id="opds-card">
                <h2>"E-readers and reading apps (OPDS)"</h2>
                <p class="muted">"For KOReader, Moon+ Reader, Thorium and other OPDS apps: add a catalogue with this address and your Folio username and password."</p>
                <Address value=s(&v, "opds")/>
            </div>
            <div class="card mt" id="abs-card">
                <h2>"Audiobookshelf apps"</h2>
                <p class="muted">"The Audiobookshelf app, ShelfPlayer, Plappa and others: use this server address and your Folio username and password."</p>
                <Address value=s(&v, "audiobookshelf")/>
            </div>
            <KoreaderCard address=s(&v, "koreader") username=s(&v, "username") set=v["koreader_password_set"].as_bool().unwrap_or(false) reload=reload/>
            <KoboCard address=v.get("kobo").and_then(Value::as_str).map(str::to_string) reload=reload/>
        }
        .into_any(),
    }
    };
    view! {
        <h1>"Other apps"</h1>
        <p class="muted">"The apps you already read and listen with can use Folio's library. Your place is kept in step with Folio in every one of them."</p>
        {body}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_fields_are_empty() {
        assert_eq!(super::s(&serde_json::json!({ "opds": "x" }), "kobo"), "");
    }
}
