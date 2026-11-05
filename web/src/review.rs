//! `#/review`: downloads Folio isn't sure of. For each one it shows what was asked for next to what arrived,
//! fact by fact, and the reason in plain words (with Copy). Accept, Reject or Search again each take one click.

use kit_leptos::api;
use kit_leptos::clipboard::copy_text;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The badge for a check's outcome: its words and its class.
pub fn outcome_badge(outcome: &str) -> (&'static str, &'static str) {
    match outcome {
        "pass" => ("Matches", "badge ok"),
        "fail" => ("Doesn't match", "badge err"),
        _ => ("Can't tell", "badge warn"),
    }
}

/// The fact-by-fact table: what was asked for next to what arrived.
fn checks_table(verdict: &Value) -> impl IntoView {
    let checks = verdict.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
    view! {
        <table class="table review-checks">
            <thead><tr><th>"Fact"</th><th>"Requested"</th><th>"Arrived"</th><th></th></tr></thead>
            <tbody>
                {checks.into_iter().map(|c| {
                    let (label, class) = outcome_badge(&s(&c, "outcome"));
                    view! {
                        <tr>
                            <td>{s(&c, "name")}</td>
                            <td>{s(&c, "requested")}</td>
                            <td>{s(&c, "arrived")}</td>
                            <td><span class=class title=s(&c, "why")>{label}</span><div class="small muted">{s(&c, "why")}</div></td>
                        </tr>
                    }
                }).collect_view()}
            </tbody>
        </table>
    }
}

/// One download waiting for a look, with its three answers. `done` runs after an answer.
fn review_item(it: &Value, done: impl Fn() + Copy + Send + Sync + 'static) -> impl IntoView {
    let grab = it.get("grab").cloned().unwrap_or(Value::Null);
    let verdict = it.get("verdict").cloned().unwrap_or(Value::Null);
    let wanted = it.get("wanted").cloned().unwrap_or(Value::Null);
    let id = grab.get("id").and_then(Value::as_i64).unwrap_or(0);
    let summary = s(&verdict, "summary");
    let book = wanted.get("edition").map(|e| s(e, "title")).filter(|t| !t.is_empty()).unwrap_or_else(|| s(&grab, "title"));
    let confidence = verdict.get("confidence").and_then(Value::as_i64).unwrap_or(0);
    let answer = move |what: &'static str| {
        move |_| {
            kit_leptos::rt::spawn(async move {
                let path = format!("/review/{id}/{what}");
                let r = if what == "reject" { api::post(&path, json!({})).await } else { api::post_empty(&path).await };
                match r {
                    Ok(v) => {
                        let msg = match what {
                            "accept" => "Accepted. It will be filed.".to_string(),
                            "reject" => "Rejected. Folio won't grab it again.".to_string(),
                            _ if !v["grab"].is_null() => format!("Rejected, and grabbed \"{}\" instead.", s(&v["grab"], "title")),
                            _ => format!("Rejected. {}", s(&v, "note")),
                        };
                        toast(&msg, "ok");
                        done();
                    }
                    Err(e) => toast(&e.message, "err"),
                }
            })
        }
    };
    let copy = summary.clone();
    view! {
        <div class="card review-item">
            <div class="row wrap between" style="gap:10px">
                <div class="grow">
                    <h3 style="margin:0">{book}</h3>
                    <div class="mono small faint" style="word-break:break-all">{s(&grab, "title")}</div>
                </div>
                <span class="badge warn">{format!("{confidence}% sure")}</span>
            </div>
            <div class="row" style="gap:8px;align-items:flex-start;margin-top:10px">
                <p class="grow" style="margin:0">{summary}</p>
                <button class="btn ghost sm" title="Copy the reason" on:click=move |_| copy_text(&copy)>"Copy"</button>
            </div>
            <div class="mt">{checks_table(&verdict)}</div>
            <div class="row wrap mt" style="gap:8px">
                <button class="btn primary" on:click=answer("accept")>"Accept"</button>
                <button class="btn danger" on:click=answer("reject")>"Reject"</button>
                <button class="btn" on:click=answer("research")>"Reject and search again"</button>
            </div>
        </div>
    }
}

/// `#/review`: every download waiting for a look, newest first.
#[component]
pub fn Review() -> impl IntoView {
    let data = LocalResource::new(|| async { api::get("/review").await.map_err(|e| e.message) });
    let reload = move || data.refetch();
    let body = move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let items = v.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
            if items.is_empty() {
                view! { <div class="empty"><p class="muted">"Nothing to look at. Every download so far was the right edition."</p></div> }
                    .into_any()
            } else {
                view! { <div class="stack" id="review">{items.iter().map(|it| review_item(it, reload)).collect_view()}</div> }.into_any()
            }
        }
    };
    view! {
        <h1>"Review"</h1>
        <p class="muted">"Folio checks every download against the edition you asked for. These are the ones it isn't sure of: compare what was requested with what arrived, then decide."</p>
        <div class="mt">{body}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_read_plainly() {
        assert_eq!(outcome_badge("pass").0, "Matches");
        assert_eq!(outcome_badge("fail").1, "badge err");
        assert_eq!(outcome_badge("unknown").0, "Can't tell");
    }
}
