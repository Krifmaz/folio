//! The port of `server/models/User.js`: who may do what (permissions by account type, library and tag access),
//! the root user the setup makes, finding a user to sign in, and the JSON the apps get for a user
//! ([`to_old_json_for_browser`], [`to_json_for_public`]).

use super::{find, from_row, list, MediaProgress, User};
use kit_server_core::{Db, Result};
use serde_json::{json, Value};

/// The account types besides root (`accountTypes`); there is only one root.
pub const ACCOUNT_TYPES: &[&str] = &["admin", "user", "guest"];

/// The permissions a new account of this type gets (`getDefaultPermissionsForUserType`).
pub fn default_permissions(kind: &str) -> Value {
    let admin = kind == "root" || kind == "admin";
    json!({
        "download": true,
        "update": admin,
        "delete": kind == "root",
        "upload": admin,
        "createEreader": admin,
        "accessAllLibraries": true,
        "accessAllTags": true,
        "accessExplicitContent": admin,
        "selectedTagsNotAccessible": false,
        "librariesAccessible": [],
        "itemTagsSelected": []
    })
}

/// The account type (`type`), empty when unset.
pub fn kind(u: &User) -> &str {
    u.kind.as_deref().unwrap_or_default()
}

/// Whether this is the root user (`isRoot`).
pub fn is_root(u: &User) -> bool {
    kind(u) == "root"
}

/// Whether this is an admin or the root user (`isAdminOrUp`).
pub fn is_admin_or_up(u: &User) -> bool {
    matches!(kind(u), "root" | "admin")
}

/// Whether the account may be used (`isActive`).
pub fn is_active(u: &User) -> bool {
    u.is_active == Some(true)
}

/// One permission flag, false when unset (`!!permissions[name]`).
pub fn permission(u: &User, name: &str) -> bool {
    u.permissions.as_ref().and_then(|p| p.get(name)).is_some_and(|v| crate::abs::settings::truthy(Some(v)))
}

/// Whether the account may use the permission now: set and the account active (`canDownload`, `canUpdate`, …).
pub fn can(u: &User, name: &str) -> bool {
    permission(u, name) && is_active(u)
}

/// A list in the permissions (`librariesAccessible`, `itemTagsSelected`).
fn permission_list(u: &User, name: &str) -> Vec<Value> {
    u.permissions.as_ref().and_then(|p| p.get(name)).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The OpenID subject the account is linked to (`authOpenIDSub`), if any.
pub fn openid_sub(u: &User) -> Option<&str> {
    u.extra_data.as_ref().and_then(|e| e.get("authOpenIDSub")).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Whether the user may open this library (`checkCanAccessLibrary`).
pub fn can_access_library(u: &User, library_id: &str) -> bool {
    if permission(u, "accessAllLibraries") {
        return true;
    }
    permission_list(u, "librariesAccessible").iter().any(|l| l.as_str() == Some(library_id))
}

/// Whether the user may see an item with these tags (`checkCanAccessLibraryItemWithTags`).
pub fn can_access_tags(u: &User, tags: &[String]) -> bool {
    if permission(u, "accessAllTags") {
        return true;
    }
    let selected: Vec<String> = permission_list(u, "itemTagsSelected").iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
    if permission(u, "selectedTagsNotAccessible") {
        return tags.iter().all(|t| !selected.contains(t));
    }
    !tags.is_empty() && selected.iter().any(|t| tags.contains(t))
}

/// Whether the user may see an item (`checkCanAccessLibraryItem`): its library, its explicit flag, its tags.
pub fn can_access_item(u: &User, library_id: &str, explicit: bool, tags: &[String]) -> bool {
    can_access_library(u, library_id) && (!explicit || can(u, "accessExplicitContent")) && can_access_tags(u, tags)
}

/// The first library the user may open, from ids in display order (`getDefaultLibraryId`).
pub fn default_library_id(u: &User, library_ids: &[String]) -> Option<String> {
    library_ids.iter().find(|l| can_access_library(u, l)).cloned()
}

/// Every library id in display order (`Library.getAllLibraryIds`).
pub fn all_library_ids(db: &dyn Db) -> Result<Vec<String>> {
    let rows = db.query("SELECT id FROM libraries ORDER BY displayOrder ASC", vec![])?;
    Ok(rows.iter().filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_string)).collect())
}

/// The user signing in with this name, any case (`getUserByUsername`).
pub fn by_username(db: &dyn Db, username: &str) -> Result<Option<User>> {
    if username.is_empty() {
        return Ok(None);
    }
    let row = db.one("SELECT * FROM users WHERE lower(username) = ? LIMIT 1", vec![json!(username.to_lowercase())])?;
    row.map(from_row).transpose()
}

/// The user with this id (`getUserById`).
pub fn by_id(db: &dyn Db, id: &str) -> Result<Option<User>> {
    if id.is_empty() {
        return Ok(None);
    }
    find(db, id)
}

/// The user with this id or, for tokens made before 2.3.0, this old id (`getUserByIdOrOldId`).
pub fn by_id_or_old_id(db: &dyn Db, id: &str) -> Result<Option<User>> {
    if id.is_empty() {
        return Ok(None);
    }
    let row =
        db.one("SELECT * FROM users WHERE id = ? OR json_extract(extraData, '$.oldUserId') = ? LIMIT 1", vec![json!(id), json!(id)])?;
    row.map(from_row).transpose()
}

/// Whether the root user exists, so the server is set up (`getHasRootUser`).
pub fn has_root(db: &dyn Db) -> Result<bool> {
    Ok(db.one("SELECT id FROM users WHERE type = 'root' LIMIT 1", vec![])?.is_some())
}

/// Makes the root user (`createRootUser`) with its old, never-expiring token. Answers `None` when a root user
/// exists already, like the original's check in a transaction.
pub fn create_root(db: &dyn Db, username: &str, pash: &str, legacy_token: impl FnOnce(&str) -> String) -> Result<Option<User>> {
    if has_root(db)? {
        return Ok(None);
    }
    let id = super::new_id();
    let now = super::date_now();
    let user = User {
        token: Some(legacy_token(&id)),
        id: Some(id),
        kind: Some("root".into()),
        username: Some(username.to_string()),
        pash: Some(pash.to_string()),
        is_active: Some(true),
        is_locked: Some(false),
        permissions: Some(default_permissions("root")),
        bookmarks: Some(json!([])),
        extra_data: Some(json!({ "seriesHideFromContinueListening": [] })),
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..User::default()
    };
    super::insert(db, &user)?;
    Ok(Some(user))
}

/// The user's listening and reading progress (`mediaProgresses`, loaded with the user in the original).
pub fn media_progresses(db: &dyn Db, user_id: &str) -> Result<Vec<MediaProgress>> {
    list(db, "`userId` = ?", "", vec![json!(user_id)])
}

/// A stored time as the API shows it: milliseconds, or null.
fn time_ms(t: &Option<String>) -> Value {
    t.as_deref().and_then(super::date_ms).map(Value::from).unwrap_or(Value::Null)
}

/// The user as the web and mobile apps know it (`toOldJSONForBrowser`). `is_old_token` is the flag the sign-in
/// check sets for tokens without an expiry (left out when `None`); `progress` is the user's media progress
/// (left out with `minimal`).
pub fn to_old_json_for_browser(
    u: &User,
    progress: &[MediaProgress],
    is_old_token: Option<bool>,
    hide_root_token: bool,
    minimal: bool,
) -> Value {
    let mut permissions = u.permissions.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    if let Some(p) = permissions.as_object_mut() {
        p.remove("librariesAccessible");
        p.remove("itemTagsSelected");
    }
    let hide_series = u
        .extra_data
        .as_ref()
        .and_then(|e| e.get("seriesHideFromContinueListening"))
        .filter(|v| v.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    let token = if is_root(u) && hide_root_token { json!("") } else { json!(u.token) };
    let mut j = json!({
        "id": u.id,
        "username": u.username,
        "email": u.email,
        "type": u.kind,
        "token": token,
        "mediaProgress": progress.iter().map(super::media_progress::old_json).collect::<Vec<_>>(),
        "seriesHideFromContinueListening": hide_series,
        "bookmarks": u.bookmarks.clone().filter(Value::is_array).unwrap_or_else(|| json!([])),
        "isActive": u.is_active.unwrap_or(false),
        "isLocked": u.is_locked.unwrap_or(false),
        "lastSeen": time_ms(&u.last_seen),
        "createdAt": time_ms(&u.created_at),
        "permissions": permissions,
        "librariesAccessible": permission_list(u, "librariesAccessible"),
        "itemTagsSelected": permission_list(u, "itemTagsSelected"),
        "hasOpenIDLink": openid_sub(u).is_some()
    });
    if let Some(old) = is_old_token {
        j["isOldToken"] = json!(old);
    }
    if minimal {
        if let Some(m) = j.as_object_mut() {
            m.remove("mediaProgress");
            m.remove("bookmarks");
        }
    }
    j
}

/// The user as other users see them in "who's online" and socket events (`toJSONForPublic`); `session` is
/// their open playback session as the client sees it, if any.
pub fn to_json_for_public(u: &User, session: Option<Value>) -> Value {
    json!({
        "id": u.id,
        "username": u.username,
        "type": u.kind,
        "session": session.unwrap_or(Value::Null),
        "lastSeen": time_ms(&u.last_seen),
        "createdAt": time_ms(&u.created_at)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(perms: Value) -> User {
        User { kind: Some("user".into()), is_active: Some(true), permissions: Some(perms), ..User::default() }
    }

    #[test]
    fn permissions_by_type_are_the_originals() {
        let root = default_permissions("root");
        assert_eq!(root["delete"], true);
        let admin = default_permissions("admin");
        assert_eq!((admin["delete"].clone(), admin["update"].clone()), (json!(false), json!(true)));
        let guest = default_permissions("guest");
        assert_eq!(
            (guest["download"].clone(), guest["upload"].clone(), guest["accessExplicitContent"].clone()),
            (json!(true), json!(false), json!(false))
        );
    }

    #[test]
    fn library_and_tag_access_follow_the_original() {
        let all = user(default_permissions("user"));
        assert!(can_access_library(&all, "x"));
        let some =
            user(json!({"accessAllLibraries": false, "librariesAccessible": ["a"], "accessAllTags": false, "itemTagsSelected": ["kids"]}));
        assert!(can_access_library(&some, "a") && !can_access_library(&some, "b"));
        assert!(can_access_tags(&some, &["kids".into()]));
        assert!(!can_access_tags(&some, &[]), "with an allow list, untagged items are hidden");
        let deny = user(json!({"accessAllTags": false, "selectedTagsNotAccessible": true, "itemTagsSelected": ["adult"]}));
        assert!(can_access_tags(&deny, &[]) && !can_access_tags(&deny, &["adult".into(), "x".into()]));
        assert_eq!(default_library_id(&some, &["b".into(), "a".into()]), Some("a".into()));
        assert!(!can_access_item(&all, "x", true, &[]), "explicit items need the permission");
    }

    #[test]
    fn root_is_made_once_and_found_by_any_case() {
        let db = kit_server_core::SqliteDb::memory();
        crate::abs::create(&db).unwrap();
        let root = create_root(&db, "Root", "", |id| format!("token-{id}")).unwrap().unwrap();
        assert!(create_root(&db, "other", "", |_| String::new()).unwrap().is_none());
        assert!(has_root(&db).unwrap());
        let found = by_username(&db, "ROOT").unwrap().unwrap();
        assert_eq!(found.id, root.id);
        assert_eq!(found.token, Some(format!("token-{}", root.id.clone().unwrap())));
        let j = to_old_json_for_browser(&found, &[], None, false, false);
        let want = crate::abs::tests_support::fixture("me");
        let mut m = crate::abs::tests_support::matcher();
        let diffs = m.diff(&j, &want);
        assert!(diffs.iter().all(|d| d.starts_with("$.username")), "{diffs:?}");
    }
}
