//! The Wanted page: the wanted list per format with each book's status, the preferences for each format (file
//! formats in order, languages, unabridged only) and Check followed now.

use crate::library::{format_label, runtime};
use kit_leptos::api;
use kit_leptos::icons::icon;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The words for a wanted status, and its badge's colour class.
pub fn status_label(status: &str) -> (&'static str, &'static str) {
    match status {
        "wanted" => ("Wanted", "info"),
        "searching" => ("Searching", "info"),
        "downloading" => ("Downloading", "info"),
        "downloaded" => ("Arrived", "info"),
        "review" => ("Needs a look", "warn"),
        "imported" => ("In the library", "ok"),
        "failed" => ("Failed", "err"),
        _ => ("Unknown", ""),
    }
}

/// The follow choices people see, with the values the server keeps.
pub const FOLLOW_CHOICES: &[(&str, &str)] =
    &[("", "Not followed"), ("audiobook", "Audiobooks"), ("ebook", "Ebooks"), ("both", "Audiobooks and ebooks")];

/// A select that follows an author or series (`kind` is `authors` or `series`) in the formats picked.
pub fn follow_select(kind: &'static str, id: i64, current: String) -> impl IntoView {
    let change = move |e: leptos::ev::Event| {
        let follow = event_target_value(&e);
        kit_leptos::rt::spawn(async move {
            match api::put(&format!("/library/{kind}/{id}/follow"), json!({ "follow": follow })).await {
                Ok(_) => {
                    toast(if follow.is_empty() { "No longer followed." } else { "Followed. New books are added to the wanted list." }, "ok")
                }
                Err(e) => toast(&e.message, "err"),
            }
        })
    };
    view! {
        <select class="input follow-select" style="max-width:220px" on:change=change aria-label="Follow">
            {FOLLOW_CHOICES.iter().map(|(v, l)| view! { <option value=*v selected=*v == current>{*l}</option> }).collect_view()}
        </select>
    }
}

/// One wanted book: cover, title, who wrote and reads it, its preferences, status and Remove.
fn wanted_row(w: &Value, admin: bool, on_change: impl Fn() + Copy + 'static) -> impl IntoView {
    let id = w.get("id").and_then(Value::as_i64).unwrap_or(0);
    let e = w.get("edition").cloned().unwrap_or(Value::Null);
    let (label, class) = status_label(&s(w, "status"));
    let authors = e
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let narr = e
        .get("narrators")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let ident = if s(&e, "format") == "audiobook" { format!("ASIN {}", s(&e, "asin")) } else { format!("ISBN {}", s(&e, "isbn13")) };
    let mut facts = vec![format_label(&s(&e, "format")).to_string(), ident, s(&e, "language").to_uppercase()];
    if let Some(m) = e.get("runtime_min").and_then(Value::as_u64) {
        facts.push(runtime(m));
    }
    if !narr.is_empty() {
        facts.push(format!("read by {narr}"));
    }
    let work = e.get("work_id").and_then(Value::as_i64).unwrap_or(0);
    let title = s(&e, "title");
    let note = s(w, "note");
    let remove = move |_| {
        let title = title.clone();
        kit_leptos::rt::spawn(async move {
            let q = kit_rules::dialogs::Confirm::new("Stop wanting this book?")
                .text(format!("{title} comes off the wanted list. It stays in the library."))
                .ok("Remove")
                .danger(true);
            if kit_leptos::modal::confirm(q).await {
                match api::del(&format!("/wanted/{id}")).await {
                    Ok(_) => on_change(),
                    Err(e) => toast(&e.message, "err"),
                }
            }
        })
    };
    view! {
        <div class="list-item wanted-row" style="gap:12px;flex-wrap:wrap">
            <a class="grow" style="min-width:220px" href=format!("#/work?id={work}")>
                <div style="font-weight:650">{s(&e, "title")}</div>
                <div class="small muted">{authors}</div>
                <div class="small faint">{facts.into_iter().filter(|f| !f.is_empty()).collect::<Vec<_>>().join(" · ")}</div>
                {(!note.is_empty()).then(|| view! { <div class="small muted">{note}</div> })}
            </a>
            <span class="small faint">{s(w, "profile")}</span>
            <span class=format!("badge {class}")>{label}</span>
            {admin.then(|| view! {
                <a class="btn sm" href=format!("#/releases?id={id}") title="See what Lantern finds">"Results"</a>
                <button class="btn sm" title="Search and grab the best result" on:click=move |_| crate::downloads::search_now(id, on_change)>"Search now"</button>
                <button class="btn ghost icon sm" title="Remove" aria-label="Remove" on:click=remove inner_html=icon("trash")></button>
            })}
        </div>
    }
}

/// The preferences of one format: file formats in order (moved up and down, turned on and off), languages
/// and, for audiobooks, unabridged only.
#[component]
fn ProfileCard(profile: Value, known: Vec<String>, admin: bool) -> impl IntoView {
    let id = profile.get("id").and_then(Value::as_i64).unwrap_or(0);
    let format = s(&profile, "format");
    let list = |k: &str| -> Vec<String> {
        profile
            .get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    };
    let (formats, set_formats) = signal(list("file_formats"));
    let (languages, set_languages) = signal(list("languages").join(", "));
    let (unabridged, set_unabridged) = signal(profile.get("unabridged_only").and_then(Value::as_bool).unwrap_or(false));
    let name = s(&profile, "name");
    let save = {
        let (name, format) = (name.clone(), format.clone());
        move |_| {
            let body = json!({
                "format": format,
                "name": name,
                "file_formats": formats.get_untracked(),
                "languages": languages.get_untracked().split(',').map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
                "unabridged_only": unabridged.get_untracked(),
            });
            kit_leptos::rt::spawn(async move {
                match api::put(&format!("/profiles/{id}"), body).await {
                    Ok(_) => toast("The preferences were saved.", "ok"),
                    Err(e) => toast(&e.message, "err"),
                }
            })
        }
    };
    let rows = {
        let known = known.clone();
        move || {
            let on = formats.get();
            let mut all: Vec<String> = on.clone();
            all.extend(known.iter().filter(|k| !on.contains(k)).cloned());
            all.into_iter()
                .map(|f| {
                    let pos = on.iter().position(|x| *x == f);
                    let (f1, f2, f3) = (f.clone(), f.clone(), f.clone());
                    let toggle = move |_| set_formats.update(|v| if let Some(i) = v.iter().position(|x| *x == f1) { v.remove(i); } else { v.push(f1.clone()) });
                    let up = move |_| set_formats.update(|v| if let Some(i) = v.iter().position(|x| *x == f2) { if i > 0 { v.swap(i, i - 1) } });
                    let down = move |_| set_formats.update(|v| if let Some(i) = v.iter().position(|x| *x == f3) { if i + 1 < v.len() { v.swap(i, i + 1) } });
                    view! {
                        <div class="row" style="gap:8px">
                            <label class="row grow" style="gap:8px"><input type="checkbox" checked=pos.is_some() disabled=!admin on:change=toggle/><span class="mono">{f.to_uppercase()}</span>{pos.map(|i| view! { <span class="small faint">{if i == 0 { "best".to_string() } else { format!("choice {}", i + 1) }}</span> })}</label>
                            {(admin && pos.is_some()).then(|| view! {
                                <button class="btn ghost sm" type="button" on:click=up aria-label="Move up">"↑"</button>
                                <button class="btn ghost sm" type="button" on:click=down aria-label="Move down">"↓"</button>
                            })}
                        </div>
                    }
                })
                .collect_view()
        }
    };
    let audio = format == "audiobook";
    view! {
        <div class="card profile-card">
            <h3>{name}</h3>
            <div class="small muted" style="margin:4px 0 12px">{if audio { "What a good copy of an audiobook is." } else { "What a good copy of an ebook is." }}</div>
            <div class="small faint" style="font-weight:650;margin-bottom:6px">"File formats, best first"</div>
            <div class="stack" style="gap:4px">{rows}</div>
            <label class="field mt"><span>"Languages"</span><input class="input" prop:value=languages disabled=!admin placeholder="en, de" on:input=move |e| set_languages.set(event_target_value(&e))/></label>
            {audio.then(|| view! { <label class="row mt" style="gap:8px"><input type="checkbox" checked=unabridged disabled=!admin on:change=move |_| set_unabridged.update(|u| *u = !*u)/><span>"Unabridged only"</span></label> })}
            {admin.then(|| view! { <button class="btn primary mt" on:click=save>"Save"</button> })}
        </div>
    }
}

/// `#/wanted`: the wanted list, its preferences and Check followed now.
#[component]
pub fn Wanted(admin: bool) -> impl IntoView {
    let (format, set_format) = signal(String::new());
    let (reload, set_reload) = signal(0u32);
    let (checking, set_checking) = signal(false);
    let list = LocalResource::new(move || {
        let (f, _) = (format.get(), reload.get());
        async move { api::get(&format!("/wanted?format={f}")).await.map_err(|e| e.message) }
    });
    let profiles = LocalResource::new(|| async { api::get("/profiles").await.map_err(|e| e.message) });
    let on_change = move || set_reload.update(|n| *n += 1);
    let check = move |_| {
        set_checking.set(true);
        kit_leptos::rt::spawn(async move {
            match api::post_empty("/follow/check").await {
                Ok(r) => {
                    let added = r.get("added").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
                    let problems: Vec<String> = r
                        .get("problems")
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                        .unwrap_or_default();
                    toast(
                        &match added {
                            0 => "No new books from followed authors and series.".to_string(),
                            1 => "1 new book was added to the wanted list.".to_string(),
                            n => format!("{n} new books were added to the wanted list."),
                        },
                        "ok",
                    );
                    for p in problems {
                        toast(&p, "warn");
                    }
                    on_change();
                }
                Err(e) => toast(&e.message, "err"),
            }
            set_checking.set(false);
        })
    };
    let seg = move |value: &'static str, label: &'static str| view! { <button type="button" class:on=move || format.get() == value on:click=move |_| set_format.set(value.to_string())>{label}</button> };
    let rows = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.get("wanted").and_then(Value::as_array).cloned().unwrap_or_default();
            if items.is_empty() {
                view! { <div class="empty" id="wanted-empty"><p class="muted">"Nothing wanted. Want an edition from its book's page, or follow an author or series."</p></div> }.into_any()
            } else {
                view! { <div class="list" id="wanted-list">{items.iter().map(|w| wanted_row(w, admin, on_change)).collect_view()}</div> }
                    .into_any()
            }
        }
    };
    let cards = move || match profiles.get() {
        Some(Ok(v)) => {
            let known = |f: &str| -> Vec<String> {
                v.pointer(&format!("/file_formats/{f}"))
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default()
            };
            v.get("profiles")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|p| {
                    let k = known(&s(&p, "format"));
                    view! { <ProfileCard profile=p known=k admin=admin/> }
                })
                .collect_view()
                .into_any()
        }
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        None => ().into_any(),
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Wanted"</h1>
            <div class="row wrap" style="gap:10px">
                <div class="seg">{seg("", "All")}{seg("audiobook", "Audiobooks")}{seg("ebook", "Ebooks")}</div>
                {admin.then(|| view! { <button class="btn" id="follow-check" disabled=checking on:click=check inner_html=format!("{}Check followed now", icon("refresh"))></button> })}
            </div>
        </div>
        <div class="card flat">{rows}</div>
        <h2 class="mt">"Preferences"</h2>
        <div class="profile-grid mt">{cards}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_have_plain_words() {
        assert_eq!(status_label("review"), ("Needs a look", "warn"));
        assert_eq!(status_label("imported").0, "In the library");
        assert_eq!(FOLLOW_CHOICES[3].1, "Audiobooks and ebooks");
    }
}
