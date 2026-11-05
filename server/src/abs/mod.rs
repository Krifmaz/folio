//! The port of Audiobookshelf's server (v2.37.2, see UPSTREAM.lock): its database, its API and its socket,
//! module by module, so that Audiobookshelf's own web and mobile apps (forked in `abs-web/` and `abs-app/`) run
//! against Folio unchanged. docs/PARITY.md lists every endpoint, setting and job, and docs/TRACKS.md which track
//! ports which module.
//!
//! The data lives in a database of its own, `<data>/absdatabase.sqlite`, with exactly the original's schema
//! ([`SCHEMA`], recorded from the original's image). An existing Audiobookshelf database can therefore be used as
//! it is.

pub mod admin;
pub mod api_keys;
pub mod auth;
pub mod bookmarks;
pub mod cron;
pub mod email;
pub mod html;
pub mod items;
pub mod library;
pub mod metadata_providers;
pub mod models;
pub mod playback;
pub mod progress;
pub mod scanner;
pub mod sessions;
pub mod settings;
pub mod socket;
pub mod tasks;
pub mod users;

use crate::AppState;
use axum::Router;
use kit_server_core::{Db, SqliteDb};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The original's schema at the pinned version: tables, indexes, triggers and the `migrationsMeta` rows.
pub const SCHEMA: &str = include_str!("schema.sql");

/// The original's version this port answers as (`serverVersion`), from UPSTREAM.lock's `abs-server` pin.
pub const ABS_VERSION: &str = "2.37.2";

/// Opens (and on first use creates) the Audiobookshelf database in the data folder.
pub fn open(data: &Path) -> anyhow::Result<SqliteDb> {
    let db = SqliteDb::open(&data.join("absdatabase.sqlite"))
        .map_err(|e| anyhow::anyhow!("The Audiobookshelf database can't be opened: {}", e.detail))?;
    create(&db).map_err(|e| anyhow::anyhow!("The Audiobookshelf database can't be made: {}", e.detail))?;
    Ok(db)
}

/// Creates the original's tables in an empty database; a database that has them already (made by Folio or by
/// Audiobookshelf itself) is left as it is.
pub fn create(db: &SqliteDb) -> kit_server_core::Result<()> {
    let made = db.one("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'libraryItems'", vec![])?;
    if made.is_none() {
        db.execute_batch(SCHEMA)?;
    }
    Ok(())
}

/// The port's server-wide state: the settings, the tokens, the sign-in limit and the folders the original
/// reports. Clones share the same settings.
#[derive(Clone)]
pub struct Server {
    /// The server settings (`Database.serverSettings`), saved with [`Server::save_settings`].
    settings: Arc<RwLock<settings::ServerSettings>>,
    /// Makes and checks tokens.
    pub tokens: Arc<auth::TokenManager>,
    /// The limit on sign-in attempts.
    pub limiter: Arc<auth::RateLimiter>,
    /// The original's `ConfigPath`: the folder of the database (Folio's data folder).
    pub config_path: PathBuf,
    /// The original's `MetadataPath`: covers, caches and backups (`<data>/metadata`).
    pub metadata_path: PathBuf,
    /// The running tasks the apps show.
    pub tasks: Arc<tasks::TaskManager>,
    /// How the server was installed (`Source`, from `SOURCE`; `docker` by default, like the original's image).
    pub source: String,
}

impl Server {
    /// Loads the settings from the database and sets up the tokens (`initTokenSecret`): a new secret is made and
    /// stored on first start, unless `JWT_SECRET_KEY` gives one. A new version is recorded like the original's
    /// `loadData` does.
    pub fn load(db: &SqliteDb, data: &Path) -> kit_server_core::Result<Server> {
        let metadata_path = data.join("metadata");
        let mut st = settings::ServerSettings::load(db, &metadata_path.display().to_string())?;
        let (tokens, made) = auth::TokenManager::new(st.get("tokenSecret").and_then(Value::as_str));
        let mut changed = false;
        if let Some(secret) = made {
            st.0.insert("tokenSecret".into(), secret.into());
            changed = true;
        }
        if st.str("version") != ABS_VERSION || st.get("buildNumber").and_then(Value::as_i64) != Some(settings::BUILD_NUMBER) {
            st.0.insert("version".into(), ABS_VERSION.into());
            st.0.insert("buildNumber".into(), settings::BUILD_NUMBER.into());
            changed = true;
        }
        if changed {
            st.save(db)?;
        }
        Ok(Server {
            settings: Arc::new(RwLock::new(st)),
            tokens: Arc::new(tokens),
            limiter: Arc::new(auth::RateLimiter::from_env()),
            config_path: data.to_path_buf(),
            metadata_path,
            tasks: Arc::new(tasks::TaskManager::default()),
            source: std::env::var("SOURCE").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "docker".into()),
        })
    }

    /// A copy of the settings in effect.
    pub fn settings(&self) -> settings::ServerSettings {
        self.settings.read().map(|s| s.clone()).unwrap_or_else(|e| e.into_inner().clone())
    }

    /// Replaces the settings and stores them (`Database.updateServerSettings`).
    pub fn save_settings(&self, db: &dyn Db, new: settings::ServerSettings) -> kit_server_core::Result<()> {
        new.save(db)?;
        match self.settings.write() {
            Ok(mut s) => *s = new,
            Err(e) => *e.into_inner() = new,
        }
        Ok(())
    }
}

/// The user a socket's token signs in as, or the original's message for `auth_failed` (`authenticateSocket`):
/// API keys, old tokens and access tokens are accepted; refresh tokens, expired tokens and inactive users are not.
/// An expired API key is turned off.
pub fn socket_user(st: &AppState, token: &str) -> Result<models::User, &'static str> {
    use models::{user, ApiKey};
    let db = &st.abs;
    let Ok(claims) = auth::jwt::verify(token, &st.abs_server.tokens.secret(), true) else { return Err("Invalid token") };
    let expired = claims.get("exp").and_then(Value::as_f64).is_some_and(|exp| exp < models::now_ms() as f64 / 1000.0);
    let found = if claims.get("type").and_then(Value::as_str) == Some("api") {
        let key_id = claims.get("keyId").and_then(Value::as_str).unwrap_or_default();
        let Some(mut key) = models::find::<ApiKey>(db, key_id).ok().flatten().filter(|k| k.is_active == Some(true)) else {
            return Err("Invalid API key");
        };
        if expired {
            key.is_active = Some(false);
            key.updated_at = Some(models::date_now());
            let _ = models::update(db, &key);
            return Err("API key expired");
        }
        user::by_id(db, key.user_id.as_deref().unwrap_or_default()).ok().flatten().ok_or("Invalid API key")?
    } else {
        if !auth::is_bearer_access_payload(&claims) {
            return Err("Invalid token");
        }
        if expired {
            return Err("Token expired");
        }
        user::by_id_or_old_id(db, claims.get("userId").and_then(Value::as_str).unwrap_or_default()).ok().flatten().ok_or("Invalid token")?
    };
    if !user::is_active(&found) {
        return Err("Invalid user");
    }
    Ok(found)
}

/// Every ported route. Each track adds its module's router here (one line each, docs/TRACKS.md).
pub fn router() -> Router<AppState> {
    Router::new()
        .merge(auth::routes::router())
        .merge(progress::router())
        .merge(bookmarks::router())
        .merge(sessions::router())
        .merge(library::router())
        .merge(items::router())
        .merge(scanner::router())
        .merge(users::router())
        .merge(api_keys::router())
        .merge(admin::router())
        .merge(metadata_providers::router())
        .merge(email::router())
}

/// Test helpers: the recorded answers of the original and the comparison of tests/parity.
#[cfg(test)]
pub(crate) mod tests_support;

#[cfg(test)]
mod tests {
    use super::*;
    use kit_server_core::RowExt;

    #[test]
    fn the_schema_makes_every_table_once() {
        let db = SqliteDb::memory();
        create(&db).unwrap();
        create(&db).unwrap();
        let tables = db.query("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name", vec![]).unwrap();
        let names: Vec<String> = tables.iter().map(|r| r.s("name")).collect();
        assert_eq!(names.len(), 26, "{names:?}");
        for t in ["libraryItems", "users", "mediaProgresses", "playbackSessions", "feeds", "apiKeys"] {
            assert!(names.iter().any(|n| n == t), "{t} missing");
        }
        let v = db.one("SELECT value FROM migrationsMeta WHERE key = 'version'", vec![]).unwrap().unwrap();
        assert_eq!(v.s("value"), ABS_VERSION);
    }
}
