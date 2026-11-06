//! Adding an author (`AuthorController.AddAuthor` and `AddAuthorService`): the request's checks, the author's
//! metadata from the metadata server, its folder (the root folder and the naming settings' author folder, made
//! unique), then the author and its metadata stored. Readarr answers 201 with the author right away and fills in
//! its books afterwards ([`super::refresh`]).
//!
//! ```text
//! POST /api/v1/author   add an author from a lookup (201 with the author)
//! ```

use super::bookinfo::Failure;
use super::naming;
use super::refresh;
use super::text::clean_author_name;
use super::validation::{display_name, is_parent_path, path_equals, profile_exists, valid_path, Failures};
use super::{authors, Answer, Fail, Key};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use serde_json::{json, Value};

/// The names of `MonitorTypes`, by number.
const MONITOR_TYPES: [&str; 8] = ["all", "future", "missing", "existing", "latest", "first", "none", "unknown"];

/// A text field of the request, or nothing.
fn text<'a>(r: &'a Value, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
}

/// Whether a text is missing or blank (`IsNullOrWhiteSpace`).
fn blank(v: Option<&str>) -> bool {
    v.is_none_or(|s| s.trim().is_empty())
}

/// A number from the request (0 when it is missing or not one, as .NET binds it).
fn int(r: &Value, k: &str) -> i64 {
    r.get(k).and_then(Value::as_i64).unwrap_or(0)
}

/// The number of single-character edits between two texts (`LevenshteinDistance`).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The last folder of a path, as .NET's `DirectoryInfo.Name` gives it.
fn folder_name(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return path.to_string();
    }
    trimmed.rsplit('/').next().unwrap_or_default().to_string()
}

/// The author a request describes, in the columns [`naming::author_folder`] reads (`AuthorResource.ToModel`).
fn requested_author(r: &Value) -> Row {
    let mut a = Row::new();
    a.insert("Name".into(), r.get("authorName").cloned().unwrap_or(Value::Null));
    a.insert("NameLastFirst".into(), r.get("authorNameLastFirst").cloned().unwrap_or(Value::Null));
    a
}

/// `AuthorFolderAsRootFolderValidator`: the root folder must not look like the author's own folder.
fn folder_as_root(db: &SqliteDb, f: &mut Failures, r: &Value) -> Answer<()> {
    let Some(root) = text(r, "rootFolderPath").filter(|p| !p.trim().is_empty()) else { return Ok(()) };
    if r.get("authorName").and_then(Value::as_str).is_none() {
        return Err(Fail::exception(500, "System.NullReferenceException", "Object reference not set to an instance of an object."));
    }
    let root_name = folder_name(root);
    let author_folder = naming::author_folder(&requested_author(r), &naming::config(db)?);
    let distance = levenshtein(&author_folder, &root_name);
    let threshold = (author_folder.chars().count() as f64 * 0.2).max(1.0);
    if author_folder == root_name || (distance as f64) < threshold {
        let v = json!(root);
        f.add(
            "RootFolderPath",
            &v,
            "AuthorFolderAsRootFolderValidator",
            format!("Root folder path '{root}' contains author folder '{author_folder}'"),
            &[("rootFolderPath", v.clone()), ("authorFolder", json!(author_folder))],
        );
    }
    Ok(())
}

/// `NotEmpty`: the text must not be missing or blank.
fn not_empty(f: &mut Failures, property: &str, value: &Value) {
    if blank(value.as_str()) {
        let name = display_name(property);
        f.add(property, value, "NotEmptyValidator", format!("'{name}' must not be empty."), &[]);
    }
}

/// The checks of `POST /api/v1/author`, in Readarr's order: the shared ones (profiles, folder), then the add's
/// own (folder or root folder, name, id at the metadata server, not added yet).
fn check(db: &SqliteDb, r: &Value) -> Answer<()> {
    let mut f = Failures::default();
    let path = text(r, "path");
    let root = text(r, "rootFolderPath");
    f.valid_id("QualityProfileId", int(r, "qualityProfileId"));
    f.valid_id("MetadataProfileId", int(r, "metadataProfileId"));
    if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        super::validation::author_path(db, &mut f, 0, p)?;
    }
    profile_exists(db, &mut f, "QualityProfileId", int(r, "qualityProfileId"))?;
    profile_exists(db, &mut f, "MetadataProfileId", int(r, "metadataProfileId"))?;
    if blank(root) {
        valid_path(&mut f, "Path", path);
    }
    if blank(path) {
        valid_path(&mut f, "RootFolderPath", root);
        folder_as_root(db, &mut f, r)?;
    }
    let name = r.get("authorName").cloned().unwrap_or(Value::Null);
    not_empty(&mut f, "AuthorName", &name);
    let foreign = r.get("foreignAuthorId").cloned().unwrap_or(Value::Null);
    not_empty(&mut f, "ForeignAuthorId", &foreign);
    if let Some(id) = foreign.as_str() {
        if !authors::rows(db, "m.ForeignAuthorId = ?", vec![json!(id)])?.is_empty() {
            f.add("ForeignAuthorId", &foreign, "AuthorExistsValidator", "This author has already been added".into(), &[]);
        }
    }
    f.check()
}

/// The add options of a request as Readarr stores them (`AddAuthorOptions`), or nothing.
fn add_options(r: &Value) -> Value {
    let Some(o) = r.get("addOptions").filter(|o| o.is_object()) else { return Value::Null };
    let monitor = match &o["monitor"] {
        Value::Number(n) => n.as_u64().and_then(|n| MONITOR_TYPES.get(n as usize)).copied().unwrap_or("all"),
        Value::String(s) => MONITOR_TYPES.iter().find(|m| m.eq_ignore_ascii_case(s)).copied().unwrap_or("all"),
        _ => "all",
    };
    json!({
        "searchForMissingBooks": o["searchForMissingBooks"].as_bool().unwrap_or(false),
        "monitor": monitor,
        "booksToMonitor": o["booksToMonitor"].as_array().cloned().unwrap_or_default(),
        "monitored": o["monitored"].as_bool().unwrap_or(false),
    })
}

/// The checks of the author to add (`AddAuthorValidator`): its folder (valid, not a root folder, not the recycle
/// bin, not another author's, not above one) and its profiles.
fn check_new_author(db: &SqliteDb, path: &str, quality: i64, metadata: i64) -> Answer<()> {
    let mut f = Failures::default();
    let v = json!(path);
    let arg = [("path", v.clone())];
    'path: {
        if !valid_path(&mut f, "Path", Some(path)) {
            break 'path;
        }
        if db.query("SELECT Path FROM RootFolders", vec![])?.iter().any(|r| path_equals(&r.s("Path"), path)) {
            f.add("Path", &v, "RootFolderValidator", format!("Path '{path}' is already configured as a root folder"), &arg);
            break 'path;
        }
        let bin = super::disk::config(db, "recyclebin")?.unwrap_or_default();
        if !bin.trim().is_empty() {
            let rel = if path_equals(&bin, path) {
                Some("set to")
            } else if is_parent_path(&bin, path) {
                Some("child of")
            } else {
                None
            };
            if let Some(rel) = rel {
                let args = [("path", v.clone()), ("relationship", json!(rel))];
                f.add("Path", &v, "RecycleBinValidator", format!("Path '{path}' is {rel} configured recycle bin folder"), &args);
                break 'path;
            }
        }
        let paths = db.query("SELECT Path FROM Authors", vec![])?;
        if paths.iter().any(|a| path_equals(&a.s("Path"), path)) {
            f.add("Path", &v, "AuthorPathValidator", format!("Path '{path}' is already configured for another author"), &arg);
            break 'path;
        }
        if paths.iter().any(|a| is_parent_path(path, &a.s("Path"))) {
            f.add("Path", &v, "AuthorAncestorValidator", format!("Path '{path}' is an ancestor of an existing author"), &arg);
        }
    }
    profile_exists(db, &mut f, "QualityProfileId", quality)?;
    profile_exists(db, &mut f, "MetadataProfileId", metadata)?;
    f.check()
}

/// Whether an author has this exact folder (`AuthorPathExists`).
fn path_taken(db: &SqliteDb, path: &str) -> kit_server_core::Result<bool> {
    Ok(db.one("SELECT Id FROM Authors WHERE Path = ?", vec![json!(path)])?.is_some())
}

/// `POST /api/v1/author`: 201 with the author as `GET` sends it; its books follow in the background.
pub async fn add_author(State(st): State<AppState>, _: Key, bytes: Bytes) -> Answer<(StatusCode, Json<Value>)> {
    let db = &st.readarr;
    let r = super::body(&bytes)?;
    check(db, &r)?;
    let foreign = text(&r, "foreignAuthorId").unwrap_or_default().to_string();
    let info = match st.readarr_lookup.get_author_info(db, &foreign, false).await {
        Ok(i) => i,
        Err(Failure::AuthorNotFound) => {
            tracing::error!("Readarr: author {foreign} was not found, it may have been removed from Goodreads");
            return Err(Fail {
                status: StatusCode::BAD_REQUEST,
                body: Some(json!([{
                    "propertyName": "ForeignAuthorId",
                    "errorMessage": "An author with this ID was not found",
                    "attemptedValue": foreign,
                    "severity": "error",
                }])),
            });
        }
        Err(e) => return Err(e.answer()),
    };
    let meta = &info.meta;
    let mut path = text(&r, "path").unwrap_or_default().to_string();
    if path.trim().is_empty() {
        let folder = naming::author_folder(meta, &naming::config(db)?);
        path = super::disk::combine(text(&r, "rootFolderPath").unwrap_or_default(), &folder);
    }
    if path_taken(db, &path)? {
        if let Some(d) = meta.get("Disambiguation").and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
            path = format!("{path} ({d})");
        }
        if path_taken(db, &path)? {
            let base = path.clone();
            let mut i = 0;
            loop {
                i += 1;
                path = format!("{base} ({i})");
                if !path_taken(db, &path)? {
                    break;
                }
            }
        }
    }
    let options = add_options(&r);
    let mut monitored = r.get("monitored").and_then(Value::as_bool).unwrap_or(false);
    if options["monitor"] == json!("none") {
        monitored = false;
    }
    let quality = int(&r, "qualityProfileId");
    let metadata = int(&r, "metadataProfileId");
    check_new_author(db, &path, quality, metadata)?;
    let (meta_id, _) = refresh::upsert_metadata(db, meta)?;
    let tags = match r.get("tags") {
        Some(Value::Array(a)) => {
            let mut seen: Vec<i64> = Vec::new();
            for t in a.iter().filter_map(Value::as_i64) {
                if !seen.contains(&t) {
                    seen.push(t);
                }
            }
            json!(json!(seen).to_string())
        }
        _ => Value::Null,
    };
    let id = db.execute(
        "INSERT INTO Authors (CleanName, Path, Monitored, LastInfoSync, QualityProfileId, Tags, Added, AddOptions, MetadataProfileId,
         AuthorMetadataId, MonitorNewItems) VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?)",
        vec![
            json!(clean_author_name(&meta.s("Name"))),
            json!(path),
            json!(monitored),
            json!(quality),
            tags,
            json!(super::json::now_param()),
            super::json::stored_doc(Some(&options)),
            json!(metadata),
            json!(meta_id),
            json!(r.get("monitorNewItems").and_then(authors::parse_monitor_new_items).unwrap_or(0)),
        ],
    )?;
    tracing::info!("Readarr: adding author {} at {path}", meta.s("Name"));
    let author = authors::get_or_404(db, id)?;
    let mut answer = authors::full(db, &[author], true)?;
    st.readarr_jobs.spawn(refresh::new_author(db.clone(), st.readarr_disk.clone(), st.readarr_lookup.clone(), id));
    Ok((StatusCode::CREATED, Json(answer.remove(0))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_compare_like_readarr() {
        assert_eq!(levenshtein("Ann Leckie", "Ann Lecki"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(folder_name("/books/"), "books");
        assert_eq!(folder_name("/media/Ann Lecki"), "Ann Lecki");
    }
}
