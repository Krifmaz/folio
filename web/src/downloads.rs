//! Downloads: the results Lantern finds for a wanted book, scored with the reasons (Grab one by hand), and every
//! grab with how far it is.

use kit_leptos::api;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Bytes as people say them (`"520 MB"`, `"3.1 MB"`, `"1.2 GB"`).
pub fn size(bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if bytes == 0 {
        String::new()
    } else if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

/// Starts a search for the wanted entry `id` and says what came of it.
pub fn search_now(id: i64, after: impl Fn() + 'static) {
    kit_leptos::rt::spawn(async move {
        match api::post_empty(&format!("/wanted/{id}/search")).await {
            Ok(v) if !v["grab"].is_null() => toast(&format!("Grabbed \"{}\".", s(&v["grab"], "title")), "ok"),
            Ok(v) => toast(&s(&v, "note"), "warn"),
            Err(e) => toast(&e.message, "err"),
        }
        after();
    });
}

/// One scored result: its name, where from, size, seeders, score, the reasons, and Grab.
fn result_row(wanted: i64, r: &Value, min: i64) -> impl IntoView {
    let rel = r.get("release").cloned().unwrap_or(Value::Null);
    let score = r.get("score").and_then(Value::as_i64).unwrap_or(0);
    let refused = r.get("refused").and_then(Value::as_str).map(str::to_string);
    let reasons: Vec<String> = r
        .get("reasons")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let (guid, indexer) = (s(&rel, "guid"), rel.get("indexerId").and_then(Value::as_i64).unwrap_or(0));
    let force = refused.is_some();
    let grab = move |_| {
        let guid = guid.clone();
        kit_leptos::rt::spawn(async move {
            if force {
                let q = kit_rules::dialogs::Confirm::new("Grab a result Folio refused?")
                    .text("Folio thinks this isn't the edition you want. It will still be checked when it arrives.")
                    .ok("Grab anyway")
                    .danger(true);
                if !kit_leptos::modal::confirm(q).await {
                    return;
                }
            }
            match api::post(&format!("/wanted/{wanted}/grab"), json!({ "guid": guid, "indexer_id": indexer, "force": force })).await {
                Ok(_) => {
                    toast("Sent to Lantern.", "ok");
                    crate::navigate("/wanted");
                }
                Err(e) => toast(&e.message, "err"),
            }
        })
    };
    let class = if refused.is_some() {
        "badge err"
    } else if score >= min {
        "badge ok"
    } else {
        "badge warn"
    };
    let seeders = rel.get("seeders").and_then(Value::as_i64).map(|n| format!("{n} seeders")).unwrap_or_default();
    let why = refused.clone().unwrap_or_else(|| reasons.join(" "));
    view! {
        <div class="list-item release-row" style="gap:12px;flex-wrap:wrap;align-items:flex-start">
            <div class="grow" style="min-width:260px">
                <div class="mono small" style="word-break:break-all">{s(&rel, "title")}</div>
                <div class="small faint">{[s(&rel, "indexer"), size(rel.get("size").and_then(Value::as_u64).unwrap_or(0)), seeders].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")}</div>
                <div class="small muted" style="margin-top:4px">{why}</div>
            </div>
            <span class=class>{if refused.is_some() { "Refused".to_string() } else { format!("{score}") }}</span>
            <button class="btn sm" on:click=grab>"Grab"</button>
        </div>
    }
}

/// `#/releases?id=<wanted id>`: what Lantern finds for one wanted book, best first.
#[component]
pub fn Releases(id: i64) -> impl IntoView {
    let data = LocalResource::new(move || async move { api::get(&format!("/wanted/{id}/releases")).await.map_err(|e| e.message) });
    let body = move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span><p class="muted">"Asking Lantern…"</p></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let min = v.get("min_score").and_then(Value::as_i64).unwrap_or(70);
            let list = v.get("results").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                view! { <div class="empty"><p class="muted">"Lantern found nothing for this edition yet."</p></div> }.into_any()
            } else {
                view! { <div class="list" id="releases">{list.iter().map(|r| result_row(id, r, min)).collect_view()}</div> }.into_any()
            }
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Results"</h1>
            <a class="btn" href="#/wanted">"Back to Wanted"</a>
        </div>
        <p class="muted">"Every result scores up to 100 on its title, author, narrator, ASIN or ISBN, file format, language, size and seeders. Refused ones are the wrong thing whatever they score."</p>
        <div class="card flat mt">{body}</div>
    }
}

/// The words for a grab's status.
pub fn grab_label(status: &str) -> &'static str {
    match status {
        "queued" => "Waiting",
        "downloading" => "Downloading",
        "completed" => "Arrived",
        "failed" => "Failed",
        _ => "Unknown",
    }
}

/// `#/downloads`: every grab, newest first, with its progress.
#[component]
pub fn Downloads() -> impl IntoView {
    let data = LocalResource::new(|| async { api::get("/grabs").await.map_err(|e| e.message) });
    let body = move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let list = v.get("grabs").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                return view! { <div class="empty"><p class="muted">"Nothing downloaded yet."</p></div> }.into_any();
            }
            view! {
                <div class="list" id="grabs">{list.iter().map(|g| {
                    let pct = (g.get("progress").and_then(Value::as_f64).unwrap_or(0.0) * 100.0).round();
                    let status = s(g, "status");
                    let err = s(g, "error");
                    view! {
                        <div class="list-item" style="gap:12px;flex-wrap:wrap">
                            <div class="grow" style="min-width:240px">
                                <div class="mono small" style="word-break:break-all">{s(g, "title")}</div>
                                <div class="small faint">{[s(g, "indexer"), size(g.get("size").and_then(Value::as_u64).unwrap_or(0)), format!("score {}", g.get("score").and_then(Value::as_i64).unwrap_or(0))].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")}</div>
                                {(!err.is_empty()).then(|| view! { <div class="small" style="color:var(--err)">{err}</div> })}
                            </div>
                            <span class="small muted">{if status == "downloading" { format!("{pct}%") } else { String::new() }}</span>
                            <span class="badge">{grab_label(&status)}</span>
                        </div>
                    }
                }).collect_view()}</div>
            }
            .into_any()
        }
    };
    view! { <h1>"Downloads"</h1><div class="card flat mt">{body}</div> }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_labels() {
        assert_eq!(size(520_000_000), "520 MB");
        assert_eq!(size(3_100_000), "3.1 MB");
        assert_eq!(size(1_200_000_000), "1.2 GB");
        assert_eq!(size(0), "");
        assert_eq!(grab_label("completed"), "Arrived");
    }
}
