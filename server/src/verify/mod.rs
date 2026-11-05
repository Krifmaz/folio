//! Verification before import: Folio never guesses. What arrived is checked against the exact edition that was
//! asked for, fact by fact, and a confidence score decides. A high score is verified (the import files it); any
//! other result goes to the Review queue. There, someone sees what was asked for next to what arrived, with
//! the reason in plain words, and accepts, rejects or searches again.
//!
//! - Audiobooks ([`audio`]): the runtime is within 3 %, and the narrator, language and chapter count match.
//!   Samples and abridged copies are caught.
//! - Ebooks ([`epub`]): the book's own OPF metadata (ISBN, title, author, language) matches. The file is a
//!   valid EPUB without DRM, and its length fits the edition.
//!
//! ```text
//! GET  /api/review                     (admins) what needs a look: {items: [{grab, wanted, edition, verdict}]}
//! POST /api/review/{grab_id}/accept    (admins) it is the right book: verified
//! POST /api/review/{grab_id}/reject    (admins) {reason?} it isn't: never grab it again for this book
//! POST /api/review/{grab_id}/research  (admins) reject it and search again at once
//! POST /api/review/{grab_id}/verify    (admins) check the files again (after fixing something by hand)
//! ```

pub mod audio;
pub mod epub;

use crate::lantern::grabs::{self, wanted_parts};
use crate::lantern::GrabFile;
use crate::metadata::Format;
use crate::wanted::store as wanted;
use crate::AppState;
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use kit_server_core::db::now;
use kit_server_core::{err, p, Admin, Db, Result, RowExt, SqliteDb};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// What one check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// it matches
    Pass,
    /// it doesn't match
    Fail,
    /// the files don't say, so Folio can't tell (counts half, never as a match)
    Unknown,
}

/// One fact checked: what was asked for, what arrived and what that means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    /// what is checked, as people say it (`"Runtime"`)
    pub name: String,
    /// what the edition says
    pub requested: String,
    /// what the files say
    pub arrived: String,
    /// whether they match
    pub outcome: Outcome,
    /// how much this fact counts toward the confidence (all of a format's checks add up to 100)
    pub weight: u32,
    /// a failure here means it is the wrong thing, whatever the score
    pub hard: bool,
    /// the finding in a plain sentence
    pub why: String,
}

impl Check {
    /// A check of `name` (`requested` against `arrived`) that counts `weight`.
    pub fn new(name: &str, weight: u32, requested: impl Into<String>, arrived: impl Into<String>) -> Check {
        Check {
            name: name.into(),
            requested: requested.into(),
            arrived: arrived.into(),
            outcome: Outcome::Unknown,
            weight,
            hard: false,
            why: String::new(),
        }
    }

    /// It matches, because `why`.
    pub fn pass(mut self, why: impl Into<String>) -> Check {
        self.outcome = Outcome::Pass;
        self.why = why.into();
        self
    }

    /// It doesn't match, because `why`.
    pub fn fail(mut self, why: impl Into<String>) -> Check {
        self.outcome = Outcome::Fail;
        self.why = why.into();
        self
    }

    /// Folio can't tell, because `why`.
    pub fn unknown(mut self, why: impl Into<String>) -> Check {
        self.outcome = Outcome::Unknown;
        self.why = why.into();
        self
    }

    /// A failure of this check means the wrong thing arrived.
    pub fn hard(mut self) -> Check {
        self.hard = true;
        self
    }
}

/// The verdict on what arrived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Verdict {
    /// 0 to 100: matches count fully and facts the files don't give count half
    pub confidence: u32,
    /// `import` (verified) or `review` (someone has to look)
    pub decision: String,
    /// the reason in plain words: what matched, or what is wrong or missing
    pub summary: String,
    /// every check
    pub checks: Vec<Check>,
}

/// The verdict on `checks`: verified when nothing that rules it out failed and the confidence reaches
/// `min_confidence`, otherwise Review.
pub fn decide(checks: Vec<Check>, min_confidence: u32) -> Verdict {
    let total: u32 = checks.iter().map(|c| c.weight).sum();
    let got: u32 = checks
        .iter()
        .map(|c| match c.outcome {
            Outcome::Pass => c.weight * 2,
            Outcome::Unknown => c.weight,
            Outcome::Fail => 0,
        })
        .sum();
    let confidence = if total == 0 { 0 } else { (got * 100 + total) / (total * 2) };
    let wrong: Vec<&Check> = checks.iter().filter(|c| c.hard && c.outcome == Outcome::Fail).collect();
    let import = wrong.is_empty() && confidence >= min_confidence && total > 0;
    let summary = if import {
        let ok: Vec<String> = checks.iter().filter(|c| c.outcome == Outcome::Pass && c.weight > 0).map(|c| c.name.to_lowercase()).collect();
        format!("It is the edition you asked for: {} match ({confidence}% sure).", list(&ok))
    } else if !wrong.is_empty() {
        format!("This isn't the edition you asked for. {}", wrong.iter().map(|c| c.why.as_str()).collect::<Vec<_>>().join(" "))
    } else {
        let doubts: Vec<&str> = checks.iter().filter(|c| c.outcome != Outcome::Pass).map(|c| c.why.as_str()).collect();
        format!(
            "Folio is only {confidence}% sure this is the edition you asked for ({min_confidence}% is needed). {}",
            if doubts.is_empty() { "Nothing could be checked.".to_string() } else { doubts.join(" ") }
        )
    };
    Verdict { confidence, decision: if import { "import" } else { "review" }.into(), summary, checks }
}

/// Words joined as people list them (`"a, b and c"`).
pub fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The file types each format takes, by extension.
pub fn takes(f: Format, path: &str) -> bool {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match f {
        Format::Audiobook => audio::EXTENSIONS.contains(&ext.as_str()),
        Format::Ebook => epub::EXTENSIONS.contains(&ext.as_str()),
    }
}

/// Checks the files of the finished grab `grab_id` and records the verdict: the grab and its wanted entry
/// become `verified` or `review`.
pub async fn run(st: &AppState, grab_id: i64) -> Result<Verdict> {
    let g = st.db.one("SELECT * FROM grabs WHERE id=?", p![grab_id])?.ok_or_else(|| err(404, "That download isn't known any more."))?;
    let wanted_id = g.i("wanted_id");
    let (_, m, pr) = wanted_parts(&st.db, wanted_id)?;
    let files: Vec<GrabFile> = serde_json::from_str(&g.s("files")).unwrap_or_default();
    let mine: Vec<&GrabFile> = files.iter().filter(|f| takes(m.format, &f.path)).collect();
    let min = st.settings.int("verify_min_confidence").clamp(0, 100) as u32;
    let checks = if mine.is_empty() {
        let what = if m.format == Format::Audiobook { "an audiobook" } else { "an ebook" };
        vec![Check::new("Files", 100, what, format!("{} files", files.len()))
            .fail(format!("None of the {} files that arrived is {what} Folio can read.", files.len()))
            .hard()]
    } else if m.format == Format::Audiobook {
        let paths: Vec<std::path::PathBuf> = mine.iter().map(|f| f.path.clone().into()).collect();
        match audio::probe(&st.settings.str("ffprobe"), &paths).await {
            Ok(facts) => audio::checks(&m, &pr, &facts),
            Err(e) => vec![Check::new("Files", 100, "a readable audiobook", "").unknown(e.detail)],
        }
    } else {
        let best = mine
            .iter()
            .min_by_key(|f| {
                let ext = std::path::Path::new(&f.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                pr.file_formats.iter().position(|x| *x == ext).unwrap_or(usize::MAX)
            })
            .map(|f| f.path.clone())
            .unwrap_or_default();
        let path = std::path::PathBuf::from(&best);
        let facts = tokio::task::spawn_blocking(move || epub::inspect(&path))
            .await
            .map_err(|e| err(500, format!("The ebook check stopped: {e}")))?;
        epub::checks(&m, &pr, &best, &facts)
    };
    let v = decide(checks, min);
    let status = if v.decision == "import" { "verified" } else { "review" };
    st.db.execute(
        "UPDATE grabs SET status=?, verdict=?, confidence=?, updated_at=? WHERE id=?",
        p![status, json!(v).to_string(), v.confidence, now(), grab_id],
    )?;
    wanted::set_status(&st.db, wanted_id, status, &v.summary)?;
    Ok(v)
}

/// Checks every grab whose files arrived and that has no verdict yet.
pub async fn pending(st: &AppState) -> Result<usize> {
    let mut n = 0;
    for r in st.db.query("SELECT id FROM grabs WHERE status='completed' ORDER BY id", p![])? {
        match run(st, r.i("id")).await {
            Ok(_) => n += 1,
            Err(e) => tracing::warn!("download {} can't be checked: {}", r.i("id"), e.detail),
        }
    }
    Ok(n)
}

/// The Review queue's routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/review", get(queue))
        .route("/api/review/{id}/accept", post(accept))
        .route("/api/review/{id}/reject", post(reject))
        .route("/api/review/{id}/research", post(research))
        .route("/api/review/{id}/verify", post(verify_again))
}

/// The grab `id` as the Review queue shows it: the grab, its wanted entry, the edition and the verdict.
fn item(db: &SqliteDb, id: i64) -> Result<Value> {
    let g = db.one("SELECT * FROM grabs WHERE id=?", p![id])?.ok_or_else(|| err(404, "That download isn't known any more."))?;
    let verdict: Value = serde_json::from_str(&g.s("verdict")).unwrap_or(Value::Null);
    let w = wanted::wanted_one(db, g.i("wanted_id")).unwrap_or(Value::Null);
    Ok(json!({ "grab": grabs::grab_row_json(&g), "wanted": w, "verdict": verdict }))
}

/// `GET /api/review`.
async fn queue(State(st): State<AppState>, _: Admin) -> Result<Json<Value>> {
    let rows = st.db.query("SELECT id FROM grabs WHERE status='review' ORDER BY updated_at DESC", p![])?;
    let items = rows.iter().map(|r| item(&st.db, r.i("id"))).collect::<Result<Vec<_>>>()?;
    Ok(Json(json!({ "items": items })))
}

/// The grab `id`, which must be waiting in the Review queue.
fn in_review(db: &SqliteDb, id: i64) -> Result<kit_server_core::Row> {
    let g = db.one("SELECT * FROM grabs WHERE id=?", p![id])?.ok_or_else(|| err(404, "That download isn't known any more."))?;
    if g.s("status") != "review" {
        return Err(err(409, "That download isn't waiting for a look any more. Reload the page to see where it is."));
    }
    Ok(g)
}

/// `POST /api/review/{id}/accept`.
async fn accept(State(st): State<AppState>, Admin(a): Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    let g = in_review(&st.db, id)?;
    st.db.execute("UPDATE grabs SET status='verified', updated_at=? WHERE id=?", p![now(), id])?;
    wanted::set_status(&st.db, g.i("wanted_id"), "verified", &format!("{} checked \"{}\" and accepted it.", a.username, g.s("title")))?;
    item(&st.db, id).map(Json)
}

/// A rejection's reason.
#[derive(Debug, Deserialize, Default)]
pub struct Reject {
    /// why, in the person's words (optional)
    #[serde(default)]
    pub reason: String,
}

/// Rejects the grab `id`: it is never grabbed again for its book, and the book is wanted again.
fn reject_one(db: &SqliteDb, user: &str, user_id: i64, id: i64, reason: &str) -> Result<i64> {
    let g = in_review(db, id)?;
    let why = if reason.trim().is_empty() { g.s("title") } else { reason.trim().to_string() };
    db.execute("UPDATE grabs SET status='rejected', updated_at=? WHERE id=?", p![now(), id])?;
    db.execute(
        "INSERT INTO rejected_releases (wanted_id, guid, title, reason, user_id, created_at) VALUES (?,?,?,?,?,?)",
        p![g.i("wanted_id"), g.s("guid"), g.s("title"), why, user_id, now()],
    )?;
    wanted::set_status(db, g.i("wanted_id"), "wanted", &format!("{user} rejected \"{}\". Folio won't grab it again.", g.s("title")))?;
    Ok(g.i("wanted_id"))
}

/// `POST /api/review/{id}/reject`.
async fn reject(State(st): State<AppState>, Admin(a): Admin, Path(id): Path<i64>, body: Option<Json<Reject>>) -> Result<Json<Value>> {
    let r = body.map(|Json(r)| r).unwrap_or_default();
    reject_one(&st.db, &a.username, a.id, id, &r.reason)?;
    item(&st.db, id).map(Json)
}

/// `POST /api/review/{id}/research`.
async fn research(State(st): State<AppState>, Admin(a): Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    let wanted_id = reject_one(&st.db, &a.username, a.id, id, "")?;
    Ok(Json(grabs::search_and_grab(&st, wanted_id).await?))
}

/// `POST /api/review/{id}/verify`.
async fn verify_again(State(st): State<AppState>, _: Admin, Path(id): Path<i64>) -> Result<Json<Value>> {
    in_review(&st.db, id)?;
    run(&st, id).await?;
    item(&st.db, id).map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_counts_unknowns_half_and_hard_failures_rule_out() {
        let ok = vec![Check::new("Runtime", 60, "", "").pass("ok"), Check::new("Narrator", 40, "", "").pass("ok")];
        let v = decide(ok, 85);
        assert_eq!((v.confidence, v.decision.as_str()), (100, "import"));
        assert!(v.summary.contains("runtime and narrator match"), "{}", v.summary);

        let half = vec![
            Check::new("Runtime", 60, "", "").pass("ok"),
            Check::new("Narrator", 40, "", "").unknown("The files don't say who reads it."),
        ];
        let v = decide(half, 85);
        assert_eq!((v.confidence, v.decision.as_str()), (80, "review"));
        assert!(v.summary.contains("80% sure") && v.summary.contains("who reads it"), "{}", v.summary);

        let wrong = vec![Check::new("Runtime", 60, "", "").fail("It is a sample.").hard(), Check::new("Narrator", 40, "", "").pass("ok")];
        let v = decide(wrong, 10);
        assert_eq!(v.decision, "review");
        assert!(v.summary.starts_with("This isn't the edition") && v.summary.contains("sample"), "{}", v.summary);
    }

    #[test]
    fn lists_read_like_sentences() {
        assert_eq!(list(&["a".into()]), "a");
        assert_eq!(list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }

    #[test]
    fn each_format_takes_its_files() {
        assert!(takes(Format::Audiobook, "/d/Book/01.MP3"));
        assert!(takes(Format::Audiobook, "/d/Book.m4b"));
        assert!(!takes(Format::Audiobook, "/d/cover.jpg"));
        assert!(takes(Format::Ebook, "/d/Book.epub"));
        assert!(!takes(Format::Ebook, "/d/Book.nfo"));
    }
}
