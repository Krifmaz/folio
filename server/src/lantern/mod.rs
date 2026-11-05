//! Lantern: the only way Folio downloads. Folio keeps no download clients and no indexers of its own; it
//! knows Lantern's address and API key, and nothing else.
//!
//! | Folio asks | Lantern call |
//! | --- | --- |
//! | search | `GET /api/v1/search?query=&type=search|book&author=&title=&categories=` (the Prowlarr API) |
//! | sign up once | `POST /api/hub/apps` `{name, media_types, callback_url}` |
//! | grab a result | `POST /api/hub/grabs` `{app, media_type, guid, indexer_id, download_url, title, reference}` → `{id, status}` |
//! | how a grab is going | `GET /api/hub/grabs/{id}` → `{id, status, progress, path, files, error}` |
//! | Lantern tells Folio | `POST <callback_url>` with the same grab object, when a grab changes |
//!
//! `docs/LANTERN.md` is the whole contract. Every call sends `X-Api-Key`.

pub mod grabs;
pub mod score;

use crate::metadata::Format;
use kit_server_core::{err, Result};
use score::{Release, SearchQuery};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A runtime in minutes as people say it (`"16 h 10 min"`).
pub fn hours(min: u32) -> String {
    match (min / 60, min % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// One file of a finished grab, where Lantern put it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GrabFile {
    /// the file's full path, as Folio sees it (Lantern's download folder is mounted at the same path)
    pub path: String,
    /// bytes
    #[serde(default)]
    pub size: u64,
}

/// A grab as Lantern reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GrabState {
    /// Lantern's id for the grab
    #[serde(default)]
    pub id: String,
    /// `queued`, `downloading`, `completed` or `failed`
    #[serde(default)]
    pub status: String,
    /// 0 to 1
    #[serde(default)]
    pub progress: f64,
    /// the folder the download finished in
    #[serde(default)]
    pub path: String,
    /// the finished files
    #[serde(default)]
    pub files: Vec<GrabFile>,
    /// why it failed, in Lantern's words
    #[serde(default)]
    pub error: String,
    /// Folio's reference (`wanted-12`), echoed back
    #[serde(default)]
    pub reference: String,
}

/// Lantern, as the settings say.
#[derive(Debug, Clone)]
pub struct Lantern {
    /// the HTTP client
    pub http: reqwest::Client,
    /// Lantern's address (`http://lantern:8080`), without a trailing slash
    pub url: String,
    /// the API key
    pub key: String,
}

impl Lantern {
    /// Lantern from the settings; 400 with what to do when the address or the key is missing.
    pub fn from_settings(http: reqwest::Client, s: &kit_server_settings::Store) -> Result<Lantern> {
        let url = s.str("lantern_url").trim().trim_end_matches('/').to_string();
        let key = s.str("lantern_api_key").trim().to_string();
        if url.is_empty() || key.is_empty() {
            return Err(err(400, "Lantern isn't set up yet. Add its address and API key under Settings → Downloads."));
        }
        Ok(Lantern { http, url, key })
    }

    /// A request to `path` with the API key.
    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http.request(method, format!("{}{path}", self.url)).header("x-api-key", &self.key)
    }

    /// Sends `req` and reads its JSON answer; errors in plain words.
    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let resp = req.send().await.map_err(|e| {
            err(
                502,
                format!(
                    "Lantern can't be reached at {} ({}). Check that it is running and the address under Settings → Downloads.",
                    self.url,
                    if e.is_timeout() { "it took too long to answer" } else { "the connection failed" }
                ),
            )
        })?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(err(502, "Lantern refused Folio's API key. Copy the key from Lantern's settings into Settings → Downloads."));
        }
        if status == reqwest::StatusCode::NOT_FOUND && body.get("detail").is_none() {
            return Err(err(
                502,
                format!("Lantern at {} doesn't offer this yet. Update Lantern to a version with the download hub.", self.url),
            ));
        }
        if !status.is_success() {
            let why = body.get("detail").and_then(Value::as_str).unwrap_or("no reason given").to_string();
            return Err(err(502, format!("Lantern answered with an error ({}): {why}", status.as_u16())));
        }
        Ok(body)
    }

    /// The results of one search in the format's categories.
    pub async fn search(&self, f: Format, q: &SearchQuery) -> Result<Vec<Release>> {
        let cats = score::categories(f).iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let mut params = vec![("query", q.text.clone()), ("categories", cats), ("limit", "100".into())];
        if q.author.is_empty() {
            params.push(("type", "search".into()));
        } else {
            params.extend([("type", "book".into()), ("author", q.author.clone()), ("title", q.title.clone())]);
        }
        let v = self.send(self.req(reqwest::Method::GET, "/api/v1/search").query(&params)).await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Signs Folio up with Lantern's download hub, so Lantern makes its categories and folders and calls back
    /// to `callback_url` (empty: Folio asks instead).
    pub async fn register(&self, callback_url: &str) -> Result<Value> {
        self.send(self.req(reqwest::Method::POST, "/api/hub/apps").json(&json!({
            "name": "Folio",
            "media_types": ["audiobook", "ebook"],
            "callback_url": callback_url,
        })))
        .await
    }

    /// Asks Lantern to download the result `r` for the wanted entry `reference`.
    pub async fn grab(&self, f: Format, r: &Release, reference: &str) -> Result<GrabState> {
        let v = self
            .send(self.req(reqwest::Method::POST, "/api/hub/grabs").json(&json!({
                "app": "Folio",
                "media_type": f.as_str(),
                "guid": r.guid,
                "indexer_id": r.indexer_id,
                "download_url": r.download_url,
                "title": r.title,
                "protocol": r.protocol,
                "reference": reference,
            })))
            .await?;
        let g: GrabState = serde_json::from_value(v).map_err(|e| err(502, format!("Lantern's answer to the grab can't be read: {e}")))?;
        if g.id.is_empty() {
            return Err(err(502, "Lantern took the grab but gave it no id."));
        }
        Ok(g)
    }

    /// How the grab `id` is going.
    pub async fn grab_state(&self, id: &str) -> Result<GrabState> {
        let v = self.send(self.req(reqwest::Method::GET, &format!("/api/hub/grabs/{id}"))).await?;
        serde_json::from_value(v).map_err(|e| err(502, format!("Lantern's answer about the grab can't be read: {e}")))
    }
}
