//! The audiobook player (`#/play?id=<edition id>`) and the Listen and Read buttons on an edition.
//!
//! The player plays an edition's audio files as one book: the time shown, the chapters and the place kept are all
//! counted from the start of the whole book, across its files (docs/TRACKS.md). It has speed, a sleep timer (after
//! some minutes or at the end of the chapter), bookmarks with notes, the chapter list, the browser's media keys and
//! lock screen controls, and keeps the place on the server: every 15 seconds while playing, on pause and on
//! leaving, so another device carries on from there. A place that can't be sent waits in the browser
//! ([`crate::offline_media`]).

use crate::offline_media::{self, audio_place, now_s};
use kit_leptos::api;
use kit_leptos::rt;
use kit_leptos::toasts::toast;
use leptos::prelude::*;
use serde_json::{json, Value};
use wasm_bindgen::JsCast;

/// A string field of a JSON object, "" when missing.
fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The buttons an edition's row shows: Listen for an audiobook, Read for an ebook.
pub fn edition_actions(edition: &Value) -> AnyView {
    let id = edition.get("id").and_then(Value::as_i64).unwrap_or(0);
    match edition.get("format").and_then(Value::as_str) {
        Some("audiobook") => {
            view! { <a class="btn sm" href=format!("#/play?id={id}") title="Listen to this audiobook">"Listen"</a> }.into_any()
        }
        Some("ebook") => view! { <a class="btn sm" href=format!("#/read?id={id}") title="Read this ebook">"Read"</a> }.into_any(),
        _ => ().into_any(),
    }
}

/// One audio file of the book, placed in the whole book.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// the file's id (`/api/files/{id}`)
    pub file_id: i64,
    /// where it starts in the whole book, in milliseconds
    pub start_ms: i64,
    /// how long it is, in milliseconds
    pub duration_ms: i64,
}

/// A chapter of the whole book.
#[derive(Debug, Clone, PartialEq)]
pub struct Chapter {
    /// its name
    pub title: String,
    /// where it starts in the whole book, in milliseconds
    pub start_ms: i64,
    /// where it ends in the whole book, in milliseconds
    pub end_ms: i64,
}

/// The audio files of `files` (the answer's `files` list) in play order, each placed after the ones before it.
pub fn tracks(files: &[Value]) -> Vec<Track> {
    let mut audio: Vec<&Value> = files.iter().filter(|f| f.get("kind").and_then(Value::as_str) == Some("audio")).collect();
    audio.sort_by_key(|f| f.get("position").and_then(Value::as_i64).unwrap_or(0));
    let mut start = 0;
    audio
        .into_iter()
        .map(|f| {
            let duration_ms = f.get("duration_ms").and_then(Value::as_i64).unwrap_or(0).max(0);
            let t = Track { file_id: f.get("id").and_then(Value::as_i64).unwrap_or(0), start_ms: start, duration_ms };
            start += duration_ms;
            t
        })
        .collect()
}

/// The whole book's length in milliseconds.
pub fn total_ms(tracks: &[Track]) -> i64 {
    tracks.last().map(|t| t.start_ms + t.duration_ms).unwrap_or(0)
}

/// The chapters of the whole book: each file's chapters moved by where the file starts. A file without chapters
/// is one chapter named after its place (`Part 2`); `fallback` (the edition's own chapters from Audnexus, counted
/// from the start of the book) is used when no file has any.
pub fn book_chapters(files: &[Value], tracks: &[Track], fallback: &[Value]) -> Vec<Chapter> {
    let chapter = |c: &Value, offset: i64| Chapter {
        title: s(c, "title"),
        start_ms: offset + c.get("start_ms").and_then(Value::as_i64).unwrap_or(0),
        end_ms: offset
            + c.get("end_ms")
                .and_then(Value::as_i64)
                .or_else(|| Some(c.get("start_ms")?.as_i64()? + c.get("length_ms")?.as_i64()?))
                .unwrap_or(0),
    };
    let mut out = Vec::new();
    for t in tracks {
        let list = files
            .iter()
            .find(|f| f.get("id").and_then(Value::as_i64) == Some(t.file_id))
            .and_then(|f| f.get("chapters"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if list.is_empty() {
            out.push(Chapter { title: String::new(), start_ms: t.start_ms, end_ms: t.start_ms + t.duration_ms });
        } else {
            out.extend(list.iter().map(|c| chapter(c, t.start_ms)));
        }
    }
    let named = out.iter().any(|c| !c.title.is_empty());
    if !named && !fallback.is_empty() {
        return fallback.iter().map(|c| chapter(c, 0)).collect();
    }
    let parts = out.len();
    for (i, c) in out.iter_mut().enumerate() {
        if c.title.is_empty() {
            c.title = if parts == 1 { "The whole book".into() } else { format!("Part {}", i + 1) };
        }
    }
    out
}

/// Which file holds the book's `ms` and how far into it: `(index, milliseconds into the file)`. Past the end is
/// the end of the last file.
pub fn locate(tracks: &[Track], ms: i64) -> (usize, i64) {
    let ms = ms.max(0);
    for (i, t) in tracks.iter().enumerate() {
        if ms < t.start_ms + t.duration_ms || i + 1 == tracks.len() {
            return (i, (ms - t.start_ms).clamp(0, t.duration_ms.max(0)));
        }
    }
    (0, 0)
}

/// The chapter playing at the book's `ms`.
pub fn chapter_at(chapters: &[Chapter], ms: i64) -> Option<usize> {
    chapters.iter().rposition(|c| c.start_ms <= ms)
}

/// A time as a clock: `"4:05"`, `"1:02:03"`.
pub fn clock(ms: i64) -> String {
    let t = ms.max(0) / 1000;
    let (h, m, s) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A time left as words: `"2 h 5 min left"`, `"12 min left"`, `"Less than a minute left"`.
pub fn time_left(ms: i64) -> String {
    let min = ms.max(0) / 60_000;
    match (min / 60, min % 60) {
        (0, 0) => "Less than a minute left".into(),
        (0, m) => format!("{m} min left"),
        (h, 0) => format!("{h} h left"),
        (h, m) => format!("{h} h {m} min left"),
    }
}

/// The speeds the player offers.
pub const SPEEDS: [f64; 10] = [0.75, 0.9, 1.0, 1.1, 1.2, 1.3, 1.5, 1.75, 2.0, 2.5];

/// A speed as people say it: `"1×"`, `"1.25×"`.
pub fn speed_label(x: f64) -> String {
    let t = format!("{x:.2}");
    format!("{}×", t.trim_end_matches('0').trim_end_matches('.'))
}

/// The sleep timer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sleep {
    /// keeps playing
    Off,
    /// stops when the browser's clock reaches this (milliseconds since 1970)
    At(u64),
    /// stops when the book reaches this (the end of the chapter it was set in), in milliseconds
    ChapterEnd(i64),
}

impl Sleep {
    /// Whether to stop now, at the clock's `now_ms` and the book's `book_ms`.
    pub fn due(self, now_ms: u64, book_ms: i64) -> bool {
        match self {
            Sleep::Off => false,
            Sleep::At(t) => now_ms >= t,
            Sleep::ChapterEnd(end) => book_ms >= end - 250,
        }
    }

    /// What the timer shows now: `"Stops in 14 min"`, `"Stops at the end of the chapter"`, "" when off.
    pub fn label(self, now_ms: u64) -> String {
        match self {
            Sleep::Off => String::new(),
            Sleep::At(t) => {
                let left = t.saturating_sub(now_ms) / 1000;
                if left >= 60 {
                    format!("Stops in {} min", left.div_ceil(60))
                } else {
                    format!("Stops in {left} s")
                }
            }
            Sleep::ChapterEnd(_) => "Stops at the end of the chapter".into(),
        }
    }
}

/// The sleep timer's choices: `(value of the select, words)`.
const SLEEP_CHOICES: [(&str, &str); 8] = [
    ("off", "Sleep timer off"),
    ("5", "5 minutes"),
    ("10", "10 minutes"),
    ("15", "15 minutes"),
    ("30", "30 minutes"),
    ("45", "45 minutes"),
    ("60", "1 hour"),
    ("chapter", "End of chapter"),
];

/// The `localStorage` key of the speed last chosen (one for every book).
const SPEED_KEY: &str = "folio.player.speed";

/// The speed last chosen in this browser, 1× when none.
fn saved_speed() -> f64 {
    rt::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(SPEED_KEY).ok().flatten())
        .and_then(|t| t.parse::<f64>().ok())
        .filter(|x| (0.5..=3.0).contains(x))
        .unwrap_or(1.0)
}

/// Keeps the speed for next time.
fn keep_speed(x: f64) {
    if let Some(s) = rt::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(SPEED_KEY, &x.to_string());
    }
}

/// How often the place is sent while playing, in milliseconds.
const SAVE_EVERY_MS: u64 = 15_000;

/// The skip back and forward buttons, in milliseconds.
const SKIP_BACK_MS: i64 = 15_000;
/// The skip forward button, in milliseconds.
const SKIP_AHEAD_MS: i64 = 30_000;

/// What the player loaded: the edition, its files and where to start.
#[derive(Clone)]
struct Loaded {
    /// the edition as the library answers it
    edition: Value,
    /// its audio files in the whole book
    tracks: Vec<Track>,
    /// the whole book's chapters
    chapters: Vec<Chapter>,
    /// where to start, in milliseconds
    start_ms: i64,
    /// when that place was made (Unix seconds), 0 for none
    start_at: f64,
    /// the person's bookmarks
    bookmarks: Vec<Value>,
    /// the edition's files as the server lists them
    files: Vec<Value>,
    /// `blob:` addresses of the files downloaded to this browser, by file id
    local: std::collections::HashMap<i64, String>,
}

/// Loads the edition, its files, the person's place (the newer of the server's and one waiting in this browser)
/// and their bookmarks.
async fn load(id: i64) -> Result<Loaded, String> {
    let edition = offline_media::json(&format!("/library/editions/{id}")).await?;
    if s(&edition, "format") != "audiobook" {
        return Err("This edition is an ebook. Open it with Read.".into());
    }
    let files = offline_media::json(&format!("/library/editions/{id}/files")).await?;
    let files = files.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let tracks = tracks(&files);
    if tracks.is_empty() {
        return Err("This audiobook has no files in the library yet. Once it is downloaded and checked, it plays here.".into());
    }
    let fallback = edition.get("chapters").and_then(Value::as_array).cloned().unwrap_or_default();
    let chapters = book_chapters(&files, &tracks, &fallback);
    let server = offline_media::json(&format!("/progress/{id}")).await.ok();
    let local = offline_media::pending(id);
    let place = match (server, local) {
        (Some(s), Some(l)) => Some(if offline_media::newer(&l, Some(&s)) { l } else { s }),
        (s, l) => s.or(l),
    };
    let finished = place.as_ref().and_then(|p| p.get("finished")).and_then(Value::as_bool) == Some(true);
    let start_ms = if finished { 0 } else { place.as_ref().and_then(|p| p.get("position_ms")).and_then(Value::as_i64).unwrap_or(0) };
    let start_at = place.as_ref().and_then(|p| p.get("updated_at")).and_then(Value::as_f64).unwrap_or(0.0);
    let bookmarks = offline_media::json(&format!("/editions/{id}/bookmarks"))
        .await
        .ok()
        .and_then(|v| v.get("bookmarks").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut local = std::collections::HashMap::new();
    if offline_media::is_downloaded(id) {
        for t in &tracks {
            if let Some(u) = offline_media::local_url(t.file_id).await {
                local.insert(t.file_id, u);
            }
        }
    }
    Ok(Loaded { edition, tracks, chapters, start_ms, start_at, bookmarks, files, local })
}

/// `#/play?id=<edition id>`: the player.
#[component]
pub fn Player(id: i64) -> impl IntoView {
    offline_media::watch();
    let data = LocalResource::new(move || load(id));
    move || match data.get() {
        None => view! { <div class="empty"><span class="spinner"></span></div> }.into_any(),
        Some(Err(e)) => view! {
            <div class="error-box" id="player-error">{e}</div>
            <a class="btn mt" href="javascript:history.back()">"Back"</a>
        }
        .into_any(),
        Some(Ok(l)) => view! { <Deck id=id loaded=l/> }.into_any(),
    }
}

/// The player itself, once the book is loaded.
#[component]
fn Deck(id: i64, loaded: Loaded) -> impl IntoView {
    let Loaded { edition, tracks, chapters, start_ms, start_at, bookmarks, files, local } = loaded;
    let local = StoredValue::new(local);
    let total = total_ms(&tracks);
    let tracks = StoredValue::new(tracks);
    let chapters = StoredValue::new(chapters);
    let audio = NodeRef::<leptos::html::Audio>::new();
    let (pos, set_pos) = signal(start_ms);
    let (cur, set_cur) = signal(usize::MAX);
    let (playing, set_playing) = signal(false);
    let (speed, set_speed) = signal(saved_speed());
    let (sleep, set_sleep) = signal(Sleep::Off);
    let (clock_now, set_clock_now) = signal(rt::now_ms());
    let (marks, set_marks) = signal(bookmarks);
    let (dragging, set_dragging) = signal(false);
    // where to go once the file that is loading can be played, and whether to play then
    let pending_seek = StoredValue::new(None::<(i64, bool)>);
    let last_sent = StoredValue::new(0u64);
    let known_at = StoredValue::new(start_at);

    let el = move || audio.get_untracked();
    // the downloaded copy when there is one, else the server's
    let url = move |i: usize| {
        tracks
            .with_value(|t| {
                t.get(i).map(|t| local.with_value(|l| l.get(&t.file_id).cloned()).unwrap_or_else(|| offline_media::file_url(t.file_id)))
            })
            .unwrap_or_default()
    };

    // sends the place now
    let save = move || {
        let place = audio_place(pos.get_untracked(), total);
        offline_media::remember(id, &place);
        known_at.set_value(place["updated_at"].as_f64().unwrap_or(0.0));
        last_sent.set_value(rt::now_ms());
        rt::spawn(async move {
            let _ = offline_media::save_progress(id, place).await;
        });
    };

    // goes to the book's `ms`, loading another file when it is in one; keeps playing when it was
    let seek = move |ms: i64| {
        let Some(a) = el() else { return };
        let ms = ms.clamp(0, total);
        let (i, off) = tracks.with_value(|t| locate(t, ms));
        set_pos.set(ms);
        if i != cur.get_untracked() {
            let play = !a.paused();
            pending_seek.set_value(Some((off, play)));
            set_cur.set(i);
            a.set_src(&url(i));
            a.load();
        } else {
            a.set_current_time(off as f64 / 1000.0);
        }
    };

    let toggle = move || {
        let Some(a) = el() else { return };
        if a.paused() {
            if let Ok(p) = a.play() {
                rt::spawn(async move {
                    if wasm_bindgen_futures::JsFuture::from(p).await.is_err() {
                        toast("The audio couldn't start. Press Play again.", "warn");
                    }
                });
            }
        } else {
            let _ = a.pause();
        }
    };
    // a move the person made: goes there and keeps it
    let jump = move |ms: i64| {
        seek(ms);
        save();
    };
    let skip = move |d: i64| jump(pos.get_untracked() + d);
    let chapter_jump = move |d: i64| {
        let p = pos.get_untracked();
        let target = chapters.with_value(|c| {
            let i = chapter_at(c, p).unwrap_or(0) as i64;
            // back goes to the start of this chapter first, unless it only just began
            let i = if d < 0 && c.get(i as usize).is_some_and(|ch| p - ch.start_ms > 3000) { i } else { i + d };
            c.get(i.clamp(0, c.len() as i64 - 1) as usize).map(|ch| ch.start_ms)
        });
        if let Some(t) = target {
            jump(t);
        }
    };

    // the first file, at the place kept
    Effect::new(move |_| {
        if audio.get().is_some() && cur.get_untracked() == usize::MAX {
            seek(start_ms);
        }
    });

    let on_meta = move |_| {
        let Some(a) = el() else { return };
        a.set_playback_rate(speed.get_untracked());
        if let Some((off, play)) = pending_seek.get_value() {
            pending_seek.set_value(None);
            a.set_current_time(off as f64 / 1000.0);
            if play {
                let _ = a.play();
            }
        }
    };
    let on_time = move |_| {
        let Some(a) = el() else { return };
        let i = cur.get_untracked();
        let start = tracks.with_value(|t| t.get(i).map(|t| t.start_ms).unwrap_or(0));
        if pending_seek.get_value().is_some() {
            return;
        }
        let ms = start + (a.current_time() * 1000.0) as i64;
        if !dragging.get_untracked() {
            set_pos.set(ms);
            if !a.paused() {
                offline_media::remember(id, &audio_place(ms, total));
            }
        }
        let now = rt::now_ms();
        set_clock_now.set(now);
        if sleep.get_untracked().due(now, ms) {
            set_sleep.set(Sleep::Off);
            let _ = a.pause();
            toast("The sleep timer stopped the book.", "info");
        }
        if !a.paused() && now.saturating_sub(last_sent.get_value()) >= SAVE_EVERY_MS {
            save();
        }
    };
    let on_ended = move |_| {
        let next = cur.get_untracked() + 1;
        if next < tracks.with_value(Vec::len) {
            pending_seek.set_value(Some((0, true)));
            set_cur.set(next);
            if let Some(a) = el() {
                a.set_src(&url(next));
                a.load();
            }
        } else {
            set_pos.set(total);
            set_playing.set(false);
            save();
            toast("You finished the book.", "ok");
        }
    };
    let on_error = move |_| {
        if pending_seek.get_value().is_some() || el().is_some_and(|a| a.error().is_some()) {
            toast("The audio file can't be played. Check that the server can reach the library folder, then try again.", "err");
        }
    };

    // the place goes to the server when the page is hidden or left
    let hide = rt::document().map(|d| {
        rt::listen(&d, "visibilitychange", move |_| {
            let hidden = rt::document().map(|d| d.hidden()).unwrap_or(false);
            if hidden {
                save();
            } else if !playing.get_untracked() {
                // back on the page: another device may have moved on
                rt::spawn(async move {
                    if let Ok(p) = api::get(&format!("/progress/{id}")).await {
                        let at = p.get("updated_at").and_then(Value::as_f64).unwrap_or(0.0);
                        if at > known_at.get_value() + 1.0 && !playing.get_untracked() {
                            known_at.set_value(at);
                            let ms = p.get("position_ms").and_then(Value::as_i64).unwrap_or(0);
                            if (ms - pos.get_untracked()).abs() > 5000 {
                                seek(ms);
                                toast(&format!("Moved to {}, where you were on another device.", clock(ms)), "info");
                            }
                        }
                    }
                });
            }
        })
    });
    let hide = StoredValue::new_local(hide);
    // leaving: only what is not reactive any more may be used here (parts of the page may be gone already)
    on_cleanup(move || {
        if let Some(a) = audio.try_get_untracked().flatten() {
            let _ = a.pause();
        }
        offline_media::send_latest(id);
        hide.try_update_value(|h| *h = None);
        clear_media_session();
        local.try_with_value(|l| {
            for u in l.values() {
                let _ = web_sys::Url::revoke_object_url(u);
            }
        });
    });

    // keys: space plays or pauses, the arrows skip, unless typing
    let keys = rt::document().map(|d| {
        rt::listen(&d, "keydown", move |e| {
            let Some(k) = e.dyn_ref::<web_sys::KeyboardEvent>() else { return };
            let typing = rt::target_el(e).is_some_and(|t| matches!(t.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT" | "BUTTON"));
            if typing || k.ctrl_key() || k.meta_key() || k.alt_key() {
                return;
            }
            match k.key().as_str() {
                " " => {
                    e.prevent_default();
                    toggle();
                }
                "ArrowLeft" => skip(-SKIP_BACK_MS),
                "ArrowRight" => skip(SKIP_AHEAD_MS),
                _ => {}
            }
        })
    });
    let keys = StoredValue::new_local(keys);
    on_cleanup(move || {
        keys.try_update_value(|k| *k = None);
    });

    set_media_session(&edition, toggle, skip);

    let edition_for_download = edition.clone();
    let title = s(&edition, "title");
    let author = crate::library::names(&edition["authors"]);
    let narrators = edition
        .get("narrators")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let cover_url = s(&edition, "cover_url");
    let work_id = edition.get("work_id").and_then(Value::as_i64).unwrap_or(0);

    let chapter_now = move || chapters.with_value(|c| chapter_at(c, pos.get()).and_then(|i| c.get(i).cloned()));
    let set_speed_to = move |x: f64| {
        set_speed.set(x);
        keep_speed(x);
        if let Some(a) = el() {
            a.set_playback_rate(x);
        }
    };
    let on_sleep = move |e: leptos::ev::Event| {
        let v = event_target_value(&e);
        let t = match v.as_str() {
            "off" => Sleep::Off,
            "chapter" => match chapters.with_value(|c| chapter_at(c, pos.get_untracked()).and_then(|i| c.get(i).map(|c| c.end_ms))) {
                Some(end) => Sleep::ChapterEnd(end),
                None => Sleep::Off,
            },
            m => m.parse::<u64>().map(|m| Sleep::At(rt::now_ms() + m * 60_000)).unwrap_or(Sleep::Off),
        };
        set_sleep.set(t);
        set_clock_now.set(rt::now_ms());
    };
    let sleep_value = move || match sleep.get() {
        Sleep::Off => "off".to_string(),
        Sleep::ChapterEnd(_) => "chapter".to_string(),
        Sleep::At(_) => "set".to_string(),
    };

    let add_mark = move |_| {
        let at = pos.get_untracked();
        let note = chapters.with_value(|c| chapter_at(c, at).and_then(|i| c.get(i).map(|c| c.title.clone()))).unwrap_or_default();
        rt::spawn(async move {
            match api::post(&format!("/editions/{id}/bookmarks"), json!({ "position_ms": at, "note": note, "created_at": now_s() })).await {
                Ok(b) => {
                    set_marks.update(|m| {
                        m.push(b);
                        m.sort_by_key(|b| b.get("position_ms").and_then(Value::as_i64).unwrap_or(0));
                    });
                    toast(&format!("Bookmark added at {}.", clock(at)), "ok");
                }
                Err(e) => toast(&format!("The bookmark wasn't kept: {}", e.message), "err"),
            }
        });
    };

    view! {
        <div class="player" id="player">
            <audio node_ref=audio preload="metadata" on:loadedmetadata=on_meta on:timeupdate=on_time on:ended=on_ended
                on:error=on_error on:play=move |_| set_playing.set(true) on:pause=move |_| { set_playing.set(false); save(); }></audio>
            <div class="player-head">
                <crate::player::Cover url=cover_url title=title.clone()/>
                <div class="grow">
                    <a class="small muted" href=format!("#/work?id={work_id}")>"Back to the book"</a>
                    <h1 id="player-title">{title}</h1>
                    <div class="muted">{author}</div>
                    {(!narrators.is_empty()).then(|| view! { <div class="small faint">{format!("Narrated by {narrators}")}</div> })}
                    <div class="player-chapter" id="player-chapter">{move || chapter_now().map(|c| c.title).unwrap_or_default()}</div>
                </div>
            </div>
            <div class="player-seek">
                <input class="player-range" type="range" min="0" max=total.to_string() step="1000" aria-label="Place in the book"
                    prop:value=move || pos.get().to_string()
                    on:input=move |e| { set_dragging.set(true); set_pos.set(event_target_value(&e).parse().unwrap_or(0)); }
                    on:change=move |e| { set_dragging.set(false); jump(event_target_value(&e).parse().unwrap_or(0)); }/>
                <div class="row between small muted">
                    <span id="player-pos">{move || clock(pos.get())}</span>
                    <span>{move || time_left(((total - pos.get()) as f64 / speed.get()) as i64)}</span>
                    <span>{clock(total)}</span>
                </div>
            </div>
            <div class="player-controls">
                <button class="btn ghost" title="Previous chapter" on:click=move |_| chapter_jump(-1)>"Previous"</button>
                <button class="btn ghost" title="Back 15 seconds" on:click=move |_| skip(-SKIP_BACK_MS)>"−15 s"</button>
                <button class="btn primary player-play" id="player-play" on:click=move |_| toggle()>
                    {move || if playing.get() { "Pause" } else { "Play" }}
                </button>
                <button class="btn ghost" title="Ahead 30 seconds" on:click=move |_| skip(SKIP_AHEAD_MS)>"+30 s"</button>
                <button class="btn ghost" title="Next chapter" on:click=move |_| chapter_jump(1)>"Next"</button>
            </div>
            <div class="player-options">
                <label class="row small muted">"Speed"
                    <select class="input" id="player-speed" on:change=move |e| set_speed_to(event_target_value(&e).parse().unwrap_or(1.0))>
                        {SPEEDS.iter().map(|x| { let x = *x; view! { <option value=x.to_string() selected=move || (speed.get() - x).abs() < 0.001>{speed_label(x)}</option> } }).collect_view()}
                    </select>
                </label>
                <label class="row small muted">"Sleep"
                    <select class="input" id="player-sleep" on:change=on_sleep prop:value=sleep_value>
                        {SLEEP_CHOICES.iter().map(|(v, w)| view! { <option value=*v>{*w}</option> }).collect_view()}
                        <option value="set" hidden=true>"Timer set"</option>
                    </select>
                </label>
                <span class="small muted" id="player-sleep-left">{move || sleep.get().label(clock_now.get())}</span>
                <button class="btn" id="player-bookmark" on:click=add_mark>"Add bookmark"</button>
                <offline_media::DownloadButton edition=edition_for_download files=files/>
            </div>
            <div class="player-lists">
                <section class="card">
                    <h2>"Chapters"</h2>
                    <div class="list player-list" id="player-chapters">
                        {chapters.get_value().into_iter().enumerate().map(|(i, c)| {
                            let at = c.start_ms;
                            let on = move || chapters.with_value(|cs| chapter_at(cs, pos.get())) == Some(i);
                            view! {
                                <button class="list-item player-item" class:on=on on:click=move |_| jump(at)>
                                    <span class="grow ellipsis">{c.title.clone()}</span>
                                    <span class="small faint">{clock(c.start_ms)}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </section>
                <section class="card">
                    <h2>"Bookmarks"</h2>
                    {move || {
                        let list = marks.get();
                        if list.is_empty() {
                            view! { <p class="small muted">"No bookmarks yet. Add one with Add bookmark while you listen."</p> }.into_any()
                        } else {
                            view! { <div class="list player-list" id="player-bookmarks">{list.into_iter().map(|b| bookmark_row(b, jump, set_marks)).collect_view()}</div> }.into_any()
                        }
                    }}
                </section>
            </div>
        </div>
    }
}

/// One bookmark: its time and note; a click goes there; the note can be changed; it can be deleted.
fn bookmark_row(b: Value, seek: impl Fn(i64) + Copy + 'static, set_marks: WriteSignal<Vec<Value>>) -> impl IntoView {
    let bid = b.get("id").and_then(Value::as_i64).unwrap_or(0);
    let at = b.get("position_ms").and_then(Value::as_i64).unwrap_or(0);
    let note = s(&b, "note");
    let edit = move |e: leptos::ev::Event| {
        let note = event_target_value(&e);
        rt::spawn(async move {
            if let Err(e) = api::patch(&format!("/bookmarks/{bid}"), json!({ "note": note })).await {
                toast(&format!("The note wasn't kept: {}", e.message), "err");
            }
        });
    };
    let remove = move |_| {
        rt::spawn(async move {
            match api::del(&format!("/bookmarks/{bid}")).await {
                Ok(_) => set_marks.update(|m| m.retain(|x| x.get("id").and_then(Value::as_i64) != Some(bid))),
                Err(e) => toast(&format!("The bookmark wasn't deleted: {}", e.message), "err"),
            }
        });
    };
    view! {
        <div class="list-item player-item">
            <button class="btn ghost sm" title="Go to this bookmark" on:click=move |_| seek(at)>{clock(at)}</button>
            <input class="input grow" value=note placeholder="Note" aria-label="Note" on:change=edit/>
            <button class="btn ghost icon sm" title="Delete this bookmark" aria-label="Delete this bookmark" on:click=remove inner_html=kit_leptos::icons::icon("trash")></button>
        </div>
    }
}

/// A book's cover, or a tile with the title's first letter when there is none or it can't be loaded.
#[component]
pub fn Cover(url: String, title: String) -> impl IntoView {
    let letter = title.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let (failed, set_failed) = signal(url.is_empty());
    move || {
        if failed.get() {
            view! { <div class="player-cover cover-blank">{letter.clone()}</div> }.into_any()
        } else {
            view! { <img class="player-cover" src=url.clone() alt="" on:error=move |_| set_failed.set(true)/> }.into_any()
        }
    }
}

/// `navigator.mediaSession`, when the browser has it.
fn media_session() -> Option<js_sys::Object> {
    let nav = rt::window()?.navigator();
    let ms = js_sys::Reflect::get(&nav, &"mediaSession".into()).ok()?;
    ms.dyn_into::<js_sys::Object>().ok()
}

/// `mediaSession.setActionHandler(action, f)` (`f` null takes the handler off).
fn set_action(ms: &js_sys::Object, action: &str, f: &wasm_bindgen::JsValue) {
    if let Ok(set) = js_sys::Reflect::get(ms, &"setActionHandler".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) {
        // a browser that doesn't know an action throws: that action just isn't offered
        let _ = set.call2(ms, &action.into(), f);
    }
}

/// The browser's media keys and lock screen: the book's title and cover, play, pause and skipping.
fn set_media_session(edition: &Value, toggle: impl Fn() + Copy + 'static, skip: impl Fn(i64) + Copy + 'static) {
    let Some(ms) = media_session() else { return };
    let meta = js_sys::Object::new();
    let set = |k: &str, v: &wasm_bindgen::JsValue| {
        let _ = js_sys::Reflect::set(&meta, &k.into(), v);
    };
    set("title", &s(edition, "title").into());
    set("artist", &crate::library::names(&edition["authors"]).into());
    let cover = s(edition, "cover_url");
    if !cover.is_empty() {
        let img = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&img, &"src".into(), &cover.into());
        set("artwork", &js_sys::Array::of1(&img).into());
    }
    let made = rt::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"MediaMetadata".into()).ok())
        .and_then(|c| c.dyn_into::<js_sys::Function>().ok())
        .and_then(|c| js_sys::Reflect::construct(&c, &js_sys::Array::of1(&meta)).ok());
    if let Some(m) = made {
        let _ = js_sys::Reflect::set(&ms, &"metadata".into(), &m);
    }
    let handler = |f: Box<dyn Fn()>| {
        let c = wasm_bindgen::closure::Closure::<dyn Fn()>::new(f);
        let r: wasm_bindgen::JsValue = c.as_ref().clone();
        c.forget();
        r
    };
    set_action(&ms, "play", &handler(Box::new(toggle)));
    set_action(&ms, "pause", &handler(Box::new(toggle)));
    set_action(&ms, "seekbackward", &handler(Box::new(move || skip(-SKIP_BACK_MS))));
    set_action(&ms, "seekforward", &handler(Box::new(move || skip(SKIP_AHEAD_MS))));
}

/// Takes the book off the browser's media keys and lock screen.
fn clear_media_session() {
    let Some(ms) = media_session() else { return };
    let _ = js_sys::Reflect::set(&ms, &"metadata".into(), &wasm_bindgen::JsValue::NULL);
    for a in ["play", "pause", "seekbackward", "seekforward"] {
        set_action(&ms, a, &wasm_bindgen::JsValue::NULL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<Value> {
        vec![
            json!({ "id": 7, "kind": "audio", "position": 1, "duration_ms": 30_000, "chapters": [] }),
            json!({ "id": 9, "kind": "cover", "position": 0, "duration_ms": 0 }),
            json!({ "id": 5, "kind": "audio", "position": 0, "duration_ms": 60_000,
                    "chapters": [{ "title": "One", "start_ms": 0, "end_ms": 40_000 }, { "title": "Two", "start_ms": 40_000, "end_ms": 60_000 }] }),
        ]
    }

    #[test]
    fn files_play_in_order_as_one_book() {
        let t = tracks(&files());
        assert_eq!(t.iter().map(|t| (t.file_id, t.start_ms)).collect::<Vec<_>>(), vec![(5, 0), (7, 60_000)]);
        assert_eq!(total_ms(&t), 90_000);
        assert_eq!(locate(&t, 0), (0, 0));
        assert_eq!(locate(&t, 59_999), (0, 59_999));
        assert_eq!(locate(&t, 60_000), (1, 0));
        assert_eq!(locate(&t, 75_000), (1, 15_000));
        assert_eq!(locate(&t, 999_999), (1, 30_000));
        assert_eq!(locate(&t, -5), (0, 0));
    }

    #[test]
    fn chapters_count_from_the_start_of_the_book() {
        let f = files();
        let c = book_chapters(&f, &tracks(&f), &[]);
        let got: Vec<(&str, i64, i64)> = c.iter().map(|c| (c.title.as_str(), c.start_ms, c.end_ms)).collect();
        assert_eq!(got, vec![("One", 0, 40_000), ("Two", 40_000, 60_000), ("Part 3", 60_000, 90_000)]);
        assert_eq!(chapter_at(&c, 45_000), Some(1));
        assert_eq!(chapter_at(&c, 89_000), Some(2));
        // no file has chapters: the edition's own (Audnexus: start and length)
        let bare = vec![json!({ "id": 1, "kind": "audio", "duration_ms": 10_000 })];
        let fb = vec![
            json!({ "title": "Opening", "start_ms": 0, "length_ms": 4_000 }),
            json!({ "title": "Ch 1", "start_ms": 4_000, "length_ms": 6_000 }),
        ];
        let c = book_chapters(&bare, &tracks(&bare), &fb);
        assert_eq!((c[1].title.as_str(), c[1].start_ms, c[1].end_ms), ("Ch 1", 4_000, 10_000));
        let c = book_chapters(&bare, &tracks(&bare), &[]);
        assert_eq!(c[0].title, "The whole book");
    }

    #[test]
    fn times_read_like_a_clock_and_in_words() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_000), "1:05");
        assert_eq!(clock(3_723_000), "1:02:03");
        assert_eq!(time_left(30_000), "Less than a minute left");
        assert_eq!(time_left(12 * 60_000), "12 min left");
        assert_eq!(time_left(125 * 60_000), "2 h 5 min left");
        assert_eq!(speed_label(1.0), "1×");
        assert_eq!(speed_label(1.75), "1.75×");
        assert_eq!(speed_label(0.9), "0.9×");
    }

    #[test]
    fn the_sleep_timer_stops_on_time_or_at_the_chapter_end() {
        assert!(!Sleep::Off.due(u64::MAX, i64::MAX));
        assert!(!Sleep::At(1000).due(999, 0));
        assert!(Sleep::At(1000).due(1000, 0));
        assert!(Sleep::ChapterEnd(40_000).due(0, 39_800));
        assert!(!Sleep::ChapterEnd(40_000).due(0, 30_000));
        assert_eq!(Sleep::At(10 * 60_000).label(0), "Stops in 10 min");
        assert_eq!(Sleep::At(30_000).label(0), "Stops in 30 s");
        assert_eq!(Sleep::Off.label(0), "");
    }
}
