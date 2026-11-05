//! Readarr's request checks (FluentValidation in `Readarr.Http` and `NzbDrone.Core.Validation`): each failed rule
//! becomes one entry of the 400 answer, in the order the rules are declared, with FluentValidation's fields.
//! Also the path rules Readarr's checks rely on (`PathExtensions`): equal paths and parent paths.

use super::Fail;
use kit_server_core::{Db, RowExt, SqliteDb};
use serde_json::{json, Map, Value};

/// The folders Readarr refuses as an author's folder on Linux (`SystemFolders`).
const SYSTEM_FOLDERS: &[&str] = &["/bin", "/boot", "/lib", "/sbin", "/proc", "/usr/bin"];

/// A property's name as FluentValidation shows it: `QualityProfileId` → `Quality Profile Id`.
pub fn display_name(property: &str) -> String {
    let mut out = String::new();
    for (i, c) in property.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// The failed rules of one request.
#[derive(Default)]
pub struct Failures(pub Vec<Value>);

impl Failures {
    /// Records a failed rule: the property, its value, the validator (`errorCode`), the message and the
    /// message's extra arguments (besides the property's name and value, which every failure has).
    pub fn add(&mut self, property: &str, value: &Value, code: &str, message: String, args: &[(&str, Value)]) {
        let mut placeholders = Map::new();
        for (k, v) in args {
            placeholders.insert((*k).to_string(), v.clone());
        }
        placeholders.insert("propertyName".into(), json!(display_name(property)));
        placeholders.insert("propertyValue".into(), value.clone());
        self.0.push(json!({
            "propertyName": property,
            "errorMessage": message,
            "attemptedValue": value,
            "severity": "error",
            "errorCode": code,
            "formattedMessageArguments": [],
            "formattedMessagePlaceholderValues": placeholders,
        }));
    }

    /// `ValidId`: the number must be greater than 0.
    pub fn valid_id(&mut self, property: &str, value: i64) {
        if value <= 0 {
            let name = display_name(property);
            self.add(
                property,
                &json!(value),
                "GreaterThanValidator",
                format!("'{name}' must be greater than '0'."),
                &[("comparisonValue", json!(0)), ("comparisonProperty", json!(""))],
            );
        }
    }

    /// The answer: nothing when every rule held, else Readarr's 400 with the failures.
    pub fn check(self) -> Result<(), Fail> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Fail { status: axum::http::StatusCode::BAD_REQUEST, body: Some(Value::Array(self.0)) })
        }
    }
}

/// A path as Readarr compares it: without trailing slashes (`/` stays `/`).
pub fn clean(path: &str) -> String {
    let p = path.trim();
    let t = p.trim_end_matches('/');
    if t.is_empty() && p.starts_with('/') {
        "/".to_string()
    } else {
        t.to_string()
    }
}

/// Whether two paths are the same folder (`PathEquals`; Linux paths are case-sensitive).
pub fn path_equals(a: &str, b: &str) -> bool {
    a == b || clean(a) == clean(b)
}

/// Whether `parent` is a folder above `child` (`IsParentPath`).
pub fn is_parent_path(parent: &str, child: &str) -> bool {
    let parent = clean(parent);
    let mut child = clean(child);
    while let Some(i) = child.rfind('/') {
        child = if i == 0 { "/".to_string() } else { child[..i].to_string() };
        if child == parent {
            return true;
        }
        if child == "/" {
            break;
        }
    }
    false
}

/// Whether a path is valid on Linux (`IsPathValid`): not blank, absolute, no NUL.
pub fn path_valid(path: &str) -> bool {
    !path.trim().is_empty() && path.starts_with('/') && !path.contains('\0')
}

/// `IsValidPath`: fails with `Invalid Path: '<path>'`.
pub fn valid_path(f: &mut Failures, property: &str, path: Option<&str>) -> bool {
    match path {
        Some(p) if path_valid(p) => true,
        Some(p) => {
            f.add(property, &json!(p), "PathValidator", format!("Invalid Path: '{p}'"), &[("path", json!(p))]);
            false
        }
        None => {
            f.add(property, &Value::Null, "PathValidator", "Invalid Path: '{path}'".into(), &[]);
            false
        }
    }
}

/// The folder rules of an author's path, stopping at the first that fails (`AuthorController`'s
/// `SharedValidator` on `Path`): valid, not a root folder, not another author's, not above an author, not the
/// recycle bin, not a system folder.
pub fn author_path(db: &SqliteDb, f: &mut Failures, author_id: i64, path: &str) -> kit_server_core::Result<()> {
    if !valid_path(f, "Path", Some(path)) {
        return Ok(());
    }
    let v = json!(path);
    let arg = [("path", v.clone())];
    let roots = db.query("SELECT Path FROM RootFolders", vec![])?;
    if roots.iter().any(|r| path_equals(&r.s("Path"), path)) {
        f.add("Path", &v, "RootFolderValidator", format!("Path '{path}' is already configured as a root folder"), &arg);
        return Ok(());
    }
    let authors = db.query("SELECT Id, Path FROM Authors", vec![])?;
    if authors.iter().any(|a| path_equals(&a.s("Path"), path) && a.i("Id") != author_id) {
        f.add("Path", &v, "AuthorPathValidator", format!("Path '{path}' is already configured for another author"), &arg);
        return Ok(());
    }
    if authors.iter().any(|a| is_parent_path(path, &a.s("Path"))) {
        f.add("Path", &v, "AuthorAncestorValidator", format!("Path '{path}' is an ancestor of an existing author"), &arg);
        return Ok(());
    }
    let bin = db.one("SELECT Value FROM Config WHERE Key = 'recyclebin'", vec![])?.map(|r| r.s("Value")).unwrap_or_default();
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
            return Ok(());
        }
    }
    for sys in SYSTEM_FOLDERS {
        let rel = if path_equals(sys, path) {
            Some("set to")
        } else if is_parent_path(sys, path) {
            Some("child of")
        } else {
            None
        };
        if let Some(rel) = rel {
            let args = [("path", v.clone()), ("systemFolder", json!(sys)), ("relationship", json!(rel))];
            f.add("Path", &v, "SystemFolderValidator", format!("Path '{path}' is {rel} system folder {sys}"), &args);
            return Ok(());
        }
    }
    Ok(())
}

/// `QualityProfileExistsValidator` and `MetadataProfileExistsValidator`: 0 passes, any other id must exist.
pub fn profile_exists(db: &SqliteDb, f: &mut Failures, property: &str, id: i64) -> kit_server_core::Result<()> {
    if id == 0 {
        return Ok(());
    }
    let (table, code, message) = if property == "QualityProfileId" {
        ("QualityProfiles", "QualityProfileExistsValidator", "Quality Profile does not exist")
    } else {
        ("MetadataProfiles", "MetadataProfileExistsValidator", "Metadata profile does not exist")
    };
    if db.one(&format!("SELECT Id FROM {table} WHERE Id = ?"), vec![json!(id)])?.is_none() {
        f.add(property, &json!(id), code, message.into(), &[]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_compare_like_readarr() {
        assert!(path_equals("/books/", "/books"));
        assert!(is_parent_path("/books", "/books/A/B"));
        assert!(is_parent_path("/", "/books"));
        assert!(!is_parent_path("/books", "/books"));
        assert!(!is_parent_path("/book", "/books/A"));
        assert_eq!(display_name("MetadataProfileId"), "Metadata Profile Id");
    }
}
