//! The library pages: every book (works) with search and a format filter, adding an edition by ASIN or ISBN,
//! one book with its exact editions, the authors and the series.

use crate::navigate;
use kit_leptos::api;
use kit_leptos::icons::icon;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// What a typed identifier is: `("isbn", digits)` for 10 or 13 digits (the last of ten may be X), otherwise
/// `("asin", capitals)`. Dashes and spaces are dropped.
pub fn guess_id(raw: &str) -> (&'static str, String) {
    let clean: String = raw.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let digits = clean.chars().all(|c| c.is_ascii_digit());
    let isbn10 = clean.len() == 10
        && clean[..9].chars().all(|c| c.is_ascii_digit())
        && clean.ends_with(|c: char| c.is_ascii_digit() || c == 'x' || c == 'X');
    if (digits && clean.len() == 13) || (isbn10 && !clean.to_ascii_uppercase().starts_with('B')) {
        ("isbn", clean.to_ascii_uppercase())
    } else {
        ("asin", clean.to_ascii_uppercase())
    }
}

/// A runtime in minutes as people say it: `"16 h 10 min"`, `"45 min"`.
pub fn runtime(min: u64) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Text without HTML tags and with the common entities read (the sources' blurbs are HTML).
pub fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The authors of a work as one line (`"Andy Weir"`, `"A and B"`, `"A, B and C"`).
pub fn names(list: &Value) -> String {
    let names: Vec<String> =
        list.as_array().map(|a| a.iter().map(|x| s(x, "name")).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
    match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n => format!("{} and {}", names[..n - 1].join(", "), names[n - 1]),
    }
}

/// The word people see for a format.
pub fn format_label(f: &str) -> &'static str {
    match f {
        "audiobook" => "Audiobook",
        "ebook" => "Ebook",
        _ => "Book",
    }
}

/// The series line of a work (`"The Stormlight Archive #1"`).
fn series_line(list: &Value) -> String {
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|x| if s(x, "position").is_empty() { s(x, "name") } else { format!("{} #{}", s(x, "name"), s(x, "position")) })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default()
}

/// A cover picture, or a plain tile with the title's first letter when there is none or it can't be loaded.
fn cover(url: String, title: String, class: &'static str) -> AnyView {
    let letter = title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let (failed, set_failed) = signal(url.is_empty());
    view! {
        {move || if failed.get() {
            view! { <div class=format!("{class} cover-blank")>{letter.clone()}</div> }.into_any()
        } else {
            view! { <img class=class src=url.clone() alt="" loading="lazy" on:error=move |_| set_failed.set(true)/> }.into_any()
        }}
    }
    .into_any()
}

/// The Add book card: an ASIN or an ISBN, looked up and kept; the book opens after.
#[component]
fn AddBook(on_added: impl Fn() + Copy + 'static) -> impl IntoView {
    let input = NodeRef::<leptos::html::Input>::new();
    let (busy, set_busy) = signal(false);
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let raw = input.get().map(|i| i.value()).unwrap_or_default();
        if raw.trim().is_empty() {
            return;
        }
        let (kind, id) = guess_id(&raw);
        set_busy.set(true);
        kit_leptos::rt::spawn(async move {
            match api::post("/library/editions", json!({ kind: id })).await {
                Ok(v) => {
                    let title = v.pointer("/work/title").and_then(Value::as_str).unwrap_or("The book").to_string();
                    let created = v.get("created").and_then(Value::as_bool).unwrap_or(false);
                    toast(
                        &if created {
                            format!("{title} was added.")
                        } else {
                            format!("{title} was already here; its details were refreshed.")
                        },
                        "ok",
                    );
                    if let Some(i) = input.get() {
                        i.set_value("");
                    }
                    on_added();
                    if let Some(id) = v.pointer("/work/id").and_then(Value::as_i64) {
                        navigate(&format!("/work?id={id}"));
                    }
                }
                Err(e) => toast(&e.message, "err"),
            }
            set_busy.set(false);
        });
    };
    view! {
        <form class="card row wrap" id="add-book" style="gap:10px" on:submit=submit>
            <div class="grow" style="min-width:220px">
                <div style="font-weight:650">"Add a book"</div>
                <div class="small muted">"An Audible ASIN adds that audiobook; an ISBN adds that ebook."</div>
            </div>
            <input class="input" style="max-width:260px" name="id" placeholder="B08G9PRS1K or 9780593135204" autocomplete="off" node_ref=input/>
            <button class="btn primary" type="submit" disabled=busy inner_html=format!("{}Add", icon("plus"))></button>
        </form>
    }
}

/// `#/`: every book, newest first, with search, a format filter and (for admins) Add a book.
#[component]
pub fn Library(admin: bool) -> impl IntoView {
    let (q, set_q) = signal(String::new());
    let (format, set_format) = signal(String::new());
    let (reload, set_reload) = signal(0u32);
    let list = LocalResource::new(move || {
        let (q, f, _) = (q.get(), format.get(), reload.get());
        async move {
            let path = format!("/library/works?q={}&format={f}", js_sys_encode(&q));
            api::get(&path).await.map_err(|e| e.message)
        }
    });
    let seg = move |value: &'static str, label: &'static str| {
        view! { <button type="button" class:on=move || format.get() == value on:click=move |_| set_format.set(value.to_string())>{label}</button> }
    };
    let grid = move || match list.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let works = v.get("works").and_then(Value::as_array).cloned().unwrap_or_default();
            if works.is_empty() {
                let text = if q.get().is_empty() && format.get().is_empty() {
                    "No books yet. Add one by its ASIN or ISBN."
                } else {
                    "No book matches."
                };
                view! { <div class="empty" id="library-empty"><p class="muted">{text}</p></div> }.into_any()
            } else {
                view! { <div class="book-grid" id="library-grid">{works.iter().map(work_tile).collect_view()}</div> }.into_any()
            }
        }
    };
    view! {
        <div class="row wrap between" style="margin-bottom:18px;gap:10px">
            <h1 style="margin:0">"Library"</h1>
            <div class="row wrap" style="gap:10px">
                <input class="input" type="search" placeholder="Title or author" style="max-width:240px" on:input=move |e| set_q.set(event_target_value(&e))/>
                <div class="seg">{seg("", "All")}{seg("audiobook", "Audiobooks")}{seg("ebook", "Ebooks")}</div>
            </div>
        </div>
        {admin.then(|| view! { <AddBook on_added=move || set_reload.update(|n| *n += 1)/> })}
        <div class="mt">{grid}</div>
    }
}

/// A query value for an address (letters, digits and `-_.~` as they are, the rest `%XX`).
fn js_sys_encode(v: &str) -> String {
    v.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// One book of the grid: cover, title, authors and its formats.
fn work_tile(w: &Value) -> impl IntoView {
    let id = w.get("id").and_then(Value::as_i64).unwrap_or(0);
    let formats: Vec<String> = w
        .get("formats")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    view! {
        <a class="book-tile" href=format!("#/work?id={id}")>
            {cover(s(w, "cover_url"), s(w, "title"), "book-cover")}
            <div class="book-title ellipsis">{s(w, "title")}</div>
            <div class="small muted ellipsis">{names(&w["authors"])}</div>
            <div class="row" style="gap:4px;margin-top:4px">{formats.into_iter().map(|f| view! { <span class="badge">{format_label(&f)}</span> }).collect_view()}</div>
        </a>
    }
}

/// One edition's row: what it is, how it is identified and the details that tell it from others.
fn edition_row(e: &Value, admin: bool, on_change: impl Fn() + Copy + 'static) -> impl IntoView {
    let id = e.get("id").and_then(Value::as_i64).unwrap_or(0);
    let format = s(e, "format");
    let ident = if format == "audiobook" { s(e, "asin") } else { s(e, "isbn13") };
    let narr: Vec<String> = e
        .get("narrators")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let length = match (e.get("runtime_min").and_then(Value::as_u64), e.get("pages").and_then(Value::as_u64)) {
        (Some(m), _) => runtime(m),
        (None, Some(p)) => format!("{p} pages"),
        _ => String::new(),
    };
    let abridged = match e.get("abridged").and_then(Value::as_bool) {
        Some(true) => "Abridged",
        Some(false) => "Unabridged",
        None => "",
    };
    let chapters = e.get("chapters").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let title = s(e, "title");
    let refresh = move |_| {
        kit_leptos::rt::spawn(async move {
            match api::post_empty(&format!("/library/editions/{id}/refresh")).await {
                Ok(_) => {
                    toast("The edition's details were refreshed.", "ok");
                    on_change();
                }
                Err(e) => toast(&e.message, "err"),
            }
        })
    };
    let want = move |_| {
        kit_leptos::rt::spawn(async move {
            match api::post("/wanted", json!({ "edition_id": id })).await {
                Ok(_) => toast("Added to the wanted list.", "ok"),
                Err(e) => toast(&e.message, "err"),
            }
        })
    };
    let remove = move |_| {
        let (title, ident) = (title.clone(), ident.clone());
        kit_leptos::rt::spawn(async move {
            let q = kit_rules::dialogs::Confirm::new("Remove this edition?")
                .text(format!("{title} ({ident}) leaves the library. You can add it again later by its ASIN or ISBN."))
                .ok("Remove")
                .danger(true);
            if kit_leptos::modal::confirm(q).await {
                match api::del(&format!("/library/editions/{id}")).await {
                    Ok(_) => on_change(),
                    Err(e) => toast(&e.message, "err"),
                }
            }
        })
    };
    let ident_shown = if format == "audiobook" { format!("ASIN {}", s(e, "asin")) } else { format!("ISBN {}", s(e, "isbn13")) };
    view! {
        <tr class="edition-row">
            <td><span class="badge">{format_label(&format)}</span></td>
            <td class="mono small">{ident_shown}</td>
            <td>{narr.join(", ")}</td>
            <td>{length}{(chapters > 0).then(|| view! { <div class="small faint">{format!("{chapters} chapters")}</div> })}</td>
            <td>{s(e, "language").to_uppercase()}</td>
            <td>{abridged}</td>
            <td>{s(e, "release_date")}<div class="small faint">{s(e, "publisher")}</div></td>
            <td style="white-space:nowrap">
                {crate::player::edition_actions(e)}
                {admin.then(|| view! {
                    <button class="btn sm" title="Want this edition" on:click=want>"Want"</button>
                    <button class="btn ghost icon sm" title="Refresh details" aria-label="Refresh details" on:click=refresh inner_html=icon("refresh")></button>
                    <button class="btn ghost icon sm" title="Remove" aria-label="Remove" on:click=remove inner_html=icon("trash")></button>
                }.into_any())}
            </td>
        </tr>
    }
}

/// `#/work?id=<id>`: one book with its authors, series, blurb and every exact edition.
#[component]
pub fn Work(id: i64, admin: bool) -> impl IntoView {
    let (reload, set_reload) = signal(0u32);
    let data = LocalResource::new(move || {
        let _ = reload.get();
        async move { api::get(&format!("/library/works/{id}")).await.map_err(|e| e.message) }
    });
    let on_change = move || set_reload.update(|n| *n += 1);
    move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div><a class="btn mt" href="#/">"Back to the library"</a> }.into_any(),
        Some(Ok(w)) => {
            let editions = w.get("editions").and_then(Value::as_array).cloned().unwrap_or_default();
            view! {
                <div class="row" style="gap:20px;align-items:flex-start;flex-wrap:wrap">
                    {cover(s(&w, "cover_url"), s(&w, "title"), "work-cover")}
                    <div class="grow" style="min-width:240px">
                        <h1 id="work-title">{s(&w, "title")}</h1>
                        <div class="muted" style="margin-top:6px">{names(&w["authors"])}</div>
                        <div class="small faint" style="margin-top:2px">{series_line(&w["series"])}</div>
                        <p class="work-blurb">{plain(&s(&w, "description"))}</p>
                    </div>
                </div>
                <h2 class="mt">"Editions"</h2>
                <div class="card flat mt table-wrap">
                    <table class="table" id="editions">
                        <thead><tr><th>"Format"</th><th>"Identifier"</th><th>"Narrator"</th><th>"Length"</th><th>"Language"</th><th>"Version"</th><th>"Released"</th><th></th></tr></thead>
                        <tbody>{editions.iter().map(|e| edition_row(e, admin, on_change)).collect_view()}</tbody>
                    </table>
                </div>
            }
            .into_any()
        }
    }
}

/// `#/authors`: every author and how many of their books are kept.
#[component]
pub fn Authors(admin: bool) -> impl IntoView {
    let data = LocalResource::new(|| async { api::get("/library/authors").await.map_err(|e| e.message) });
    let body = move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let list = v.get("authors").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                return view! { <div class="empty"><p class="muted">"No authors yet. They come with the books you add."</p></div> }
                    .into_any();
            }
            view! {
                <div class="list">{list.iter().map(|a| {
                    let n = a.get("works").and_then(Value::as_i64).unwrap_or(0);
                    let id = a.get("id").and_then(Value::as_i64).unwrap_or(0);
                    let follow = s(a, "follow");
                    view! {
                        <div class="list-item" style="gap:10px;flex-wrap:wrap">
                            <span class="grow" style="font-weight:650">{s(a, "name")}</span>
                            <span class="small muted">{if n == 1 { "1 book".to_string() } else { format!("{n} books") }}</span>
                            {admin.then(|| crate::wanted::follow_select("authors", id, follow))}
                        </div>
                    }
                }).collect_view()}</div>
            }
            .into_any()
        }
    };
    view! { <h1>"Authors"</h1><div class="card flat mt">{body}</div> }
}

/// `#/series`: every series with its books in reading order.
#[component]
pub fn Series(admin: bool) -> impl IntoView {
    let data = LocalResource::new(|| async { api::get("/library/series").await.map_err(|e| e.message) });
    let body = move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! { <div class="error-box">{e}</div> }.into_any(),
        Some(Ok(v)) => {
            let list = v.get("series").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                return view! { <div class="card"><p class="muted" style="margin:0">"No series yet. They come with the books you add."</p></div> }.into_any();
            }
            list.iter()
                .map(|x| {
                    let works = x.get("works").and_then(Value::as_array).cloned().unwrap_or_default();
                    let (id, follow) = (x.get("id").and_then(Value::as_i64).unwrap_or(0), s(x, "follow"));
                    view! {
                        <div class="card mt">
                            <div class="row between wrap" style="gap:10px"><h3>{s(x, "name")}</h3>{admin.then(|| crate::wanted::follow_select("series", id, follow))}</div>
                            <div class="list mt">{works.iter().map(|w| {
                                let id = w.get("id").and_then(Value::as_i64).unwrap_or(0);
                                let pos = s(w, "position");
                                view! { <a class="list-item" href=format!("#/work?id={id}")><span class="faint mono" style="min-width:3ch">{pos}</span><span class="grow">{s(w, "title")}</span></a> }
                            }).collect_view()}</div>
                        </div>
                    }
                })
                .collect_view()
                .into_any()
        }
    };
    view! { <h1>"Series"</h1>{body} }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ids_are_told_apart() {
        assert_eq!(guess_id("978-0-593-13520-4"), ("isbn", "9780593135204".into()));
        assert_eq!(guess_id("080442957x"), ("isbn", "080442957X".into()));
        assert_eq!(guess_id("b08g9prs1k"), ("asin", "B08G9PRS1K".into()));
        assert_eq!(guess_id("1234567890"), ("isbn", "1234567890".into()));
    }

    #[test]
    fn words_for_lengths_names_and_blurbs() {
        assert_eq!(runtime(970), "16 h 10 min");
        assert_eq!(runtime(45), "45 min");
        assert_eq!(runtime(120), "2 h");
        assert_eq!(plain("<p><b>Grace</b> &amp; the <i>stars</i></p>"), "Grace & the stars");
        assert_eq!(names(&json!([{"name": "A"}, {"name": "B"}, {"name": "C"}])), "A, B and C");
        assert_eq!(names(&json!([{"name": "A"}])), "A");
        assert_eq!(series_line(&json!([{"name": "Stormlight", "position": "1"}])), "Stormlight #1");
        assert_eq!(js_sys_encode("a b&c"), "a%20b%26c");
    }
}
