//! Move in (`#/migrate`, admins): pick the app you are coming from and where its database is, check it with a dry
//! run, read what would happen, then bring it in. Past runs are listed with their reports.

use kit_leptos::api;
use kit_leptos::details::CopyButton;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// A number field of a JSON object, 0 when missing.
fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// The words for what happened to a book (`dry`: the run only looked).
pub fn outcome_label(outcome: &str, dry: bool) -> &'static str {
    match outcome {
        "in_folio" => "In Folio",
        "add" => "Will be added",
        "added" if dry => "Will be added",
        "added" => "Added",
        "no_id" => "No ASIN or ISBN",
        "not_found" => "Not found",
        "refused" => "Refused",
        _ => "Unknown",
    }
}

/// Whether an outcome needs a person to look at it.
pub fn needs_you(outcome: &str) -> bool {
    matches!(outcome, "no_id" | "not_found" | "refused")
}

/// The badge class for an outcome.
fn badge(outcome: &str) -> &'static str {
    match outcome {
        "in_folio" | "added" | "add" => "badge ok",
        "refused" | "not_found" => "badge err",
        _ => "badge warn",
    }
}

/// The counts at the top of a report, as label and number, in reading order.
pub fn count_rows(counts: &Value, dry: bool) -> Vec<(&'static str, u64)> {
    vec![
        ("Books and wanted entries read", n(counts, "items")),
        ("Already in Folio", n(counts, "in_folio")),
        (if dry { "To be added" } else { "Added" }, n(counts, "added")),
        if dry {
            ("Books whose files will be checked and filed", n(counts, "files_queued"))
        } else {
            ("Books handed to the import to be checked and filed", n(counts, "files_handed"))
        },
        (if dry { "To go on the wanted list" } else { "Put on the wanted list" }, n(counts, "wanted")),
        (if dry { "Places to bring in" } else { "Places brought in" }, n(counts, "progress")),
        (if dry { "Bookmarks to bring in" } else { "Bookmarks brought in" }, n(counts, "bookmarks")),
        ("Need you, or left out", n(counts, "needs_you")),
    ]
}

/// One book or wanted entry of a report.
fn item_row(i: &Value, dry: bool) -> impl IntoView {
    let outcome = s(i, "outcome");
    let reason = s(i, "reason");
    let authors: Vec<String> = i
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let what = format!(
        "{} {}",
        if s(i, "format") == "audiobook" { "Audiobook" } else { "Ebook" },
        if s(i, "kind") == "wanted" { "(wanted)" } else { "" }
    );
    let facts = [what.trim().to_string(), authors.join(", "), s(i, "key"), s(i, "source_id")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    view! {
        <div class="list-item" style="gap:12px;flex-wrap:wrap;align-items:flex-start">
            <div class="grow" style="min-width:240px">
                <div style="font-weight:600">{s(i, "title")}</div>
                <div class="small faint">{facts}</div>
                {(!reason.is_empty()).then(|| { let r = reason.clone(); view! {
                    <div class="row small muted" style="gap:6px;margin-top:4px;align-items:flex-start"><span class="grow">{reason}</span><CopyButton text=r/></div>
                } })}
            </div>
            <span class=badge(&outcome)>{outcome_label(&outcome, dry)}</span>
        </div>
    }
}

/// A finished run's report: the counts, who goes to which account, every book (those that need you first), and
/// what was left out.
fn report_view(run: &Value) -> AnyView {
    let r = run.get("report").cloned().unwrap_or(Value::Null);
    let dry = run.get("dry_run").and_then(Value::as_bool) == Some(true);
    let counts = r.get("counts").cloned().unwrap_or(Value::Null);
    let mut items = r.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
    items.sort_by_key(|i| !needs_you(&s(i, "outcome")));
    let users = r.get("users").and_then(Value::as_array).cloned().unwrap_or_default();
    let skipped = r.get("skipped").and_then(Value::as_array).cloned().unwrap_or_default();
    view! {
        <div class="card flat mt" id="migrate-report">
            <div class="section-head"><h3>{if dry { "What would happen" } else { "What happened" }}</h3></div>
            <div class="table-wrap"><table class="table">
                <tbody>{count_rows(&counts, dry).into_iter().map(|(l, v)| view! { <tr><td>{l}</td><td class="mono">{v}</td></tr> }).collect_view()}</tbody>
            </table></div>
        </div>
        {(!users.is_empty()).then(|| view! {
            <div class="card flat mt">
                <div class="section-head"><h3>"People"</h3></div>
                <p class="small muted">"Each person's places and bookmarks go to the Folio account with the same user name. Make an account with that name, then run it again, to bring theirs in too."</p>
                <div class="list">{users.iter().map(|u| {
                    let to = s(u, "username");
                    view! {
                        <div class="list-item" style="gap:12px;flex-wrap:wrap">
                            <div class="grow"><div style="font-weight:600">{s(u, "name")}</div>
                                <div class="small faint">{format!("{} places, {} bookmarks", n(u, "progress"), n(u, "bookmarks"))}</div></div>
                            {if to.is_empty() { view! { <span class="badge warn">"No account"</span> }.into_any() } else { view! { <span class="badge ok">{format!("To {to}")}</span> }.into_any() }}
                        </div>
                    }
                }).collect_view()}</div>
            </div>
        })}
        <div class="card flat mt">
            <div class="section-head"><h3>"Books"</h3></div>
            {if items.is_empty() {
                view! { <div class="empty"><p class="muted">"No books or wanted entries in this source."</p></div> }.into_any()
            } else {
                view! { <div class="list" id="migrate-items">{items.iter().map(|i| item_row(i, dry)).collect_view()}</div> }.into_any()
            }}
        </div>
        {(!skipped.is_empty()).then(|| view! {
            <div class="card flat mt">
                <div class="section-head"><h3>"Left out"</h3></div>
                <div class="list">{skipped.iter().map(|x| { let why = s(x, "reason"); view! {
                    <div class="list-item" style="gap:8px;align-items:flex-start">
                        <div class="grow"><div class="small">{why.clone()}</div><div class="small faint mono">{s(x, "item")}</div></div>
                        <CopyButton text=why/>
                    </div>
                } }).collect_view()}</div>
            </div>
        })}
    }
    .into_any()
}

/// Waits for the run `id` to end, showing how far it got; then the run.
async fn follow(id: i64, set_run: WriteSignal<Option<Value>>) {
    loop {
        match api::get(&format!("/migrate/runs/{id}")).await {
            Ok(v) => {
                let running = s(&v, "status") == "running";
                set_run.set(Some(v));
                if !running {
                    return;
                }
            }
            Err(e) => {
                toast(&e.message, "err");
                return;
            }
        }
        kit_leptos::rt::sleep(1000).await;
    }
}

/// `#/migrate`: the source form, the run going now or last, and past runs.
#[component]
pub fn Migrate() -> impl IntoView {
    let sources = LocalResource::new(|| async { api::get("/migrate/sources").await.map_err(|e| e.message) });
    let (refresh, set_refresh) = signal(0u32);
    let past = LocalResource::new(move || {
        refresh.get();
        async { api::get("/migrate/runs").await.map_err(|e| e.message) }
    });
    let (source, set_source) = signal("abs".to_string());
    let (path, set_path) = signal(String::new());
    let (busy, set_busy) = signal(false);
    let (run, set_run) = signal::<Option<Value>>(None);

    let go = move |dry: bool| {
        let body = json!({ "source": source.get_untracked(), "path": path.get_untracked(), "dry_run": dry });
        kit_leptos::rt::spawn(async move {
            if !dry {
                let q = kit_rules::dialogs::Confirm::new("Bring it in now?")
                    .text("Folio adds the books, the wanted list, places and bookmarks shown in the check. The old app's files and database are not changed.")
                    .ok("Bring it in");
                if !kit_leptos::modal::confirm(q).await {
                    return;
                }
            }
            set_busy.set(true);
            match api::post("/migrate/runs", body).await {
                Ok(v) => {
                    let id = v.get("id").and_then(Value::as_i64).unwrap_or(0);
                    set_run.set(Some(v));
                    follow(id, set_run).await;
                    match run.get_untracked() {
                        Some(r) if s(&r, "status") == "failed" => toast(&s(&r, "error"), "err"),
                        Some(_) if dry => toast("The check is done. Read it below, then bring it in.", "ok"),
                        Some(_) => toast("Brought in.", "ok"),
                        None => {}
                    }
                }
                Err(e) => toast(&e.message, "err"),
            }
            set_busy.set(false);
            set_refresh.update(|n| *n += 1);
        })
    };

    let hint = move || {
        let list = sources.get().and_then(Result::ok).unwrap_or(Value::Null);
        let chosen = list.get("sources").and_then(Value::as_array).and_then(|a| a.iter().find(|x| s(x, "key") == source.get()).cloned());
        chosen.map(|c| {
            view! {
                <p class="small muted">{s(&c, "gives")}</p>
                <p class="small faint">{format!("Give the place of {} (or the folder that holds it) as Folio sees it. In Docker, mount the old app's folder into Folio's container, read-only.", s(&c, "file_name"))}</p>
            }
        })
    };
    let options = move || match sources.get() {
        Some(Ok(v)) => v
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|x| {
                let k = s(&x, "key");
                view! { <option value=k.clone() selected=move || source.get() == k>{s(&x, "name")}</option> }
            })
            .collect_view()
            .into_any(),
        Some(Err(e)) => view! { <option>{e}</option> }.into_any(),
        None => view! { <option>"…"</option> }.into_any(),
    };
    let current = move || {
        run.get().map(|r| {
            let status = s(&r, "status");
            let dry = r.get("dry_run").and_then(Value::as_bool) == Some(true);
            match status.as_str() {
                "running" => view! {
                    <div class="card flat mt"><div class="row" style="gap:10px"><span class="spinner"></span>
                        <span class="muted">{format!("{} {} of {} books…", if dry { "Checking" } else { "Bringing in" }, n(&r, "done"), n(&r, "total"))}</span></div></div>
                }
                .into_any(),
                "failed" => { let e = s(&r, "error"); view! { <div class="error-box mt row" style="gap:8px"><span class="grow">{e.clone()}</span><CopyButton text=e/></div> }.into_any() }
                _ => view! {
                    {report_view(&r)}
                    {dry.then(|| view! { <div class="row mt" style="gap:10px"><button class="btn primary" id="migrate-run" disabled=move || busy.get() on:click=move |_| go(false)>"Bring it in"</button></div> })}
                }
                .into_any(),
            }
        })
    };
    let history = move || match past.get() {
        Some(Ok(v)) => {
            let list = v.get("runs").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                return view! { <div class="empty"><p class="muted">"Nothing brought in yet."</p></div> }.into_any();
            }
            view! { <div class="list">{list.into_iter().map(|r| {
                let id = r.get("id").and_then(Value::as_i64).unwrap_or(0);
                let open = move |_| kit_leptos::rt::spawn(async move { follow(id, set_run).await });
                let kind = if r.get("dry_run").and_then(Value::as_bool) == Some(true) { "Check" } else { "Brought in" };
                view! {
                    <div class="list-item" style="gap:12px;flex-wrap:wrap">
                        <div class="grow" style="min-width:220px"><div style="font-weight:600">{format!("{kind} · {}", s(&r, "source"))}</div>
                            <div class="small faint mono ellipsis">{s(&r, "path")}</div></div>
                        <span class="badge">{match s(&r, "status").as_str() { "running" => "Running", "failed" => "Stopped", _ => "Done" }}</span>
                        <button class="btn sm" on:click=open>"Show"</button>
                    </div>
                }
            }).collect_view()}</div> }.into_any()
        }
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
    };

    view! {
        <h1>"Move in"</h1>
        <p class="muted">"Bring in your books, wanted lists and everyone's places from the app you used before. Check first: nothing is added until you say so, and the old app is never changed."</p>
        <div class="card flat mt">
            <div class="form-grid">
                <label class="field"><span>"Coming from"</span>
                    <select class="input" id="migrate-source" on:change=move |e| set_source.set(event_target_value(&e))>{options}</select></label>
                <label class="field"><span>"Where its database is"</span>
                    <input class="input mono" id="migrate-path" placeholder="/import/audiobookshelf/config/absdatabase.sqlite" autocomplete="off"
                        prop:value=move || path.get() on:input=move |e| set_path.set(event_target_value(&e))/></label>
            </div>
            {hint}
            <div class="row mt" style="gap:10px">
                <button class="btn primary" id="migrate-check" disabled=move || busy.get() || path.get().trim().is_empty() on:click=move |_| go(true)>"Check"</button>
            </div>
        </div>
        {current}
        <div class="section-head mt"><h3>"Earlier"</h3></div>
        <div class="card flat">{history}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_and_counts_read_plainly() {
        assert_eq!(outcome_label("added", true), "Will be added");
        assert_eq!(outcome_label("added", false), "Added");
        assert_eq!(outcome_label("no_id", false), "No ASIN or ISBN");
        assert!(needs_you("refused") && !needs_you("in_folio"));
        let rows = count_rows(&json!({ "items": 4, "added": 2, "progress": 3 }), true);
        assert_eq!(rows[2], ("To be added", 2));
        assert_eq!(rows[5], ("Places to bring in", 3));
        assert_eq!(count_rows(&json!({}), false)[2], ("Added", 0));
    }
}
