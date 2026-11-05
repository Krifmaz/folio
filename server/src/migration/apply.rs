//! Bringing a plan in: each book and wanted entry is matched to its exact edition, then (in a real run) the edition
//! is kept, wanted, given its people's progress and bookmarks, and its files are queued for the import.
//!
//! **Never a guess.** An audiobook is matched only by its Audible ASIN and an ebook only by its ISBN. A book without
//! one is listed as needing a person, with the reason, and nothing is added for it.
//!
//! A **dry run** does every check that needs nothing new: what is in Folio already, which ASINs and ISBNs are
//! valid, whose progress has an account to go to. It looks nothing up and writes nothing but its report.
//!
//! People are paired with Folio accounts by user name (any case), or as the run's `users` map says.

use super::store;
use crate::library::{self, ids, EditionId};
use crate::playback::store as playback;
use crate::wanted::store as wanted;
use crate::AppState;
use kit_server_core::{p, Db, Result, RowExt, SqliteDb};
use migrate::{Format, Plan};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// What to do with a plan.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Options {
    /// look and report only (true), or bring everything in (false)
    pub dry_run: bool,
    /// a source user name → the Folio account id their progress goes to (0: leave their progress out); names not
    /// in it go to the account with the same user name
    #[serde(default)]
    pub users: HashMap<String, i64>,
}

/// What happened to one book or wanted entry.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// the source's id for it
    pub source_id: String,
    /// `book` (has files) or `wanted`
    pub kind: &'static str,
    /// `audiobook` or `ebook`
    pub format: Format,
    /// its title
    pub title: String,
    /// its authors
    pub authors: Vec<String>,
    /// the ASIN or ISBN it is matched by ("" when it has none)
    pub key: String,
    /// `in_folio`, `add` (dry run: would be looked up and added), `added`, `no_id`, `not_found` or `refused`
    pub outcome: &'static str,
    /// why, in plain words, when it needs a person
    pub reason: String,
    /// the edition it is (or became)
    pub edition_id: Option<i64>,
}

/// How a source user was paired with a Folio account.
#[derive(Debug, Clone, Serialize)]
pub struct UserMatch {
    /// their user name in the source
    pub name: String,
    /// the Folio account, when there is one
    pub user_id: Option<i64>,
    /// that account's user name
    pub username: String,
    /// progress entries of theirs
    pub progress: usize,
    /// bookmarks of theirs
    pub bookmarks: usize,
}

/// The counts at the top of the report.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    /// books and wanted entries read
    pub items: usize,
    /// already in Folio
    pub in_folio: usize,
    /// added (or, in a dry run, to be looked up and added)
    pub added: usize,
    /// books whose files wait for the import
    pub files_queued: usize,
    /// put on the wanted list (dry run: to be)
    pub wanted: usize,
    /// progress entries brought in (dry run: to be)
    pub progress: usize,
    /// bookmarks brought in (dry run: to be)
    pub bookmarks: usize,
    /// books, wanted entries, progress and bookmarks that need a person, or were left out
    pub needs_you: usize,
}

/// A run's report: shown as the dry run, kept with the run.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// `abs`, `calibre` …
    pub source: migrate::Source,
    /// whether it only looked
    pub dry_run: bool,
    /// what the source holds
    pub summary: migrate::Summary,
    /// what this run did
    pub counts: Counts,
    /// every book and wanted entry, with what happened to it
    pub items: Vec<Item>,
    /// how the source's people were paired with accounts
    pub users: Vec<UserMatch>,
    /// what was left out (by the reader, and progress or bookmarks that had nowhere to go), with the reason
    pub skipped: Vec<migrate::Skipped>,
}

/// Where a source's book stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    /// it is this edition
    Edition(i64),
    /// a dry run would add it (its progress would come along)
    Later,
}

/// Brings `plan` in as run `run_id` (or only checks it, for a dry run) as the account `by`; the report.
pub async fn apply(st: &AppState, run_id: i64, plan: Plan, opts: &Options, by: i64) -> Result<Report> {
    let db = &st.db;
    let total = plan.books.len() + plan.wanted.len();
    let mut items = Vec::with_capacity(total);
    let mut counts = Counts { items: total, ..Counts::default() };
    let mut targets: HashMap<String, Target> = HashMap::new();
    let mut done = 0;

    for book in &plan.books {
        let (key, found) = edition_for(st, book.format, &book.asin, &book.isbn, opts.dry_run).await?;
        let item = Item {
            source_id: book.source_id.clone(),
            kind: "book",
            format: book.format,
            title: book.title.clone(),
            authors: book.authors.clone(),
            key,
            outcome: found.outcome,
            reason: found.reason,
            edition_id: found.edition,
        };
        match (found.edition, found.outcome) {
            (Some(e), _) => {
                targets.insert(book.source_id.clone(), Target::Edition(e));
                if !opts.dry_run {
                    store::remember(db, &book.source_id, e, run_id)?;
                    if store::queue_files(db, book, e, run_id)? {
                        counts.files_queued += 1;
                    }
                } else if no_files(db, e)? {
                    counts.files_queued += 1;
                }
            }
            (None, "add") => {
                targets.insert(book.source_id.clone(), Target::Later);
                counts.files_queued += 1;
            }
            _ => {}
        }
        tally(&mut counts, item.outcome);
        items.push(item);
        done += 1;
        store::step(db, run_id, done, total)?;
    }

    for w in &plan.wanted {
        let (key, found) = edition_for(st, w.format, &w.asin, &w.isbn, opts.dry_run).await?;
        let mut item = Item {
            source_id: w.source_id.clone(),
            kind: "wanted",
            format: w.format,
            title: w.title.clone(),
            authors: w.authors.clone(),
            key,
            outcome: found.outcome,
            reason: found.reason,
            edition_id: found.edition,
        };
        if let Some(e) = found.edition {
            if !opts.dry_run {
                store::remember(db, &w.source_id, e, run_id)?;
                match wanted::want(db, e, None, by, "migration") {
                    Ok(_) => counts.wanted += 1,
                    Err(why) => {
                        item.outcome = "refused";
                        item.reason = why.detail;
                    }
                }
            } else if no_files(db, e)? {
                counts.wanted += 1;
            }
        } else if item.outcome == "add" {
            counts.wanted += 1;
        }
        tally(&mut counts, item.outcome);
        items.push(item);
        done += 1;
        store::step(db, run_id, done, total)?;
    }

    let mut skipped = plan.skipped.clone();
    let users = pair_users(db, &plan, opts)?;
    let account: HashMap<&str, Option<i64>> = users.iter().map(|u| (u.name.as_str(), u.user_id)).collect();
    let target = |source_id: &str, targets: &HashMap<String, Target>| -> Result<Option<Target>> {
        if let Some(t) = targets.get(source_id) {
            return Ok(Some(*t));
        }
        Ok(store::edition_of(db, source_id)?.map(Target::Edition))
    };

    for pr in &plan.progress {
        let Some(user_id) = account.get(pr.user.as_str()).copied().flatten() else {
            skipped.push(left_out(&pr.book, format!("{}'s place is left out: no Folio account goes with that name.", pr.user)));
            continue;
        };
        match target(&pr.book, &targets)? {
            None => skipped.push(left_out(&pr.book, nowhere(plan.source, &pr.book, &pr.user))),
            Some(Target::Later) => counts.progress += 1,
            Some(Target::Edition(_)) if opts.dry_run => counts.progress += 1,
            Some(Target::Edition(e)) => {
                playback::save_progress(db, user_id, e, place(plan.source, pr))?;
                counts.progress += 1;
            }
        }
    }

    for b in &plan.bookmarks {
        let Some(user_id) = account.get(b.user.as_str()).copied().flatten() else {
            skipped.push(left_out(&b.book, format!("{}'s bookmark is left out: no Folio account goes with that name.", b.user)));
            continue;
        };
        match target(&b.book, &targets)? {
            None => skipped.push(left_out(&b.book, nowhere(plan.source, &b.book, &b.user))),
            Some(Target::Later) => counts.bookmarks += 1,
            Some(Target::Edition(e)) => {
                let have = playback::bookmarks(db, user_id, e)?.iter().any(|k| (k.position_ms - b.position_ms).abs() < 1000);
                if !have {
                    if !opts.dry_run {
                        let mark = playback::Bookmark { position_ms: b.position_ms, note: b.title.clone(), ..Default::default() };
                        playback::add_bookmark(db, user_id, e, mark)?;
                    }
                    counts.bookmarks += 1;
                }
            }
        }
    }

    counts.needs_you += skipped.len();
    Ok(Report { source: plan.source, dry_run: opts.dry_run, summary: plan.summary(), counts, items, users, skipped })
}

/// What matching one book found.
struct Found {
    /// see [`Item::outcome`]
    outcome: &'static str,
    /// see [`Item::reason`]
    reason: String,
    /// the edition, when it is in Folio (or now is)
    edition: Option<i64>,
}

/// Finds (and, in a real run, adds) the exact edition of a book: by ASIN for an audiobook, by ISBN for an ebook.
/// Answers the key it used and what happened.
async fn edition_for(st: &AppState, format: Format, asin: &str, isbn: &str, dry_run: bool) -> Result<(String, Found)> {
    let none = |reason: String| Found { outcome: "no_id", reason, edition: None };
    let (key, ask) = match format {
        Format::Audiobook => {
            match ids::asin(asin) {
                Some(a) => (a.clone(), EditionId { asin: a, ..Default::default() }),
                None => return Ok((
                    String::new(),
                    none(
                        "This audiobook has no Audible ASIN, so its exact edition can't be told. Add it by its ASIN on the Library page."
                            .into(),
                    ),
                )),
            }
        }
        Format::Ebook => match ids::isbn13(isbn) {
            Some(i) => (i.clone(), EditionId { isbn: i, ..Default::default() }),
            None => {
                return Ok((
                    String::new(),
                    none(
                        "This ebook has no valid ISBN, so its exact edition can't be told. Add it by its ISBN on the Library page.".into(),
                    ),
                ))
            }
        },
    };
    let kept = match format {
        Format::Audiobook => st.db.one("SELECT id FROM editions WHERE asin=?", p![key])?,
        Format::Ebook => st.db.one("SELECT id FROM editions WHERE format='ebook' AND isbn13=?", p![key])?,
    };
    if let Some(r) = kept {
        return Ok((key, Found { outcome: "in_folio", reason: String::new(), edition: Some(r.i("id")) }));
    }
    if dry_run {
        return Ok((key, Found { outcome: "add", reason: String::new(), edition: None }));
    }
    let found = match library::look_up(st, &ask).await {
        Ok(m) => match library::store::save(&st.db, &m) {
            Ok(saved) => Found { outcome: "added", reason: String::new(), edition: Some(saved.edition_id) },
            Err(e) => Found { outcome: "not_found", reason: e.detail, edition: None },
        },
        Err(e) => Found { outcome: "not_found", reason: e.detail, edition: None },
    };
    Ok((key, found))
}

/// Adds an outcome to the counts.
fn tally(c: &mut Counts, outcome: &str) {
    match outcome {
        "in_folio" => c.in_folio += 1,
        "add" | "added" => c.added += 1,
        _ => c.needs_you += 1,
    }
}

/// Whether the edition has no files in the library yet.
fn no_files(db: &SqliteDb, edition_id: i64) -> Result<bool> {
    Ok(db.one("SELECT id FROM edition_files WHERE edition_id=? LIMIT 1", p![edition_id])?.is_none())
}

/// Pairs each source user with a Folio account: the run's map first, then the same user name in any case.
fn pair_users(db: &SqliteDb, plan: &Plan, opts: &Options) -> Result<Vec<UserMatch>> {
    let accounts: Vec<(i64, String)> =
        db.query("SELECT id, username FROM users WHERE disabled=0", p![])?.iter().map(|r| (r.i("id"), r.s("username"))).collect();
    let mut names: Vec<&str> =
        plan.progress.iter().map(|p| p.user.as_str()).chain(plan.bookmarks.iter().map(|b| b.user.as_str())).collect();
    names.sort_unstable();
    names.dedup();
    Ok(names
        .into_iter()
        .map(|name| {
            let account = match opts.users.get(name) {
                Some(0) => None,
                Some(id) => accounts.iter().find(|(a, _)| a == id),
                None => accounts.iter().find(|(_, u)| u.eq_ignore_ascii_case(name)),
            };
            UserMatch {
                name: name.to_string(),
                user_id: account.map(|(id, _)| *id),
                username: account.map(|(_, u)| u.clone()).unwrap_or_default(),
                progress: plan.progress.iter().filter(|p| p.user == name).count(),
                bookmarks: plan.bookmarks.iter().filter(|b| b.user == name).count(),
            }
        })
        .collect())
}

/// A place from the source as Folio keeps it. A place without a time counts as old, so it never replaces one a
/// person made in Folio.
fn place(source: migrate::Source, p: &migrate::Progress) -> playback::Progress {
    let fraction = p.fraction.unwrap_or(if p.finished { 1.0 } else { 0.0 });
    let locator = match p.format {
        Format::Audiobook => Value::Null,
        Format::Ebook => {
            let mut l = json!({ "href": "", "progression": 0.0, "total_progression": fraction });
            if !p.cfi.is_empty() {
                l["cfi"] = json!(p.cfi);
            }
            l
        }
    };
    playback::Progress {
        edition_id: 0,
        position_ms: p.position_ms.unwrap_or(0).max(0),
        locator,
        fraction,
        finished: p.finished,
        device: format!("migration:{}", source.key()),
        updated_at: p.updated_at.filter(|t| *t > 0.0).unwrap_or(1.0),
    }
}

/// A left-out note for `item`.
fn left_out(item: &str, reason: String) -> migrate::Skipped {
    migrate::Skipped { item: item.to_string(), reason }
}

/// Why a person's place in `book` had nowhere to go.
fn nowhere(source: migrate::Source, book: &str, user: &str) -> String {
    if source == migrate::Source::CalibreWeb {
        format!("{user}'s place in Calibre book {book} is left out: bring in the Calibre library first, then Calibre-Web again.")
    } else {
        format!("{user}'s place in {book} is left out: that book wasn't brought in (see why in the list).")
    }
}
