//! Following authors and series: new books of a followed author or series are added to the library and the
//! wanted list, in the formats followed, when the preferences take them.
//!
//! | Followed | Audiobooks from | Ebooks from |
//! | --- | --- | --- |
//! | an author | Audible's catalogue, by the author's name | Open Library, the author's works (needs the author's Open Library key) |
//! | a series | Audible's catalogue, the series' books (needs the series' ASIN) | Open Library, an edition of each book of the series already known |
//!
//! Everything offered is looked at once (`follow_seen`), so a book removed from the wanted list stays removed;
//! a source that can't be reached is asked again at the next check.

use super::store::{self, Profile};
use crate::library::{self, text, EditionId};
use crate::metadata::{audible, openlibrary, Format};
use crate::AppState;
use kit_server_core::{p, Db, Result, RowExt};
use serde::Serialize;

/// What a check did.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Report {
    /// the books added to the wanted list: `"Title (audiobook)"`
    pub added: Vec<String>,
    /// how many offers were looked at for the first time
    pub looked_at: usize,
    /// what could not be asked, in plain words (asked again next time)
    pub problems: Vec<String>,
}

/// What `follow` (`""`, `audiobook`, `ebook`, `both`) covers.
fn covers(follow: &str, f: Format) -> bool {
    follow == "both" || follow == f.as_str()
}

/// Checks every followed author and series once.
pub async fn check(st: &AppState) -> Result<Report> {
    let mut r = Report::default();
    let audio = store::default_profile(&st.db, Format::Audiobook)?;
    let ebook = store::default_profile(&st.db, Format::Ebook)?;
    let p = library::providers(st);
    for a in st.db.query("SELECT * FROM authors WHERE follow <> '' ORDER BY id", p![])? {
        let (name, follow) = (a.s("name"), a.s("follow"));
        if covers(&follow, Format::Audiobook) {
            match audible::by_author(&p, &name).await {
                Ok(products) => {
                    let key = text::person_key(&name);
                    for prod in products.into_iter().filter(|x| x.book && x.authors.iter().any(|n| text::person_key(n) == key)) {
                        offer_audiobook(st, &audio, &prod.asin, Some(&prod), &mut r).await?;
                    }
                }
                Err(e) => r.problems.push(format!("{name}'s audiobooks: {}", e.detail)),
            }
        }
        if covers(&follow, Format::Ebook) {
            let ol = a.s("ol_key");
            if ol.is_empty() {
                r.problems.push(format!(
                    "{name}'s ebooks: Open Library doesn't know this author yet. Add one of their ebooks by ISBN, and new ones follow."
                ));
            } else {
                match openlibrary::author_works(&p, &ol).await {
                    Ok(works) => {
                        for (work_key, title) in works {
                            offer_ebook_work(st, &ebook, &work_key, &title, "openlibrary-work", &mut r).await?;
                        }
                    }
                    Err(e) => r.problems.push(format!("{name}'s ebooks: {}", e.detail)),
                }
            }
        }
    }
    for s in st.db.query("SELECT * FROM series WHERE follow <> '' ORDER BY id", p![])? {
        let (name, follow) = (s.s("name"), s.s("follow"));
        if covers(&follow, Format::Audiobook) {
            let asin = s.s("asin");
            if asin.is_empty() {
                r.problems.push(format!(
                    "{name}'s audiobooks: Audible doesn't know this series yet. Add one of its audiobooks by ASIN, and the rest follow."
                ));
            } else {
                match audible::series_books(&p, &asin).await {
                    Ok(asins) => {
                        for a in asins {
                            offer_audiobook(st, &audio, &a, None, &mut r).await?;
                        }
                    }
                    Err(e) => r.problems.push(format!("{name}'s audiobooks: {}", e.detail)),
                }
            }
        }
        if covers(&follow, Format::Ebook) {
            let works = st.db.query(
                "SELECT w.id, w.title, w.ol_work FROM work_series ws JOIN works w ON w.id=ws.work_id
                 WHERE ws.series_id=? AND NOT EXISTS (SELECT 1 FROM editions e WHERE e.work_id=w.id AND e.format='ebook')",
                p![s.i("id")],
            )?;
            for w in works {
                if w.s("ol_work").is_empty() {
                    continue;
                }
                offer_ebook_work(st, &ebook, &w.s("ol_work"), &w.s("title"), "openlibrary-series", &mut r).await?;
            }
        }
    }
    Ok(r)
}

/// One audiobook a followed author or series offers: looked up on Audnexus, kept and wanted when the
/// preferences take it. `prod` is the catalogue's own facts, which can refuse it without asking Audnexus.
async fn offer_audiobook(st: &AppState, pr: &Profile, asin: &str, prod: Option<&audible::Product>, r: &mut Report) -> Result<()> {
    if store::seen(&st.db, "audible", asin)? {
        return Ok(());
    }
    if st.db.one("SELECT id FROM editions WHERE asin=?", p![asin])?.is_some() {
        return store::mark_seen(&st.db, "audible", asin, "already in the library");
    }
    r.looked_at += 1;
    if let Some(prod) = prod {
        let quick = crate::metadata::EditionMeta {
            format: Format::Audiobook,
            language: prod.language.clone(),
            abridged: prod.abridged,
            ..Default::default()
        };
        if let Some(why) = pr.refuses(&quick) {
            return store::mark_seen(&st.db, "audible", asin, &why);
        }
    }
    let m = match library::look_up(st, &EditionId { asin: asin.to_string(), ..Default::default() }).await {
        Ok(m) => m,
        Err(e) if e.status.as_u16() == 404 => return store::mark_seen(&st.db, "audible", asin, &e.detail),
        Err(e) => {
            r.problems.push(e.detail);
            return Ok(());
        }
    };
    keep_and_want(st, pr, &m, "audible", asin, r)
}

/// One work a followed author or series offers as an ebook: an edition with an ISBN in a wanted language (Open
/// Library's ebooks first), looked up, kept and wanted.
async fn offer_ebook_work(st: &AppState, pr: &Profile, work_key: &str, title: &str, source: &str, r: &mut Report) -> Result<()> {
    if store::seen(&st.db, source, work_key)? {
        return Ok(());
    }
    let have =
        st.db.one("SELECT 1 AS x FROM works w JOIN editions e ON e.work_id=w.id WHERE w.ol_work=? AND e.format='ebook'", p![work_key])?;
    if have.is_some() {
        return store::mark_seen(&st.db, source, work_key, "already in the library");
    }
    r.looked_at += 1;
    let p = library::providers(st);
    let editions = match openlibrary::work_editions(&p, work_key).await {
        Ok(e) => e,
        Err(e) => {
            r.problems.push(format!("{title}: {}", e.detail));
            return Ok(());
        }
    };
    let fits = |e: &&openlibrary::EditionRef| pr.languages.is_empty() || e.language.is_empty() || pr.languages.contains(&e.language);
    let mut pick: Vec<&openlibrary::EditionRef> = editions.iter().filter(fits).collect();
    // ebooks first, then editions that say their language
    pick.sort_by_key(|e| (!e.ebook, e.language.is_empty()));
    let Some(choice) = pick.first() else {
        return store::mark_seen(&st.db, source, work_key, "no edition with an ISBN in a wanted language");
    };
    let m = match library::look_up(st, &EditionId { isbn: choice.isbn13.clone(), ..Default::default() }).await {
        Ok(m) => m,
        Err(e) if e.status.as_u16() == 404 => return store::mark_seen(&st.db, source, work_key, &e.detail),
        Err(e) => {
            r.problems.push(format!("{title}: {}", e.detail));
            return Ok(());
        }
    };
    keep_and_want(st, pr, &m, source, work_key, r)
}

/// Keeps the edition and wants it when the preferences take it; records what came of the offer.
fn keep_and_want(st: &AppState, pr: &Profile, m: &crate::metadata::EditionMeta, source: &str, key: &str, r: &mut Report) -> Result<()> {
    if let Some(why) = pr.refuses(m) {
        return store::mark_seen(&st.db, source, key, &why);
    }
    let saved = library::store::save(&st.db, m)?;
    store::want(&st.db, saved.edition_id, Some(pr.id), 0, "follow")?;
    r.added.push(format!("{} ({})", m.title, m.format.as_str()));
    store::mark_seen(&st.db, source, key, "wanted")
}

/// Checks the followed authors and series every `follow_check_hours` hours (0: never), the first time a
/// minute after the start. For the server binary; tests call [`check`].
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        loop {
            let hours = st.settings.int("follow_check_hours");
            if hours > 0 {
                match check(&st).await {
                    Ok(r) => tracing::info!(
                        "followed authors and series: {} new wanted, {} looked at, {} problems",
                        r.added.len(),
                        r.looked_at,
                        r.problems.len()
                    ),
                    Err(e) => tracing::warn!("followed authors and series can't be checked: {}", e.detail),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(3600 * hours.clamp(1, 24 * 7) as u64)).await;
        }
    });
}
