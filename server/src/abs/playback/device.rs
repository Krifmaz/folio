//! The device a playback session is listened on (`server/objects/DeviceInfo.js`, stored as
//! `server/models/Device.js`): who the client says it is (app, version, phone model) and what its browser's
//! user agent tells (browser, system), saved once per device id.

use crate::abs::html::strip_all_tags;
use crate::abs::models::{self, Device};
use kit_server_core::{Db, Result};
use serde_json::{json, Map, Value};

/// A device as the original's `DeviceInfo` holds it. Every field is optional; `to_json` leaves out the empty ones.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceInfo {
    /// The stored device's id (`devices.id`).
    pub id: Option<String>,
    /// The user who last used it.
    pub user_id: Option<String>,
    /// The id the client gives its device, or one made up from the request.
    pub device_id: Option<String>,
    /// The address the request came from.
    pub ip_address: Option<String>,
    /// The browser's name, from the user agent.
    pub browser_name: Option<String>,
    /// The browser's version, from the user agent.
    pub browser_version: Option<String>,
    /// The system's name, from the user agent.
    pub os_name: Option<String>,
    /// The system's version, from the user agent.
    pub os_version: Option<String>,
    /// `mobile` or `tablet`, from the user agent.
    pub device_type: Option<String>,
    /// The app's version (the server's own for the web app).
    pub client_version: Option<String>,
    /// The phone's maker.
    pub manufacturer: Option<String>,
    /// The phone's model.
    pub model: Option<String>,
    /// Android's SDK level.
    pub sdk_version: Option<String>,
    /// The app's name, such as "Abs Web" or "Abs Android".
    pub client_name: Option<String>,
    /// A readable name for the device, such as "Linux x86_64 Firefox".
    pub device_name: Option<String>,
}

/// The fields in the order the original writes them, with a way to read and set each.
type Field = (&'static str, fn(&DeviceInfo) -> &Option<String>, fn(&mut DeviceInfo) -> &mut Option<String>);

/// Every field of a device (`toJSON`'s keys, in order).
const FIELDS: &[Field] = &[
    ("id", |d| &d.id, |d| &mut d.id),
    ("userId", |d| &d.user_id, |d| &mut d.user_id),
    ("deviceId", |d| &d.device_id, |d| &mut d.device_id),
    ("ipAddress", |d| &d.ip_address, |d| &mut d.ip_address),
    ("browserName", |d| &d.browser_name, |d| &mut d.browser_name),
    ("browserVersion", |d| &d.browser_version, |d| &mut d.browser_version),
    ("osName", |d| &d.os_name, |d| &mut d.os_name),
    ("osVersion", |d| &d.os_version, |d| &mut d.os_version),
    ("deviceType", |d| &d.device_type, |d| &mut d.device_type),
    ("clientVersion", |d| &d.client_version, |d| &mut d.client_version),
    ("manufacturer", |d| &d.manufacturer, |d| &mut d.manufacturer),
    ("model", |d| &d.model, |d| &mut d.model),
    ("sdkVersion", |d| &d.sdk_version, |d| &mut d.sdk_version),
    ("clientName", |d| &d.client_name, |d| &mut d.client_name),
    ("deviceName", |d| &d.device_name, |d| &mut d.device_name),
];

/// The fields the original strips of HTML when it reads a stored device (`DeviceInfo.stringFields`).
const STRING_FIELDS: &[&str] = &["deviceId", "clientVersion", "manufacturer", "model", "sdkVersion", "clientName", "deviceName"];

/// A text value of a client's field, without HTML; empty for anything that isn't text (`stripAllTags`).
fn stripped(v: Option<&Value>) -> String {
    strip_all_tags(v.and_then(Value::as_str))
}

/// `Some(text)` unless the text is empty (JavaScript's `text || null`).
fn some(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

impl DeviceInfo {
    /// A device from stored or sent JSON (`new DeviceInfo(json)`): known keys only, text fields without HTML (an
    /// empty text when `null`, like the original).
    pub fn from_json(v: &Value) -> DeviceInfo {
        let mut d = DeviceInfo::default();
        let Some(m) = v.as_object() else { return d };
        for (key, _, set) in FIELDS {
            match m.get(*key) {
                None => {}
                // `stripAllTags(null)` is an empty text, which the original then keeps.
                Some(Value::Null) if STRING_FIELDS.contains(key) => *set(&mut d) = Some(String::new()),
                Some(Value::Null) => {}
                Some(Value::String(s)) => {
                    *set(&mut d) = Some(if STRING_FIELDS.contains(key) { strip_all_tags(Some(s)) } else { s.clone() })
                }
                Some(other) => *set(&mut d) = Some(if STRING_FIELDS.contains(key) { String::new() } else { other.to_string() }),
            }
        }
        d
    }

    /// The device as JSON, empty fields left out (`toJSON`).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for (key, get, _) in FIELDS {
            if let Some(v) = get(self) {
                m.insert((*key).into(), Value::from(v.clone()));
            }
        }
        Value::Object(m)
    }

    /// How the device is named in the logs and progress events (`deviceDescription`).
    pub fn description(&self) -> String {
        let n = |v: &Option<String>| v.clone().unwrap_or_else(|| "null".into());
        if let Some(model) = &self.model {
            if self.sdk_version.is_some() {
                return format!("{model} SDK {} / v{}", n(&self.sdk_version), n(&self.client_version));
            }
            return format!("{model} / v{}", n(&self.client_version));
        }
        format!("{} {} / {}", n(&self.os_name), n(&self.os_version), n(&self.browser_name))
    }

    /// A made-up device id when the client gives none (`getTempDeviceId`).
    fn temp_device_id(&self) -> String {
        use base64::Engine;
        let keys = [
            &self.user_id,
            &self.browser_name,
            &self.browser_version,
            &self.os_name,
            &self.os_version,
            &self.client_version,
            &self.manufacturer,
            &self.model,
            &self.sdk_version,
            &self.ip_address,
        ]
        .map(|k| k.clone().unwrap_or_default());
        format!("temp-{}", base64::engine::general_purpose::STANDARD.encode(keys.join("-")))
    }

    /// A new device from a request (`setData`): the address, the user agent and what the client sent.
    pub fn from_request(ip: Option<String>, user_agent: &str, client: &Value, server_version: &str, user_id: Option<String>) -> DeviceInfo {
        let ua = parse_user_agent(user_agent);
        let id = models::new_id();
        let mut d = DeviceInfo {
            device_id: Some(match client.get("deviceId") {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                Some(v) if crate::abs::models::media_progress::js_truthy(Some(v)) => v.to_string(),
                _ => id.clone(),
            }),
            id: Some(id),
            user_id,
            ip_address: ip.filter(|i| !i.is_empty()),
            browser_name: ua.browser_name,
            browser_version: ua.browser_version,
            os_name: ua.os_name,
            os_version: ua.os_version,
            device_type: ua.device_type,
            client_version: Some(some(stripped(client.get("clientVersion"))).unwrap_or_else(|| server_version.to_string())),
            manufacturer: some(stripped(client.get("manufacturer"))),
            model: some(stripped(client.get("model"))),
            ..DeviceInfo::default()
        };
        d.sdk_version = match client.get("sdkVersion") {
            Some(Value::Number(n)) => Some(crate::abs::library::util::js_num(n.as_f64().unwrap_or(f64::NAN)).to_string()),
            v => some(stripped(v)),
        };
        d.client_name = some(stripped(client.get("clientName")));
        let maker = || d.manufacturer.clone().unwrap_or_else(|| "Unknown".into());
        if d.sdk_version.is_some() {
            let name = format!("{} {}", maker(), d.model.clone().unwrap_or_default());
            d.client_name.get_or_insert_with(|| "Abs Android".into());
            d.device_name = Some(name);
        } else if d.model.is_some() {
            let name = format!("{} {}", maker(), d.model.clone().unwrap_or_default());
            d.client_name.get_or_insert_with(|| "Abs iOS".into());
            d.device_name = Some(name);
        } else if let (Some(os), Some(browser)) = (&d.os_name, &d.browser_name) {
            let name = format!("{os} {} {browser}", d.os_version.clone().unwrap_or_else(|| "N/A".into()));
            d.client_name.get_or_insert_with(|| "Abs Web".into());
            d.device_name = Some(name);
        } else if d.client_name.is_none() {
            d.client_name = Some("Unknown".into());
        }
        if d.device_id.as_deref().unwrap_or_default().is_empty() {
            d.device_id = Some(d.temp_device_id());
        }
        d
    }

    /// Takes over a newer description of the same device (`update`): every field but the ids. Answers whether
    /// anything changed.
    pub fn update(&mut self, newer: &DeviceInfo) -> bool {
        let mut changed = false;
        for (key, get, set) in FIELDS {
            if *key == "id" || *key == "deviceId" {
                continue;
            }
            let (new, old) = (get(newer).clone(), get(self).clone());
            if new.is_some() && new != old {
                *set(self) = new;
                changed = true;
            } else if new.is_none() && old.as_deref().is_some_and(|o| !o.is_empty()) {
                *set(self) = None;
                changed = true;
            }
        }
        changed
    }

    /// The device as the original reads it back from its row (`Device.getOldDevice`).
    pub fn from_row(row: &Device) -> DeviceInfo {
        let extra = row.extra_data.clone().unwrap_or(Value::Null);
        let x = |k: &str| extra.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let android = row.client_name.as_deref() == Some("Abs Android");
        let version = row.device_version.clone().filter(|v| !v.is_empty());
        DeviceInfo::from_json(&json!({
            "id": row.id,
            "deviceId": row.device_id,
            "userId": row.user_id,
            "ipAddress": row.ip_address,
            "browserName": x("browserName"),
            "browserVersion": if android { None } else { version.clone() },
            "osName": x("osName"),
            "osVersion": x("osVersion"),
            "clientVersion": row.client_version.clone().filter(|v| !v.is_empty()),
            "manufacturer": x("manufacturer"),
            "model": x("model"),
            "sdkVersion": if android { version } else { None },
            "deviceName": row.device_name,
            "clientName": row.client_name,
        }))
    }

    /// The row the original stores for a device (`Device.getFromOld`), keeping `base`'s creation time.
    fn to_row(&self, base: Option<Device>) -> Device {
        let mut extra = Map::new();
        for (key, v) in [
            ("manufacturer", &self.manufacturer),
            ("model", &self.model),
            ("osName", &self.os_name),
            ("osVersion", &self.os_version),
            ("browserName", &self.browser_name),
        ] {
            if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
                extra.insert(key.into(), Value::from(v.clone()));
            }
        }
        let now = models::date_now();
        let base = base.unwrap_or_else(|| Device { created_at: Some(now.clone()), ..Device::default() });
        let nonempty = |v: &Option<String>| v.clone().filter(|v| !v.is_empty());
        Device {
            id: self.id.clone(),
            device_id: self.device_id.clone(),
            client_name: nonempty(&self.client_name),
            client_version: nonempty(&self.client_version),
            ip_address: self.ip_address.clone(),
            device_name: nonempty(&self.device_name),
            device_version: nonempty(&self.sdk_version).or_else(|| nonempty(&self.browser_version)),
            extra_data: Some(Value::Object(extra)),
            created_at: base.created_at,
            updated_at: Some(now),
            user_id: self.user_id.clone(),
        }
    }
}

/// The device of a request (`PlaybackSessionManager.getDeviceInfo`): a device the client has used before is
/// brought up to date and reused; otherwise the new device is stored.
pub fn for_request(db: &dyn Db, fresh: DeviceInfo) -> Result<DeviceInfo> {
    if let Some(device_id) = fresh.device_id.clone().filter(|_| fresh.device_id.is_some()) {
        if let Some(row) = models::find_by::<Device>(db, "deviceId", device_id.as_str())? {
            let mut existing = DeviceInfo::from_row(&row);
            if existing.update(&fresh) {
                models::update(db, &existing.to_row(Some(row)))?;
            }
            return Ok(existing);
        }
    }
    models::insert(db, &fresh.to_row(None))?;
    Ok(fresh)
}

/// The stored device with this id, as the original reads it.
pub fn by_id(db: &dyn Db, id: &str) -> Result<Option<DeviceInfo>> {
    Ok(models::find::<Device>(db, id)?.map(|row| DeviceInfo::from_row(&row)))
}

/// What a user agent says about the browser, system and kind of device.
#[derive(Debug, Default, PartialEq)]
pub struct UserAgent {
    /// The browser's name.
    pub browser_name: Option<String>,
    /// The browser's version.
    pub browser_version: Option<String>,
    /// The system's name.
    pub os_name: Option<String>,
    /// The system's version.
    pub os_version: Option<String>,
    /// `mobile` or `tablet`.
    pub device_type: Option<String>,
}

/// Where `needle` starts in `hay`, ignoring case.
fn find(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
}

/// The version-like text (`[\w.]`) that starts at `at`.
fn word_at(ua: &str, at: usize) -> String {
    ua[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect()
}

/// The text matched for `token` (keeping the user agent's own case) and the version after it.
fn token(ua: &str, token: &str) -> Option<(String, String)> {
    let at = find(ua, token)?;
    let name = ua[at..at + token.len()].trim_end_matches(['/', ' ']).to_string();
    Some((name, word_at(ua, at + token.len())))
}

/// `Some(text)` unless it is empty.
fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

/// The browser, system and device kind of a user agent, by the rules of the original's UAParser.js (0.7.31) for
/// the browsers and systems people use: Chrome, Edge, Opera, Samsung Internet, Firefox and Safari; Windows, macOS,
/// iOS, Android, ChromeOS and Linux.
pub fn parse_user_agent(ua: &str) -> UserAgent {
    let ua: String = ua.chars().take(255).collect();
    let ua = ua.as_str();
    let mut out = UserAgent::default();
    let has = |t: &str| find(ua, t).is_some();
    let browser: Option<(String, String)> = if let Some((_, v)) = token(ua, "crios/").or_else(|| token(ua, "crmo/")) {
        Some(("Chrome".into(), v))
    } else if let Some((_, v)) = ["edgios/", "edga/", "edge/", "edg/"].iter().find_map(|t| token(ua, t)) {
        Some(("Edge".into(), v))
    } else if let Some((_, v)) = token(ua, "opr/") {
        Some(("Opera".into(), v))
    } else if let Some((_, v)) = token(ua, "samsungbrowser/") {
        Some(("Samsung Browser".into(), v))
    } else if let Some((_, v)) = token(ua, "fxios/") {
        Some(("Firefox".into(), v))
    } else if let Some((_, v)) = token(ua, "headlesschrome/") {
        Some(("Chrome Headless".into(), v))
    } else if has(" wv)") && has("chrome/") {
        token(ua, "chrome/").map(|(_, v)| ("Chrome WebView".into(), v))
    } else if has("droid") && has(" version/") && has("safari") && !has("chrome/") {
        token(ua, " version/").map(|(_, v)| ("Android Browser".into(), v))
    } else if let Some(found) = token(ua, "chrome/") {
        Some(found)
    } else if has("version/") && has("safari") {
        let (_, v) = token(ua, "version/").unwrap_or_default();
        let mobile = find(ua, "mobile/").is_some();
        Some((if mobile { "Mobile Safari".into() } else { "Safari".into() }, v))
    } else {
        token(ua, "firefox/")
    };
    if let Some((name, version)) = browser {
        out.browser_name = opt(name);
        out.browser_version = opt(version);
    }
    let os: Option<(String, String)> = if let Some(at) = find(ua, "windows nt ") {
        let v = word_at(ua, at + 11);
        let mapped = match v.as_str() {
            "10.0" | "6.4" => "10",
            "6.3" => "8.1",
            "6.2" => "8",
            "6.1" => "7",
            "6.0" => "Vista",
            "5.1" | "5.2" => "XP",
            "5.0" => "2000",
            _ => v.as_str(),
        };
        Some(("Windows".into(), mapped.to_string()))
    } else if has("iphone") || has("ipad") || has("ipod") {
        let v = find(ua, " os ").map(|at| word_at(ua, at + 4).replace('_', ".")).unwrap_or_default();
        Some(("iOS".into(), v))
    } else if let Some(at) = find(ua, "mac os x") {
        let rest: String = ua[at + 8..].trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || "_. ".contains(*c)).collect();
        Some(("Mac OS".into(), rest.trim_end().replace('_', ".")))
    } else if let Some((name, v)) = token(ua, "android ").or_else(|| token(ua, "android/")).or_else(|| token(ua, "android")) {
        Some((name.trim().to_string(), v))
    } else if let Some(at) = find(ua, "cros ") {
        let arch = word_at(ua, at + 5);
        let v = word_at(ua, at + 5 + arch.len() + 1);
        Some(("Chromium OS".into(), v))
    } else if let Some((name, v)) = ["ubuntu", "debian", "fedora", "centos", "arch linux", "manjaro", "gentoo"]
        .iter()
        .find_map(|d| find(ua, d).map(|at| (ua[at..at + d.len()].to_string(), word_at(ua, at + d.len() + 1))))
    {
        Some((name, v))
    } else if let Some(at) = find(ua, "linux") {
        let name = ua[at..at + 5].to_string();
        let after = &ua[at + 5..];
        let v = if let Some(stripped) = after.strip_prefix(' ') { word_at(stripped, 0) } else { word_at(after, 0) };
        Some((name, v))
    } else {
        None
    };
    if let Some((name, version)) = os {
        out.os_name = opt(name);
        out.os_version = opt(version);
    }
    out.device_type = if has("ipad") || has("tablet") || (has("droid") && !has("mobile") && has("safari")) {
        Some("tablet".into())
    } else if has("iphone") || has("ipod") || has("mobile") {
        Some("mobile".into())
    } else {
        None
    };
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agents_read_like_the_originals_parser() {
        let ff = parse_user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0");
        assert_eq!(
            (ff.browser_name.as_deref(), ff.browser_version.as_deref(), ff.os_name.as_deref(), ff.os_version.as_deref()),
            (Some("Firefox"), Some("132.0"), Some("Linux"), Some("x86_64"))
        );
        let ch = parse_user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
        );
        assert_eq!(
            (ch.browser_name.as_deref(), ch.browser_version.as_deref(), ch.os_name.as_deref(), ch.os_version.as_deref()),
            (Some("Chrome"), Some("130.0.0.0"), Some("Windows"), Some("10"))
        );
        let ios = parse_user_agent(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
        );
        assert_eq!(
            (ios.browser_name.as_deref(), ios.os_name.as_deref(), ios.os_version.as_deref(), ios.device_type.as_deref()),
            (Some("Mobile Safari"), Some("iOS"), Some("17.5"), Some("mobile"))
        );
        assert_eq!(parse_user_agent("Python-urllib/3.12"), UserAgent::default());
    }

    #[test]
    fn a_phone_is_named_from_what_the_app_sends() {
        let d = DeviceInfo::from_request(
            Some("10.0.0.2".into()),
            "okhttp/4.12",
            &json!({"deviceId": "p-1", "manufacturer": "Google", "model": "Pixel 8", "sdkVersion": 34, "clientVersion": "0.9.81"}),
            "2.37.2",
            Some("u".into()),
        );
        assert_eq!(d.client_name.as_deref(), Some("Abs Android"));
        assert_eq!(d.device_name.as_deref(), Some("Google Pixel 8"));
        assert_eq!(d.sdk_version.as_deref(), Some("34"));
        assert_eq!(d.description(), "Pixel 8 SDK 34 / v0.9.81");
        let web = DeviceInfo::from_request(None, "", &json!({}), "2.37.2", None);
        assert_eq!((web.client_name.as_deref(), web.client_version.as_deref()), (Some("Unknown"), Some("2.37.2")));
        assert_eq!(web.device_id, web.id);
    }
}
