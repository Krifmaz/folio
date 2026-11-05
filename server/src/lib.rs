//! The server: the app's state, the kit server crates wired to it, and the web apps: Audiobookshelf's at `/`,
//! Folio's own pages at `/folio/`.
//!
//! | Route | From |
//! | --- | --- |
//! | `GET /api/health` | kit-server-health |
//! | `/api/changelog`, `/api/changelog/new`, `/api/changelog/seen` (What's new), `/api/auth/prefs` (the popup switch) | kit-server-changelog |
//! | `/api/notices`, `/api/notices/check`, `/api/notices/{id}`, `/api/notices/{id}/check` | kit-server-notices |
//! | `/api/bug-reports/*`, `/api/crash-reports` | kit-server-bugreports |
//! | `/api/diagnostics`, `/api/diagnostics/{id}/test` | kit-server-diagnostics |
//! | `/api/settings`, `/api/settings/schema`, `/api/settings/secret/{key}` | kit-server-settings |
//! | `/api/auth/login`, `/api/auth/logout`, `/api/auth/me` | [`auth`] |
//! | `/api/library/*`: works, editions, authors, series, lookups | [`library`] |
//! | `/api/wanted*`, `/api/profiles*`, following authors and series | [`wanted`] |
//! | `/api/wanted/{id}/search`, `/releases`, `/grab`, `/api/grabs`, `/api/lantern/*`: downloads through Lantern | [`lantern`] |
//!
//! Everything the app adds of its own goes into [`app`] next to them.

pub mod abs;
pub mod auth;
pub mod checks;
pub mod compat;
pub mod import;
pub mod lantern;
pub mod library;
pub mod metadata;
pub mod migration;
pub mod playback;
pub mod readarr;
pub mod verify;
pub mod wanted;

use axum::http::{header, HeaderValue};
use axum::Router;
use kit_server_changelog::{Changelog, ChangelogHost};
use kit_server_core::{Account, Host, PrefsStore, SqliteDb, SqlitePrefs};
use kit_server_settings::{DefaultValue, Field, Group, KeyDef, Section, SettingsHost, SqliteSettings, Store, Table, Values};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

/// The settings table generated from settings.toml by build.rs: `Keys` (every setting's name), `TABLE` and
/// `SCHEMA`.
pub mod settings_gen {
    #![allow(missing_docs)]
    use super::{DefaultValue, Field, Group, KeyDef, Section};
    include!(concat!(env!("OUT_DIR"), "/settings_gen.rs"));
}

/// The release notes built in from docs/releases by build.rs: `NOTES`, (version, Markdown) pairs.
mod changelog_gen {
    include!(concat!(env!("OUT_DIR"), "/changelog_gen.rs"));
}

/// The server's settings table (settings.toml).
pub static SETTINGS: Table = Table { keys: settings_gen::TABLE, sections: settings_gen::SCHEMA };

/// The server's version: `APP_VERSION` at build time (the release workflow sets it), else Cargo.toml's.
pub const VERSION: &str = match option_env!("APP_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

/// The binary's name (`CARGO_PKG_NAME`, the project name): the session cookie's prefix, the web app's script
/// name (`<name>-web-<hash>.js`), the `X-Requested-With` value the web app sends and the `x-<name>` mark on every
/// answer.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The port from `APP_PORT`, 8080 when it is not set or not a number.
pub fn port() -> u16 {
    std::env::var("APP_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080)
}

/// The app's state, handed to every route; clones share everything.
#[derive(Clone)]
pub struct AppState {
    /// The database (`<data>/app.db`): the app's own accounts and sessions, and the kit's `kit_*` tables.
    pub db: SqliteDb,
    /// The bug reports and crash logs, in the kit's `kit_bug_report*` and `kit_client_crashes` tables.
    pub reports: kit_server_bugreports::SqliteReports,
    /// Each account's preferences (What's new's seen version and switch), in the kit's `kit_account_prefs`.
    pub prefs: SqlitePrefs,
    /// The settings in effect: environment, then saved (the kit's `kit_settings`), then settings.toml's defaults.
    pub settings: Store,
    /// The data folder: the database, bug report files. The only folder the server writes to.
    pub data_dir: PathBuf,
    /// The built web apps: abs-web's dist at the top (served at `/`), Folio's own pages (trunk's dist) in
    /// `folio/` (served at `/folio/`).
    pub static_dir: PathBuf,
    /// What's new: the release notes built into this server.
    pub changelog: Arc<Changelog>,
    /// The notices (kept: the data folder notice holds its last probe).
    pub notices: kit_server_notices::Notices<AppState>,
    /// Sign-in and bug report limits per address or account.
    pub limits: Arc<auth::Limits>,
    /// The HTTP client for the metadata sources (and later Lantern).
    pub http: reqwest::Client,
    /// The Audiobookshelf port's database (`<data>/absdatabase.sqlite`, the original's schema).
    pub abs: SqliteDb,
    /// The Audiobookshelf port's connected socket clients.
    pub sockets: abs::socket::Sockets,
    /// The Readarr port's database (`<data>/readarr.db`, the original's schema).
    pub readarr: SqliteDb,
    /// Where the Readarr port finds its paths on this machine (`READARR_DISK_ROOT`, else as they are).
    pub readarr_disk: readarr::disk::Disk,
    /// The Readarr port's metadata client, which keeps answers and authors between lookups
    /// (`READARR_GOODREADS_URL` says where Goodreads is, else www.goodreads.com).
    pub readarr_lookup: readarr::bookinfo::BookInfo,
    /// The Audiobookshelf port's settings, tokens and sign-in limit.
    pub abs_server: abs::Server,
}

impl AppState {
    /// The state from the environment: the data folder `APP_DATA_DIR` (default /data) and the web app in
    /// `APP_STATIC_DIR` (default /app/static).
    pub async fn from_env() -> anyhow::Result<AppState> {
        let data = PathBuf::from(std::env::var("APP_DATA_DIR").unwrap_or_else(|_| "/data".into()));
        let static_dir = PathBuf::from(std::env::var("APP_STATIC_DIR").unwrap_or_else(|_| "/app/static".into()));
        let st = AppState::open(&data, static_dir, |name| std::env::var(name).ok()).await?;
        auth::ensure_owner(&st, std::env::var("APP_ADMIN_USER").ok(), std::env::var("APP_ADMIN_PASSWORD").ok())?;
        Ok(st)
    }

    /// The state on the data folder `data` (made when missing) with the web app in `static_dir`; `env` reads
    /// the settings' environment variables (a test passes its own).
    pub async fn open(
        data: &Path,
        static_dir: PathBuf,
        env: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> anyhow::Result<AppState> {
        std::fs::create_dir_all(data).map_err(|e| anyhow::anyhow!("The data folder {} can't be made: {e}", data.display()))?;
        let db = SqliteDb::open(&data.join("app.db")).map_err(|e| anyhow::anyhow!("The database can't be opened: {}", e.detail))?;
        auth::migrate(&db)?;
        library::store::migrate(&db)?;
        wanted::store::migrate(&db)?;
        lantern::grabs::migrate(&db)?;
        playback::migrate(&db)?;
        compat::migrate(&db)?;
        migration::migrate(&db)?;
        let abs = abs::open(data)?;
        let readarr = readarr::open(data)?;
        let readarr_disk = readarr::disk::Disk::new(env("READARR_DISK_ROOT"));
        let readarr_lookup = readarr::bookinfo::BookInfo::new(env("READARR_GOODREADS_URL"));
        let abs_server =
            abs::Server::load(&abs, data).map_err(|e| anyhow::anyhow!("The Audiobookshelf settings can't be read: {}", e.detail))?;
        let kit = |e: kit_server_core::Error| anyhow::anyhow!("The kit's tables can't be made: {}", e.detail);
        let prefs = SqlitePrefs::new(db.clone()).map_err(kit)?;
        let reports = kit_server_bugreports::SqliteReports::new(db.clone()).map_err(kit)?;
        let saved = SqliteSettings::new(db.clone()).map_err(kit)?;
        let notices = checks::notices().keep_in(Arc::new(kit_server_notices::SqliteNotices::new(db.clone()).map_err(kit)?));
        notices.restore().await.map_err(kit)?;
        let settings =
            Store::load_with(saved, SETTINGS, env).await.map_err(|e| anyhow::anyhow!("The settings can't be read: {}", e.detail))?;
        let changelog = Arc::new(Changelog::new(&settings.str("app_name"), VERSION, changelog_gen::NOTES));
        Ok(AppState {
            db,
            reports,
            prefs,
            settings,
            data_dir: data.to_path_buf(),
            static_dir,
            changelog,
            notices,
            limits: Arc::new(auth::Limits::default()),
            http: metadata::http_client(),
            abs,
            sockets: abs::socket::Sockets::default(),
            readarr,
            readarr_disk,
            readarr_lookup,
            abs_server,
        })
    }

    /// The app's name as people see it (the `app_name` setting).
    pub fn app_name(&self) -> String {
        self.settings.str("app_name")
    }
}

impl Host for AppState {
    async fn signed_in(&self, parts: &mut axum::http::request::Parts) -> kit_server_core::Result<Account> {
        auth::signed_in(self, parts)
    }
}

impl ChangelogHost for AppState {
    fn changelog(&self) -> &Changelog {
        &self.changelog
    }

    fn prefs(&self) -> &dyn PrefsStore {
        &self.prefs
    }
}

impl SettingsHost for AppState {
    fn settings(&self) -> &dyn Values {
        &self.settings
    }
}

impl kit_server_bugreports::BugReportsHost for AppState {
    fn data_dir(&self) -> PathBuf {
        self.data_dir.clone()
    }

    fn app_name(&self) -> String {
        AppState::app_name(self)
    }

    fn app_version(&self) -> String {
        VERSION.to_string()
    }

    fn limited(&self, key: &str, limit: usize, window_secs: f64) -> bool {
        self.limits.limited(key, limit, window_secs, true)
    }

    fn reports(&self) -> &dyn kit_server_bugreports::BugReportStore {
        &self.reports
    }

    fn mail_address(&self, user_id: i64) -> kit_server_core::Result<String> {
        use kit_server_core::{Db, RowExt};
        Ok(self.db.one("SELECT email FROM users WHERE id=?", kit_server_core::p![user_id])?.map(|r| r.s("email")).unwrap_or_default())
    }
}

impl kit_server_bugreports::crash::CrashHost for AppState {
    fn crashes(&self) -> &dyn kit_server_bugreports::CrashStore {
        &self.reports
    }
}

impl kit_server_notices::NoticesHost for AppState {
    fn notices(&self) -> kit_server_notices::Notices<AppState> {
        self.notices.clone()
    }
}

impl kit_server_diagnostics::Diagnostics for AppState {
    fn diagnostics(&self) -> kit_server_diagnostics::Registry<AppState> {
        checks::diagnostics()
    }
}

/// The whole app: the kit's routes, the sign-in routes, and the web apps' files with the brotli and gzip
/// copies the image keeps next to each file when the browser takes them. The static folder holds the
/// Audiobookshelf web app (`abs-web/`, at `/`; its index.html answers every path that is not a file, as its
/// pages are drawn in the browser) and Folio's own pages in `folio/` (at `/folio/`).
pub fn app(st: AppState) -> Router {
    let web = kit_server_health::web_build(&st.static_dir.join(OWN_PAGES), &format!("{NAME}-web-"));
    let index = ServeFile::new(st.static_dir.join("index.html")).precompressed_br().precompressed_gzip();
    let spa = ServeDir::new(&st.static_dir).precompressed_br().precompressed_gzip().fallback(index);
    let spa = tower_http::set_header::SetResponseHeader::if_not_present(spa, header::VARY, HeaderValue::from_static("accept-encoding"));
    let page = spa.clone();
    kit_server_notices::spawn_rechecks(st.clone());
    abs::backups::spawn_schedule(st.clone());
    let sockets = abs::socket::layer(st.clone());
    Router::new()
        .merge(kit_server_health::router(kit_server_health::Health::new(VERSION, web)))
        .merge(kit_server_changelog::router())
        .merge(kit_server_changelog::prefs_router("/api/auth/prefs"))
        .merge(kit_server_notices::router())
        .merge(kit_server_bugreports::router())
        .merge(kit_server_bugreports::crash::router())
        .merge(kit_server_diagnostics::router())
        .merge(kit_server_settings::router())
        .merge(auth::router())
        .merge(library::router())
        .merge(wanted::router())
        .merge(lantern::grabs::router())
        .merge(verify::router())
        .merge(import::router())
        .merge(playback::router())
        .merge(compat::router())
        .merge(migration::router())
        .merge(abs::router())
        .merge(readarr::router())
        .method_not_allowed_fallback(move |req: axum::extract::Request| page_or_405(page.clone(), req))
        .with_state(st)
        .fallback_service(spa)
        .layer(sockets)
        .layer(axum::middleware::from_fn(security_headers))
        .layer(tower_http::compression::CompressionLayer::new())
}

/// A request whose path the server knows only for other methods: a page of the web app when it is a `GET` or
/// `HEAD` (Audiobookshelf's web app has pages at `/login` and `/logout`, where its API takes `POST`, as
/// upstream's server does), "405 Method Not Allowed" otherwise.
async fn page_or_405<S>(pages: S, req: axum::extract::Request) -> axum::response::Response
where
    S: tower::Service<axum::extract::Request, Error = std::convert::Infallible>,
    S::Response: axum::response::IntoResponse,
{
    use axum::response::IntoResponse;
    use tower::ServiceExt;
    if req.method() == axum::http::Method::GET || req.method() == axum::http::Method::HEAD {
        match pages.oneshot(req).await {
            Ok(r) => r.into_response(),
            Err(e) => match e {},
        }
    } else {
        axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response()
    }
}

/// Middleware: the security headers, the `x-<name>` mark and the cache policy on every answer.
async fn security_headers(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let path = req.uri().path().to_string();
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.entry("x-content-type-options").or_insert(HeaderValue::from_static("nosniff"));
    h.entry("x-frame-options").or_insert(HeaderValue::from_static("DENY"));
    h.entry("referrer-policy").or_insert(HeaderValue::from_static("same-origin"));
    // every answer says it is this app's: the web app tells a proxy's error page (a 502 while the server is
    // down) from the server's own answers by it
    if let Ok(name) = header::HeaderName::try_from(format!("x-{NAME}")) {
        h.insert(name, HeaderValue::from_static("1"));
    }
    let cache = if path.starts_with("/api/") {
        "no-store"
    } else if hashed_asset(&path) {
        // trunk names the build's files by their content
        "public, max-age=31536000, immutable"
    } else {
        // index.html and the other files: cheap 304s, updates show at once
        "no-cache"
    };
    h.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static(cache));
    resp
}

/// The folder (in the static folder) and path (`/folio/`) of Folio's own pages.
pub const OWN_PAGES: &str = "folio";

/// A file whose name says what is in it, so it never changes under that name:
/// `/folio/<name>-web-f92ab5b84b305a59_bg.wasm` (trunk: a hash of 12 hex digits or more; it drops leading zeros)
/// or anything under `/_nuxt/` (the Audiobookshelf web app's build, named by content).
pub fn hashed_asset(path: &str) -> bool {
    if path.starts_with("/_nuxt/") {
        return true;
    }
    let Some(name) = path.strip_prefix(&format!("/{OWN_PAGES}/")) else { return false };
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let stem = stem.strip_suffix("_bg").unwrap_or(stem);
    !name.contains('/') && stem.rsplit_once('-').is_some_and(|(_, h)| h.len() >= 12 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether the server on `port` of this machine answers `/api/health` with `ok` within five seconds: the
/// Docker image's health check (`<app> healthcheck`), which has no curl. Plain HTTP/1.0 over a TCP socket.
pub async fn healthcheck(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let ask = async {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        s.write_all(b"GET /api/health HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n").await.ok()?;
        let mut answer = Vec::new();
        s.read_to_end(&mut answer).await.ok()?;
        let answer = String::from_utf8_lossy(&answer);
        Some(answer.starts_with("HTTP/1.0 200") || answer.starts_with("HTTP/1.1 200"))
    };
    matches!(tokio::time::timeout(std::time::Duration::from_secs(5), ask).await, Ok(Some(true)))
}

/// Resolves on Ctrl-C or SIGTERM (`docker stop`), so the server finishes the requests it has and exits.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
