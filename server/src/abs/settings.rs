//! The port of `server/objects/settings/ServerSettings.js` and of the `settings` table rows the original keeps
//! (`models/Setting.js`): the server's settings, read the way the original reads them (defaults, old names,
//! values of the wrong kind), stored as the same JSON under the key `server-settings`, and shown to the apps
//! with the same keys ([`ServerSettings::to_json_for_browser`]).
//!
//! The settings are kept as the JSON object the original keeps, so a value the web app sends is stored as it was
//! sent, like the original does.

use crate::abs::models::{self, Setting};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// The keys `PATCH /api/settings` may change (`PATCHABLE_SETTINGS_KEYS`).
pub const PATCHABLE: &[&str] = &[
    "scannerParseSubtitle",
    "scannerFindCovers",
    "scannerCoverProvider",
    "scannerPreferMatchedMetadata",
    "scannerDisableWatcher",
    "storeCoverWithItem",
    "storeMetadataWithItem",
    "allowIframe",
    "allowedOrigins",
    "backupSchedule",
    "backupsToKeep",
    "maxBackupSize",
    "logLevel",
    "homeBookshelfView",
    "bookshelfView",
    "dateFormat",
    "timeFormat",
    "language",
    "chromecastEnabled",
    "sortingIgnorePrefix",
];

/// The keys kept from the apps (`toJSONForBrowser` deletes them).
const PRIVATE: &[&str] = &[
    "tokenSecret",
    "authOpenIDClientID",
    "authOpenIDClientSecret",
    "authOpenIDMobileRedirectURIs",
    "authOpenIDGroupClaim",
    "authOpenIDAdvancedPermsClaim",
];

/// The sign-in settings, in the original's order (`authenticationSettings` without the sample).
const AUTH_KEYS: &[&str] = &[
    "authLoginCustomMessage",
    "authActiveAuthMethods",
    "authOpenIDIssuerURL",
    "authOpenIDAuthorizationURL",
    "authOpenIDTokenURL",
    "authOpenIDUserInfoURL",
    "authOpenIDJwksURL",
    "authOpenIDLogoutURL",
    "authOpenIDClientID",
    "authOpenIDClientSecret",
    "authOpenIDTokenSigningAlgorithm",
    "authOpenIDButtonText",
    "authOpenIDAutoLaunch",
    "authOpenIDAutoRegister",
    "authOpenIDMatchExistingBy",
    "authOpenIDMobileRedirectURIs",
    "authOpenIDGroupClaim",
    "authOpenIDAdvancedPermsClaim",
    "authOpenIDSubfolderForRedirectURLs",
];

/// The sign-in methods the server knows (`supportedAuthMethods`).
pub const SUPPORTED_AUTH_METHODS: &[&str] = &["local", "openid"];

/// The example of the OpenID permissions claim the settings page shows (`User.getSampleAbsPermissions`).
pub const SAMPLE_PERMISSIONS: &str = "{\n  \"canDownload\": false,\n  \"canUpload\": false,\n  \"canDelete\": false,\n  \"canUpdate\": false,\n  \"canAccessExplicitContent\": false,\n  \"canAccessAllLibraries\": false,\n  \"canAccessAllTags\": false,\n  \"canCreateEReader\": false,\n  \"tagsAreDenylist\": false,\n  \"allowedLibraries\": [\n    \"5406ba8a-16e1-451d-96d7-4931b0a0d966\",\n    \"918fd848-7c1d-4a02-818a-847435a879ca\"\n  ],\n  \"allowedTags\": [\n    \"ExampleTag\",\n    \"AnotherTag\",\n    \"ThirdTag\"\n  ]\n}";

/// Whether an app address may receive the OpenID sign-in (`isValidRedirectURI`: `scheme://host/path`, letters,
/// digits, `_`, `.`, `-` and `/` only).
pub fn is_redirect_uri(uri: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let Some((scheme, rest)) = uri.split_once("://") else { return false };
    if scheme.is_empty() || !scheme.chars().all(word) {
        return false;
    }
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, Some(p))).unwrap_or((rest, None));
    !host.is_empty()
        && host.chars().all(|c| word(c) || c == '.' || c == '-')
        && path.is_none_or(|p| p.chars().all(|c| word(c) || c == '.' || c == '/' || c == '-'))
}

/// The original's default log level (`LogLevel.INFO`).
pub const LOG_LEVEL_INFO: i64 = 2;

/// The build number of the pinned original (`package.json`'s `buildNumber`).
pub const BUILD_NUMBER: i64 = 1;

/// The server's settings, as the original's `ServerSettings` object holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerSettings(pub Map<String, Value>);

/// Whether a JSON value counts as true in JavaScript (`!!value`).
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// JavaScript's `Number(value)`, `None` for NaN (and for a missing value, which is `undefined`).
pub fn js_number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Null => Some(0.0),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::Number(n) => n.as_f64(),
        Value::String(s) if s.trim().is_empty() => Some(0.0),
        Value::String(s) => s.trim().parse().ok(),
        Value::Array(a) if a.is_empty() => Some(0.0),
        Value::Array(a) if a.len() == 1 => js_number(a.first()),
        Value::Object(_) | Value::Array(_) => None,
    }
}

/// A number as JSON: whole numbers as integers, like JavaScript prints them.
fn number(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9e15 {
        Value::from(f as i64)
    } else {
        Value::from(f)
    }
}

/// The host's time zone (`getHostTimeZone`): `TZ` when set, else UTC.
pub fn host_time_zone() -> String {
    std::env::var("TZ").ok().filter(|t| !t.trim().is_empty() && !t.starts_with(':')).unwrap_or_else(|| "UTC".into())
}

impl ServerSettings {
    /// The settings of a new server (the constructor without stored settings). `metadata_path` is the folder of
    /// covers, caches and backups.
    pub fn defaults(metadata_path: &str) -> ServerSettings {
        let text = r#"{
            "id": "server-settings",
            "tokenSecret": null,
            "scannerFindCovers": false,
            "scannerCoverProvider": "google",
            "scannerParseSubtitle": false,
            "scannerPreferMatchedMetadata": false,
            "scannerDisableWatcher": false,
            "storeCoverWithItem": false,
            "storeMetadataWithItem": false,
            "metadataFileFormat": "json",
            "rateLimitLoginRequests": 10,
            "rateLimitLoginWindow": 600000,
            "allowIframe": false,
                        "backupSchedule": false,
            "backupsToKeep": 2,
            "maxBackupSize": 1,
            "loggerDailyLogsToKeep": 7,
            "loggerScannerLogsToKeep": 2,
            "homeBookshelfView": 1,
            "bookshelfView": 1,
            "podcastEpisodeSchedule": "0 * * * *",
            "sortingIgnorePrefix": false,
            "sortingPrefixes": ["the", "a"],
            "chromecastEnabled": false,
            "dateFormat": "MM/dd/yyyy",
            "timeFormat": "HH:mm",
            "language": "en-us",
            "allowedOrigins": [],
            "logLevel": 2,
            "version": "",
            "buildNumber": 1,
            "authLoginCustomMessage": null,
            "authActiveAuthMethods": ["local"],
            "authOpenIDIssuerURL": null,
            "authOpenIDAuthorizationURL": null,
            "authOpenIDTokenURL": null,
            "authOpenIDUserInfoURL": null,
            "authOpenIDJwksURL": null,
            "authOpenIDLogoutURL": null,
            "authOpenIDClientID": null,
            "authOpenIDClientSecret": null,
            "authOpenIDTokenSigningAlgorithm": "RS256",
            "authOpenIDButtonText": "Login with OpenId",
            "authOpenIDAutoLaunch": false,
            "authOpenIDAutoRegister": false,
            "authOpenIDMatchExistingBy": null,
            "authOpenIDMobileRedirectURIs": ["audiobookshelf://oauth"],
            "authOpenIDGroupClaim": "",
            "authOpenIDAdvancedPermsClaim": ""
        }"#;
        let mut map: Map<String, Value> = serde_json::from_str(text).unwrap_or_default();
        map.insert("backupPath".into(), json!(format!("{}/backups", metadata_path.trim_end_matches('/'))));
        map.insert("logLevel".into(), json!(LOG_LEVEL_INFO));
        map.insert("version".into(), json!(crate::abs::ABS_VERSION));
        map.insert("buildNumber".into(), json!(BUILD_NUMBER));
        ServerSettings(map)
    }

    /// The settings read from stored JSON (`construct`): every value checked and defaulted like the original,
    /// old names moved to new ones, and the `BACKUP_PATH` and `ALLOW_IFRAME` environment applied.
    pub fn construct(s: &Map<String, Value>, metadata_path: &str) -> ServerSettings {
        let get = |k: &str| s.get(k);
        let or = |k: &str, d: Value| if truthy(get(k)) { get(k).cloned().unwrap_or(d) } else { d };
        let flag = |k: &str| Value::Bool(truthy(get(k)));
        let or_null = |k: &str| or(k, Value::Null);
        let mut m = ServerSettings::defaults(metadata_path).0;
        let mut set = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        set("tokenSecret", get("tokenSecret").cloned().unwrap_or(Value::Null));
        set("scannerFindCovers", flag("scannerFindCovers"));
        set("scannerCoverProvider", or("scannerCoverProvider", json!("google")));
        set("scannerParseSubtitle", get("scannerParseSubtitle").cloned().unwrap_or(Value::Null));
        set("scannerPreferMatchedMetadata", flag("scannerPreferMatchedMetadata"));
        set("scannerDisableWatcher", flag("scannerDisableWatcher"));
        set("storeCoverWithItem", flag("storeCoverWithItem"));
        set("storeMetadataWithItem", flag("storeMetadataWithItem"));
        set("rateLimitLoginRequests", js_number(get("rateLimitLoginRequests")).map(number).unwrap_or(json!(10)));
        set("rateLimitLoginWindow", js_number(get("rateLimitLoginWindow")).map(number).unwrap_or(json!(600_000)));
        set("allowIframe", flag("allowIframe"));
        set("backupPath", or("backupPath", json!(format!("{}/backups", metadata_path.trim_end_matches('/')))));
        set("backupSchedule", or("backupSchedule", json!(false)));
        set("backupsToKeep", or("backupsToKeep", json!(2)));
        let max = get("maxBackupSize");
        set("maxBackupSize", if max.and_then(Value::as_f64) == Some(0.0) { json!(0) } else { or("maxBackupSize", json!(1)) });
        set("loggerDailyLogsToKeep", or("loggerDailyLogsToKeep", json!(7)));
        set("loggerScannerLogsToKeep", or("loggerScannerLogsToKeep", json!(2)));
        set("homeBookshelfView", or("homeBookshelfView", json!(0)));
        set("bookshelfView", or("bookshelfView", json!(0)));
        set("sortingIgnorePrefix", flag("sortingIgnorePrefix"));
        set("sortingPrefixes", or("sortingPrefixes", json!(["the"])));
        set("chromecastEnabled", flag("chromecastEnabled"));
        set("dateFormat", or("dateFormat", json!("MM/dd/yyyy")));
        set("timeFormat", or("timeFormat", json!("HH:mm")));
        set("language", or("language", json!("en-us")));
        set("allowedOrigins", or("allowedOrigins", json!([])));
        set("logLevel", or("logLevel", json!(LOG_LEVEL_INFO)));
        set("version", or_null("version"));
        set("buildNumber", or("buildNumber", json!(0)));
        let message = crate::abs::html::sanitize(get("authLoginCustomMessage").and_then(Value::as_str));
        set("authLoginCustomMessage", if message.is_empty() { Value::Null } else { json!(message) });
        set("authActiveAuthMethods", or("authActiveAuthMethods", json!(["local"])));
        for k in [
            "authOpenIDIssuerURL",
            "authOpenIDAuthorizationURL",
            "authOpenIDTokenURL",
            "authOpenIDUserInfoURL",
            "authOpenIDJwksURL",
            "authOpenIDLogoutURL",
            "authOpenIDClientID",
            "authOpenIDClientSecret",
            "authOpenIDMatchExistingBy",
        ] {
            set(k, or_null(k));
        }
        set("authOpenIDTokenSigningAlgorithm", or("authOpenIDTokenSigningAlgorithm", json!("RS256")));
        set("authOpenIDButtonText", or("authOpenIDButtonText", json!("Login with OpenId")));
        set("authOpenIDAutoLaunch", flag("authOpenIDAutoLaunch"));
        set("authOpenIDAutoRegister", flag("authOpenIDAutoRegister"));
        set("authOpenIDMobileRedirectURIs", or("authOpenIDMobileRedirectURIs", json!(["audiobookshelf://oauth"])));
        set("authOpenIDGroupClaim", or("authOpenIDGroupClaim", json!("")));
        set("authOpenIDAdvancedPermsClaim", or("authOpenIDAdvancedPermsClaim", json!("")));
        match get("authOpenIDSubfolderForRedirectURLs") {
            Some(v) => set("authOpenIDSubfolderForRedirectURLs", v.clone()),
            None => {
                m.remove("authOpenIDSubfolderForRedirectURLs");
            }
        }
        let mut st = ServerSettings(m);

        // the active sign-in methods: a list, OpenID only when set up, never empty
        let mut methods: Vec<Value> =
            st.0.get("authActiveAuthMethods").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![json!("local")]);
        if !st.is_openid_valid() {
            if let Some(i) = methods.iter().position(|m| m == "openid") {
                methods.remove(i);
            }
        }
        if methods.is_empty() {
            methods.push(json!("local"));
        }
        st.0.insert("authActiveAuthMethods".into(), Value::Array(methods));

        // settings renamed in earlier versions
        let defined = |k: &str| get(k).is_some_and(|v| !v.is_null());
        if defined("storeCoverWithBook") {
            st.0.insert("storeCoverWithItem".into(), flag("storeCoverWithBook"));
        }
        if defined("storeMetadataWithBook") {
            st.0.insert("storeMetadataWithItem".into(), flag("storeMetadataWithBook"));
        }
        if !defined("homeBookshelfView") {
            match get("bookshelfView") {
                Some(v) => st.0.insert("homeBookshelfView".into(), v.clone()),
                None => st.0.remove("homeBookshelfView"),
            };
        }
        // only JSON metadata files since 2.4.5
        st.0.insert("metadataFileFormat".into(), json!("json"));

        if let Ok(path) = std::env::var("BACKUP_PATH") {
            if !path.is_empty() {
                st.0.insert("backupPath".into(), json!(path));
            }
        }
        if std::env::var("ALLOW_IFRAME").as_deref() == Ok("1") {
            st.0.insert("allowIframe".into(), json!(true));
        }
        st
    }

    /// A value by key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// A text value by key, empty when not text.
    pub fn str(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    /// The sign-in methods turned on (`authActiveAuthMethods`).
    pub fn active_auth_methods(&self) -> Vec<String> {
        self.get("authActiveAuthMethods")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    }

    /// Whether every OpenID setting needed to sign in with it is set (`isOpenIDAuthSettingsValid`).
    pub fn is_openid_valid(&self) -> bool {
        [
            "authOpenIDIssuerURL",
            "authOpenIDAuthorizationURL",
            "authOpenIDTokenURL",
            "authOpenIDUserInfoURL",
            "authOpenIDJwksURL",
            "authOpenIDClientID",
            "authOpenIDClientSecret",
            "authOpenIDTokenSigningAlgorithm",
        ]
        .iter()
        .all(|k| truthy(self.get(k)))
    }

    /// Everything, as stored (`toJSON`), the token secret included.
    pub fn to_json(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// What the apps see (`toJSONForBrowser`): without the secrets, with the host's time zone.
    pub fn to_json_for_browser(&self) -> Value {
        let mut m = self.0.clone();
        for k in PRIVATE {
            m.remove(*k);
        }
        m.insert("timeZone".into(), json!(host_time_zone()));
        Value::Object(m)
    }

    /// What the sign-in page needs (`authFormData`).
    pub fn auth_form_data(&self) -> Value {
        let message = crate::abs::html::sanitize(self.get("authLoginCustomMessage").and_then(Value::as_str));
        let mut m = Map::new();
        m.insert("authLoginCustomMessage".into(), json!(message));
        if self.active_auth_methods().iter().any(|a| a == "openid") {
            m.insert("authOpenIDButtonText".into(), self.get("authOpenIDButtonText").cloned().unwrap_or(Value::Null));
            m.insert("authOpenIDAutoLaunch".into(), self.get("authOpenIDAutoLaunch").cloned().unwrap_or(Value::Null));
        }
        Value::Object(m)
    }

    /// Applies the changes an admin sent (`update`): only [`PATCHABLE`] keys, stored as sent. Answers whether
    /// anything changed.
    pub fn update(&mut self, payload: &Map<String, Value>) -> bool {
        let mut changed = false;
        for (k, v) in payload {
            if PATCHABLE.contains(&k.as_str()) && self.0.get(k) != Some(v) {
                self.0.insert(k.clone(), v.clone());
                changed = true;
            }
        }
        changed
    }

    /// The sign-in settings an admin edits (`authenticationSettings`), with the sample of the OpenID permissions
    /// claim.
    pub fn authentication_settings(&self) -> Value {
        let mut m = Map::new();
        // a setting the original never set is left out of its JSON
        for k in AUTH_KEYS {
            if let Some(v) = self.get(k) {
                m.insert((*k).to_string(), v.clone());
            }
        }
        m.insert("authOpenIDSamplePermissions".into(), json!(SAMPLE_PERMISSIONS));
        Value::Object(m)
    }

    /// Applies an admin's change to the sign-in settings (`updateAuthSettings`), checking each value like the
    /// original: only known sign-in methods, valid app addresses for OpenID, true or false for the OpenID
    /// switches, text or nothing for the rest (empty text is nothing); the sign-in message is cleaned of unsafe
    /// HTML. Wrong values are skipped. Answers whether anything changed.
    pub fn update_auth_settings(&mut self, payload: &Map<String, Value>) -> bool {
        let mut changed = false;
        for key in AUTH_KEYS.iter().chain(["authOpenIDSamplePermissions"].iter()) {
            let Some(new) = payload.get(*key) else { continue };
            match *key {
                "authActiveAuthMethods" => {
                    let mut methods: Vec<String> = new
                        .as_array()
                        .map(|a| {
                            a.iter().filter_map(Value::as_str).filter(|m| SUPPORTED_AUTH_METHODS.contains(m)).map(str::to_string).collect()
                        })
                        .unwrap_or_default();
                    if methods.is_empty() {
                        tracing::warn!("The sign-in methods sent aren't valid; they were left as they were");
                        continue;
                    }
                    methods.sort();
                    let mut current = self.active_auth_methods();
                    current.sort();
                    self.0.insert((*key).into(), json!(current));
                    if methods != current {
                        self.0.insert((*key).into(), json!(methods));
                        changed = true;
                    }
                }
                "authOpenIDMobileRedirectURIs" => {
                    let Some(uris) = new.as_array() else { continue };
                    let star = uris.iter().any(|u| u == "*");
                    if (star && uris.len() > 1) || uris.iter().any(|u| u != "*" && !u.as_str().is_some_and(is_redirect_uri)) {
                        tracing::warn!("The app addresses for OpenID sent aren't valid; they were left as they were");
                        continue;
                    }
                    let current = self.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
                    if current.iter().any(|u| !uris.contains(u)) || uris.iter().any(|u| !current.contains(u)) {
                        self.0.insert((*key).into(), Value::Array(uris.clone()));
                        changed = true;
                    }
                }
                _ => {
                    if ["authOpenIDAutoLaunch", "authOpenIDAutoRegister"].contains(key) {
                        if !new.is_boolean() {
                            continue;
                        }
                    } else if !new.is_null() && !new.is_string() {
                        continue;
                    }
                    let keeps_empty = *key == "authOpenIDSubfolderForRedirectURLs";
                    let mut value = new.clone();
                    if value == json!("") && !keeps_empty {
                        value = Value::Null;
                    }
                    if *key == "authLoginCustomMessage" {
                        if let Some(text) = value.as_str() {
                            let clean = crate::abs::html::sanitize(Some(text));
                            value = if clean.is_empty() { Value::Null } else { json!(clean) };
                        }
                    }
                    let mut current = if *key == "authOpenIDSamplePermissions" {
                        json!(SAMPLE_PERMISSIONS)
                    } else {
                        self.get(key).cloned().unwrap_or(Value::Null)
                    };
                    if current == json!("") && !keeps_empty {
                        current = Value::Null;
                    }
                    if value != current {
                        // the original sets the sample on the settings object, where it is never stored
                        if *key != "authOpenIDSamplePermissions" {
                            self.0.insert((*key).into(), value);
                        }
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// Reads the stored settings (`Setting.getOldSettings`), or the defaults when none are stored.
    pub fn load(db: &dyn Db, metadata_path: &str) -> Result<ServerSettings> {
        let row: Option<Setting> = models::find_by(db, "key", "server-settings")?;
        Ok(match row.and_then(|r| r.value) {
            Some(Value::Object(stored)) => ServerSettings::construct(&stored, metadata_path),
            _ => ServerSettings::defaults(metadata_path),
        })
    }

    /// Stores the settings (`updateSettingObj`: an upsert of the whole JSON under `server-settings`).
    pub fn save(&self, db: &dyn Db) -> Result<()> {
        save_setting(db, "server-settings", &self.to_json())
    }
}

/// Stores one settings object under its key, like the original's upsert: kept `createdAt`, new `updatedAt`.
pub fn save_setting(db: &dyn Db, key: &str, value: &Value) -> Result<()> {
    let now = models::date_now();
    match models::find_by::<Setting>(db, "key", key)? {
        Some(mut row) => {
            row.value = Some(value.clone());
            row.updated_at = Some(now);
            models::update_by(db, "key", &row)
        }
        None => models::insert(
            db,
            &Setting { key: Some(key.to_string()), value: Some(value.clone()), created_at: Some(now.clone()), updated_at: Some(now) },
        ),
    }
}

/// A stored settings object other than the server's (`email-settings`, `notification-settings`), if any.
pub fn load_setting(db: &dyn Db, key: &str) -> Result<Option<Value>> {
    Ok(models::find_by::<Setting>(db, "key", key)?.and_then(|r| r.value))
}

/// The e-reader devices this user may send books to (`EmailSettings.getEReaderDevices`), from the stored email
/// settings: each device's `availabilityOption` (admins by default, users, guests, or listed users).
pub fn ereader_devices(db: &dyn Db, u: &models::User) -> Result<Vec<Value>> {
    let email = load_setting(db, "email-settings")?.unwrap_or(Value::Null);
    let devices = email.get("ereaderDevices").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(devices.into_iter().filter(|d| device_allowed(d, u)).collect())
}

/// Whether a user may use an e-reader device (`checkUserCanAccessDevice`).
pub fn device_allowed(device: &Value, u: &models::User) -> bool {
    use crate::abs::models::user;
    match device.get("availabilityOption").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("adminOrUp") {
        "adminOrUp" => user::is_admin_or_up(u),
        "userOrUp" => user::is_admin_or_up(u) || user::kind(u) == "user",
        "guestOrUp" => true,
        "specificUsers" => device.get("users").and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x.as_str() == u.id.as_deref())),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_addresses_for_openid_are_checked_like_the_original() {
        for ok in ["audiobookshelf://oauth", "folio://cb", "my_app://host.name-1/a/b.c-d/", "x://h/"] {
            assert!(is_redirect_uri(ok), "{ok}");
        }
        for bad in ["", "oauth", "://host", "a b://h", "app://", "app://h?x=1", "app://h/p q", "https://h:8080/"] {
            assert!(!is_redirect_uri(bad), "{bad}");
        }
    }

    fn fixture_settings() -> Value {
        crate::abs::tests_support::fixture("authorize")["serverSettings"].clone()
    }

    #[test]
    fn a_new_server_shows_the_originals_settings() {
        let mut want = fixture_settings();
        want["backupPath"] = json!("/data/metadata/backups");
        assert_eq!(ServerSettings::defaults("/data/metadata").to_json_for_browser(), want);
    }

    #[test]
    fn stored_settings_read_like_the_original() {
        let stored = ServerSettings::defaults("/m").to_json();
        let back = ServerSettings::construct(stored.as_object().unwrap(), "/m");
        assert_eq!(back, ServerSettings::defaults("/m"));

        let old = json!({
            "storeCoverWithBook": true,
            "bookshelfView": 0,
            "rateLimitLoginRequests": "25",
            "maxBackupSize": 0,
            "authActiveAuthMethods": ["openid"],
            "authLoginCustomMessage": "<script>x</script><b>Hi</b>",
            "metadataFileFormat": "abs"
        });
        let s = ServerSettings::construct(old.as_object().unwrap(), "/m");
        assert_eq!(s.get("storeCoverWithItem"), Some(&json!(true)));
        assert_eq!(s.get("homeBookshelfView"), Some(&json!(0)));
        assert_eq!(s.get("rateLimitLoginRequests"), Some(&json!(25)));
        assert_eq!(s.get("maxBackupSize"), Some(&json!(0)));
        assert_eq!(s.active_auth_methods(), vec!["local"], "OpenID isn't set up, so it falls back to local");
        assert_eq!(s.get("authLoginCustomMessage"), Some(&json!("<b>Hi</b>")));
        assert_eq!(s.get("metadataFileFormat"), Some(&json!("json")));
        assert_eq!(s.get("sortingPrefixes"), Some(&json!(["the"])));
        assert_eq!(s.get("version"), Some(&Value::Null));
        assert_eq!(s.get("buildNumber"), Some(&json!(0)));
    }

    #[test]
    fn only_patchable_settings_change() {
        let mut s = ServerSettings::defaults("/m");
        assert!(!s.update(json!({"language": "en-us"}).as_object().unwrap()));
        assert!(s.update(json!({"language": "de", "tokenSecret": "x"}).as_object().unwrap()));
        assert_eq!(s.str("language"), "de");
        assert_eq!(s.get("tokenSecret"), Some(&Value::Null));
    }

    #[test]
    fn e_reader_devices_follow_their_availability() {
        let db = kit_server_core::SqliteDb::memory();
        crate::abs::create(&db).unwrap();
        let devices = json!({"ereaderDevices": [
            {"name": "admins", "email": "a@x"},
            {"name": "users", "email": "u@x", "availabilityOption": "userOrUp"},
            {"name": "mine", "email": "m@x", "availabilityOption": "specificUsers", "users": ["u1"]}
        ]});
        save_setting(&db, "email-settings", &devices).unwrap();
        let mut u = models::User { id: Some("u1".into()), kind: Some("user".into()), ..Default::default() };
        let names =
            |u: &models::User| ereader_devices(&db, u).unwrap().iter().map(|d| d["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&u), vec!["users", "mine"]);
        u.kind = Some("admin".into());
        u.id = Some("u2".into());
        assert_eq!(names(&u), vec!["admins", "users"]);
    }

    #[test]
    fn settings_are_stored_under_their_key() {
        let db = kit_server_core::SqliteDb::memory();
        crate::abs::create(&db).unwrap();
        assert_eq!(ServerSettings::load(&db, "/m").unwrap(), ServerSettings::defaults("/m"));
        let mut s = ServerSettings::defaults("/m");
        s.update(json!({"dateFormat": "yyyy-MM-dd"}).as_object().unwrap());
        s.save(&db).unwrap();
        s.save(&db).unwrap();
        assert_eq!(ServerSettings::load(&db, "/m").unwrap(), s);
        let n = db.query("SELECT key FROM settings", vec![]).unwrap();
        assert_eq!(n.len(), 1);
    }
}
