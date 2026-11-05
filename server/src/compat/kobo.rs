//! Kobo sync: a Kobo e-reader syncs Folio's ebooks into its library over Wi-Fi, downloads them, and keeps its
//! place in step with Folio's reader, the Android app and KOReader. Written from the public descriptions of the
//! Kobo store's sync protocol (as the Kobo devices speak it); no store calls are passed on.
//!
//! On the Kobo (connected by USB), `.kobo/Kobo/Kobo eReader.conf`, section `[OneStoreServices]`:
//! `api_endpoint=https://<folio>/kobo/<token>`, with the token from Folio's Other apps page.
//!
//! ```text
//! GET    /api/apps/kobo                                (signed in) {token, url}: the Kobo sync address, made on first ask
//! POST   /api/apps/kobo                                (signed in) a new address; the old one stops working
//! GET    /kobo/{token}/v1/initialization            where the Kobo finds everything
//! POST   /kobo/{token}/v1/auth/device               the Kobo's own sign-in (any; the token is the key)
//! GET    /kobo/{token}/v1/library/sync              what's new or changed since the last sync (x-kobo-synctoken)
//! GET    /kobo/{token}/v1/library/{id}/metadata     one book's facts
//! GET    /kobo/{token}/v1/library/{id}/state        the place in it; PUT keeps the Kobo's place
//! DELETE /kobo/{token}/v1/library/{id}              removed on the Kobo: not synced to it again
//! GET    /kobo/{token}/download/{id}/{format}       the EPUB
//! GET    /kobo/{token}/{image}/{w}/{h}[/{q}]/{grey}/image.jpg  the cover
//! *      /kobo/{token}/…                          the rest of the store: an empty answer
//! ```

use super::model::Item;
use super::shelf::{self, Who};
use crate::playback::store as playback;
use crate::AppState;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use kit_server_core::{err, p, Db, Result, RowExt, SignedIn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Books in one sync answer; the Kobo asks again for the rest.
pub const SYNC_PAGE: usize = 100;

/// The routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/kobo", get(get_token).post(new_token))
        .route("/kobo/{token}/v1/initialization", get(initialization))
        .route("/kobo/{token}/v1/auth/device", post(auth_device))
        .route("/kobo/{token}/v1/library/sync", get(sync))
        .route("/kobo/{token}/v1/library/{id}/metadata", get(metadata))
        .route("/kobo/{token}/v1/library/{id}/state", get(get_state).put(put_state))
        .route("/kobo/{token}/v1/library/{id}", axum::routing::delete(remove).fallback(library_other))
        .route("/kobo/{token}/download/{id}/{format}", get(download))
        .route("/kobo/{token}/{*rest}", any(elsewhere))
}

/// How a Kobo token is kept: its SHA-256 in hex.
fn token_key(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// The account of a Kobo sync token; 401 when there is none.
fn owner(st: &AppState, token: &str) -> Result<Who> {
    let row = st
        .db
        .one(
            "SELECT u.id, u.username FROM compat_kobo_tokens t JOIN users u ON u.id=t.user_id WHERE t.token_key=? AND u.disabled=0",
            p![token_key(token)],
        )?
        .ok_or_else(|| {
            err(401, "This Kobo sync address doesn't work any more. Copy the new one from Folio's Other apps page into the Kobo.")
        })?;
    Ok(Who { id: row.i("id"), username: row.s("username") })
}

/// Folio's address as the Kobo reached it (`https://folio.example`).
pub fn origin(headers: &HeaderMap) -> String {
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let host = h("x-forwarded-host").or_else(|| h("host")).unwrap_or_else(|| "localhost".into());
    let proto = h("x-forwarded-proto").unwrap_or_else(|| "http".into());
    format!("{proto}://{}", host.split(',').next().unwrap_or("").trim())
}

/// The token's Kobo sync address (`https://folio.example/kobo/<token>`).
fn base(headers: &HeaderMap, token: &str) -> String {
    format!("{}/kobo/{token}", origin(headers))
}

/// `GET /api/apps/kobo`: the account's Kobo sync address, made the first time.
async fn get_token(State(st): State<AppState>, SignedIn(me): SignedIn, headers: HeaderMap) -> Result<Json<Value>> {
    let kept = st.db.one("SELECT token FROM compat_kobo_tokens WHERE user_id=?", p![me.id])?;
    let token = match kept {
        Some(r) => r.s("token"),
        None => make_token(&st, me.id)?,
    };
    Ok(Json(json!({ "token": token, "url": base(&headers, &token) })))
}

/// `POST /api/apps/kobo`: a new Kobo sync address; the old one stops working.
async fn new_token(State(st): State<AppState>, SignedIn(me): SignedIn, headers: HeaderMap) -> Result<Json<Value>> {
    let token = make_token(&st, me.id)?;
    Ok(Json(json!({ "token": token, "url": base(&headers, &token) })))
}

/// Makes the account's Kobo token (replacing the old one).
fn make_token(st: &AppState, user: i64) -> Result<String> {
    // letters and digits only: it goes into a Kobo's settings file by hand
    let token: String = crate::auth::random_token(24).chars().filter(char::is_ascii_alphanumeric).collect();
    st.db.execute("DELETE FROM compat_kobo_tokens WHERE user_id=?", p![user])?;
    st.db.execute(
        "INSERT INTO compat_kobo_tokens (token_key, token, user_id, created_at) VALUES (?,?,?,?)",
        p![token_key(&token), token, user, super::now()],
    )?;
    Ok(token)
}

/// A UUID for edition `id` (the Kobo names books by UUIDs).
pub fn uuid(id: i64) -> String {
    format!("00000000-0000-4000-8000-{:012x}", id.max(0))
}

/// The edition of a UUID made by [`uuid`].
pub fn edition_of(uuid: &str) -> Option<i64> {
    let tail = uuid.strip_prefix("00000000-0000-4000-8000-")?;
    i64::from_str_radix(tail, 16).ok()
}

/// A time as the Kobo writes it.
fn time(secs: f64) -> String {
    super::opds::rfc3339(secs)
}

/// `GET /kobo/{token}/v1/initialization`: the addresses the Kobo uses. Folio's for the library, sync, books,
/// places and covers; the Kobo store's own for the store.
async fn initialization(State(st): State<AppState>, Path(token): Path<String>, headers: HeaderMap) -> Result<Response> {
    owner(&st, &token)?;
    let me = base(&headers, &token);
    let store = "https://storeapi.kobo.com";
    let mut r = serde_json::Map::new();
    for (k, v) in [
        ("account_page", "https://secure.kobobooks.com/profile"),
        ("account_page_rakuten", "https://my.rakuten.co.jp/"),
        ("add_entitlement", "/v1/library/{RevisionIds}"),
        ("affiliaterequest", "/v1/affiliate"),
        ("audiobook_subscription_orange_deal_inclusion_url", "https://authorize.kobo.com/inclusion"),
        ("authorproduct_recommendations", "/v1/products/books/authors/recommendations"),
        ("autocomplete", "/v1/products/autocomplete"),
        ("blackstone_header", "{\"key\":\"x-amz-request-payer\",\"value\":\"requester\"}"),
        ("book", "/v1/products/books/{ProductId}"),
        ("book_detail_page", "https://store.kobobooks.com/{culture}/ebook/{slug}"),
        ("book_landing_page", "https://store.kobobooks.com/ebooks"),
        ("book_subscription", "/v1/products/books/subscriptions"),
        ("categories", "/v1/categories"),
        ("categories_page", "https://store.kobobooks.com/ebooks/categories"),
        ("category", "/v1/categories/{CategoryId}"),
        ("category_featured_lists", "/v1/categories/{CategoryId}/featured"),
        ("category_products", "/v1/categories/{CategoryId}/products"),
        ("checkout_borrowed_book", "/v1/library/borrow"),
        ("configuration_data", "/v1/configuration"),
        ("content_access_book", "/v1/products/books/{ProductId}/access"),
        ("customer_care_live_chat", "https://v2.zopim.com/widget/livechat.html?key=Y6gwUmnu4OATxN3Tli4Av9bYN319BTdO"),
        ("daily_deal", "/v1/products/dailydeal"),
        ("deals", "/v1/deals"),
        ("delete_entitlement", "/v1/library/{Ids}"),
        ("delete_tag", "/v1/library/tags/{TagId}"),
        ("delete_tag_items", "/v1/library/tags/{TagId}/items/delete"),
        ("device_auth", "/v1/auth/device"),
        ("device_refresh", "/v1/auth/refresh"),
        ("dictionary_host", "https://kbdownload1-a.akamaihd.net"),
        ("discovery_host", "https://discovery.kobobooks.com"),
        ("eula_page", "https://www.kobo.com/termsofuse?style=onestore"),
        ("exchange_auth", "/v1/auth/exchange"),
        ("external_book", "/v1/products/books/external/{Ids}"),
        ("featured_list", "/v1/products/featured/{FeaturedListId}"),
        ("featured_lists", "/v1/products/featured"),
        ("free_books_page", "{\"EN\":\"https://www.kobo.com/{region}/{language}/p/free-ebooks\"}"),
        ("fte_feedback", "/v1/products/ftefeedback"),
        ("get_tests_request", "/v1/analytics/gettests"),
        ("giftcard_epd_redeem_url", "https://www.kobo.com/{storefront}/{language}/redeem-ereader"),
        ("giftcard_redeem_url", "https://www.kobo.com/{storefront}/{language}/redeem"),
        ("help_page", "https://www.kobo.com/help"),
        ("image_host", ""),
        ("image_url_quality_template", "/{ImageId}/{Width}/{Height}/{Quality}/{IsGreyscale}/image.jpg"),
        ("image_url_template", "/{ImageId}/{Width}/{Height}/false/image.jpg"),
        ("kobo_audiobooks_enabled", "False"),
        ("kobo_audiobooks_orange_deal_enabled", "False"),
        ("kobo_audiobooks_subscriptions_enabled", "False"),
        ("kobo_nativeborrow_enabled", "False"),
        ("kobo_onestorelibrary_enabled", "False"),
        ("kobo_redeem_enabled", "False"),
        ("kobo_shelfie_enabled", "False"),
        ("kobo_subscriptions_enabled", "False"),
        ("kobo_superpoints_enabled", "False"),
        ("kobo_wishlist_enabled", "False"),
        ("library_book", "/v1/user/library/books/{LibraryItemId}"),
        ("library_items", "/v1/user/library"),
        ("library_metadata", "/v1/library/{Ids}/metadata"),
        ("library_prices", "/v1/user/library/previews/prices"),
        ("library_search", "/v1/library/search"),
        ("library_sync", "/v1/library/sync"),
        ("love_dashboard_page", "https://store.kobobooks.com/{culture}/kobosuperpoints"),
        ("love_points_redemption_page", "https://store.kobobooks.com/{culture}/KoboSuperPointsRedemption?productId={ProductId}"),
        ("magazine_landing_page", "https://store.kobobooks.com/emagazines"),
        ("notifications_registration_issue", "/v1/notifications/registration"),
        ("oauth_host", "https://oauth.kobo.com"),
        ("password_retrieval_page", "https://www.kobobooks.com/passwordretrieval.html"),
        ("post_analytics_event", "/v1/analytics/event"),
        ("privacy_page", "https://www.kobo.com/privacypolicy?style=onestore"),
        ("product_nextread", "/v1/products/{ProductIds}/nextread"),
        ("product_prices", "/v1/products/{ProductIds}/prices"),
        ("product_recommendations", "/v1/products/{ProductId}/recommendations"),
        ("product_reviews", "/v1/products/{ProductIds}/reviews"),
        ("products", "/v1/products"),
        ("provider_external_sign_in_page", "https://authorize.kobo.com/ExternalSignIn/{providerName}?returnUrl=http://kobo.com/"),
        ("purchase_buy", "https://www.kobo.com/checkout/createpurchase/"),
        ("purchase_buy_templated", "https://www.kobo.com/{culture}/checkout/createpurchase/{ProductId}"),
        ("quickbuy_checkout", "/v1/store/quickbuy/{PurchaseId}/checkout"),
        ("quickbuy_create", "/v1/store/quickbuy/purchase"),
        ("rating", "/v1/products/{ProductId}/rating/{Rating}"),
        ("reading_state", "/v1/library/{Ids}/state"),
        ("redeem_interstitial_page", "https://store.kobobooks.com"),
        ("registration_page", "https://authorize.kobo.com/signup?returnUrl=http://kobo.com/"),
        ("related_items", "/v1/products/{Id}/related"),
        ("remaining_book_series", "/v1/products/books/series/{SeriesId}"),
        ("rename_tag", "/v1/library/tags/{TagId}"),
        ("review", "/v1/products/reviews/{ReviewId}"),
        ("review_sentiment", "/v1/products/reviews/{ReviewId}/sentiment/{Sentiment}"),
        ("shelfie_recommendations", "/v1/user/recommendations/shelfie"),
        ("sign_in_page", "https://authorize.kobo.com/signin?returnUrl=http://kobo.com/"),
        ("social_authorization_host", "https://social.kobobooks.com:8443"),
        ("social_host", "https://social.kobobooks.com"),
        ("stacks_host_productId", "https://store.kobobooks.com/collections/byproductid/"),
        ("store_home", "www.kobo.com/{region}/{language}"),
        ("store_host", "store.kobobooks.com"),
        ("store_newreleases", "https://store.kobobooks.com/{culture}/List/new-releases/961XUjtsU0qxkFItWOutGA"),
        ("store_search", "https://store.kobobooks.com/{culture}/Search?Query={query}"),
        ("store_top50", "https://store.kobobooks.com/{culture}/ebooks/Top"),
        ("tag_items", "/v1/library/tags/{TagId}/Items"),
        ("tags", "/v1/library/tags"),
        ("taste_profile", "/v1/products/tasteprofile"),
        ("update_accessibility_to_preview", "/v1/library/{EntitlementIds}/preview"),
        ("use_one_store", "False"),
        ("user_loyalty_benefits", "/v1/user/loyalty/benefits"),
        ("user_platform", "/v1/user/platform"),
        ("user_profile", "/v1/user/profile"),
        ("user_ratings", "/v1/user/ratings"),
        ("user_recommendations", "/v1/user/recommendations"),
        ("user_reviews", "/v1/user/reviews"),
        ("user_wishlist", "/v1/user/wishlist"),
        ("userguide_host", "https://kbdownload1-a.akamaihd.net"),
        ("wishlist_page", "https://store.kobobooks.com/{region}/{language}/account/wishlist"),
    ] {
        let v = if v.starts_with('/') && !k.starts_with("image_url") { format!("{me}{v}") } else { v.to_string() };
        r.insert(k.to_string(), json!(v));
    }
    // the covers come from Folio, the rest of the store stays the Kobo store's
    r.insert("image_host".into(), json!(origin(&headers)));
    r.insert("image_url_template".into(), json!(format!("{me}/{{ImageId}}/{{Width}}/{{Height}}/false/image.jpg")));
    r.insert(
        "image_url_quality_template".into(),
        json!(format!("{me}/{{ImageId}}/{{Width}}/{{Height}}/{{Quality}}/{{IsGreyscale}}/image.jpg")),
    );
    r.insert("store_api".into(), json!(store));
    let mut resp = Json(json!({ "Resources": r })).into_response();
    // an empty JSON object, base64: what the store sends so the Kobo goes on
    resp.headers_mut().insert("x-kobo-apitoken", HeaderValue::from_static("e30="));
    Ok(resp)
}

/// `POST /kobo/{token}/v1/auth/device` body (the Kobo's own key).
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct DeviceAuth {
    /// the key the Kobo sends
    #[serde(default)]
    pub user_key: String,
}

/// `POST /kobo/{token}/v1/auth/device`: the Kobo's sign-in. The address's token is the key, so any answer will do.
async fn auth_device(State(st): State<AppState>, Path(token): Path<String>, body: Option<Json<DeviceAuth>>) -> Result<Json<Value>> {
    owner(&st, &token)?;
    let key = body.map(|Json(b)| b.user_key).unwrap_or_default();
    Ok(Json(json!({
        "AccessToken": crate::auth::random_token(24),
        "RefreshToken": crate::auth::random_token(24),
        "TokenType": "Bearer",
        "TrackingId": uuid(0),
        "UserKey": key
    })))
}

/// Where the last sync got to: the change time and edition of the last book sent, and the time of the last
/// place sent. Sent to the Kobo as base64 JSON in `x-kobo-synctoken`, and back with the next sync.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SyncMark {
    /// the change time of the last book sent
    #[serde(default)]
    pub t: f64,
    /// that book's edition (books changed at the same time come in edition order)
    #[serde(default)]
    pub id: i64,
    /// the time of the last place sent
    #[serde(default)]
    pub p: f64,
}

impl SyncMark {
    /// The mark the Kobo sent back (none: from the start).
    pub fn read(headers: &HeaderMap) -> SyncMark {
        use base64::Engine;
        headers
            .get("x-kobo-synctoken")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v.trim()).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// The mark as the header's value.
    pub fn write(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(self).unwrap_or_default())
    }
}

/// The ebooks the account syncs to its Kobo: EPUBs that weren't removed on the Kobo.
fn kobo_books(st: &AppState, who: &Who) -> Result<Vec<Item>> {
    let removed: std::collections::HashSet<i64> =
        st.db.query("SELECT edition_id FROM compat_kobo_removed WHERE user_id=?", p![who.id])?.iter().map(|r| r.i("edition_id")).collect();
    Ok(shelf::items(st, who)?
        .into_iter()
        .filter(|i| !i.is_audio() && !removed.contains(&i.id) && i.files.iter().any(|f| f.kind == "epub"))
        .collect())
}

/// When a book last changed, for syncing.
fn changed_at(it: &Item) -> f64 {
    it.updated_at.max(it.added_at)
}

/// A book's entitlement: that it is in the library.
fn entitlement(it: &Item) -> Value {
    let id = uuid(it.id);
    json!({
        "Accessibility": "Full",
        "ActivePeriod": { "From": time(it.added_at) },
        "Created": time(it.added_at),
        "CrossRevisionId": id,
        "Id": id,
        "IsHiddenFromArchive": false,
        "IsLocked": false,
        "IsRemoved": false,
        "LastModified": time(changed_at(it)),
        "OriginCategory": "Imported",
        "RevisionId": id,
        "Status": "Active"
    })
}

/// A book's facts as the Kobo shows them, with where to download it.
fn book_metadata(it: &Item, me: &str) -> Value {
    let id = uuid(it.id);
    let m = &it.meta;
    let f = it.files.iter().find(|f| f.kind == "epub");
    let size = f.map(|f| f.size).unwrap_or(0);
    let downloads: Vec<Value> = ["EPUB3", "EPUB"]
        .iter()
        .map(|fmt| json!({ "Format": fmt, "Size": size, "Url": format!("{me}/download/{}/{}", it.id, fmt.to_lowercase()), "Platform": "Generic" }))
        .collect();
    let mut v = json!({
        "Categories": ["00000000-0000-0000-0000-000000000001"],
        "Contributors": m.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "ContributorRoles": m.authors.iter().map(|a| json!({ "Name": a.name })).collect::<Vec<_>>(),
        "CoverImageId": id,
        "CrossRevisionId": id,
        "CurrentDisplayPrice": { "CurrencyCode": "USD", "TotalAmount": 0 },
        "CurrentLoveDisplayPrice": { "TotalAmount": 0 },
        "Description": super::plain(&m.description),
        "DownloadUrls": downloads,
        "EntitlementId": id,
        "ExternalIds": [],
        "Genre": "00000000-0000-0000-0000-000000000001",
        "IsEligibleForKoboLove": false,
        "IsInternetArchive": false,
        "IsPreOrder": false,
        "IsSocialEnabled": true,
        "Language": if m.language.is_empty() { "en".to_string() } else { m.language.clone() },
        "PhoneticPronunciations": {},
        "PublicationDate": if m.release_date.is_empty() { Value::Null } else { json!(m.release_date) },
        "Publisher": { "Imprint": "", "Name": m.publisher },
        "RevisionId": id,
        "Title": it.full_title(),
        "WorkId": uuid(it.work_id)
    });
    if !m.isbn13.is_empty() {
        v["Isbn"] = json!(m.isbn13);
    }
    if let Some(s) = m.series.first() {
        let n: f64 = s.position.split(['-', ' ']).next().and_then(|p| p.parse().ok()).unwrap_or(0.0);
        v["Series"] = json!({ "Name": s.name, "Number": s.position, "NumberFloat": n, "Id": format!("{:x}", Sha256::digest(crate::library::text::person_key(&s.name).as_bytes()))[..32].to_string() });
    }
    v
}

/// The person's place in a book as the Kobo reads it.
fn reading_state(it: &Item, place: Option<&playback::Progress>) -> Value {
    let id = uuid(it.id);
    let at = place.map(|p| p.updated_at).unwrap_or(it.added_at);
    let (status, times) = match place {
        Some(p) if p.finished => ("Finished", 1),
        Some(p) if p.fraction > 0.0 => ("Reading", 1),
        _ => ("ReadyToRead", 0),
    };
    let mut bookmark = json!({ "LastModified": time(at) });
    if let Some(p) = place {
        bookmark["ProgressPercent"] = json!((p.fraction * 100.0).round());
        if let Some(chapter) = p.locator.get("progression").and_then(Value::as_f64) {
            bookmark["ContentSourceProgressPercent"] = json!((chapter * 100.0).round());
        }
        if let Some(k) = p.locator.get("kobo").filter(|k| k.is_object()) {
            bookmark["Location"] = k.clone();
        } else if let Some(href) = p.locator.get("href").and_then(Value::as_str) {
            bookmark["Location"] = json!({ "Source": href, "Type": "KoboSpan", "Value": "" });
        }
    }
    json!({
        "Created": time(it.added_at),
        "CurrentBookmark": bookmark,
        "EntitlementId": id,
        "LastModified": time(at),
        "PriorityTimestamp": time(at),
        "Statistics": { "LastModified": time(at) },
        "StatusInfo": { "LastModified": time(at), "Status": status, "TimesStartedReading": times }
    })
}

/// `GET /kobo/{token}/v1/library/sync`: the books new or changed since the mark, then the places that moved,
/// 100 books at a time (`x-kobo-sync: continue` while more are waiting).
async fn sync(State(st): State<AppState>, Path(token): Path<String>, headers: HeaderMap) -> Result<Response> {
    let who = owner(&st, &token)?;
    let me = base(&headers, &token);
    let mark = SyncMark::read(&headers);
    let mut books = kobo_books(&st, &who)?;
    books.sort_by(|a, b| changed_at(a).total_cmp(&changed_at(b)).then(a.id.cmp(&b.id)));
    let pending: Vec<&Item> = books.iter().filter(|i| (changed_at(i), i.id) > (mark.t, mark.id)).collect();
    let places: std::collections::HashMap<i64, playback::Progress> =
        playback::all_progress(&st.db, who.id, None)?.into_iter().map(|p| (p.edition_id, p)).collect();
    let mut out = Vec::new();
    let mut next = mark.clone();
    for it in pending.iter().take(SYNC_PAGE) {
        let key = if it.added_at > mark.t || mark.t == 0.0 { "NewEntitlement" } else { "ChangedEntitlement" };
        out.push(json!({ key: {
            "BookEntitlement": entitlement(it),
            "BookMetadata": book_metadata(it, &me),
            "ReadingState": reading_state(it, places.get(&it.id))
        }}));
        next.t = changed_at(it);
        next.id = it.id;
    }
    let more = pending.len() > SYNC_PAGE;
    if !more {
        // places that moved on other devices, for books the Kobo already has
        let sent: std::collections::HashSet<i64> = pending.iter().map(|i| i.id).collect();
        let mut latest = mark.p;
        for it in books.iter().filter(|i| !sent.contains(&i.id)) {
            if let Some(p) = places.get(&it.id).filter(|p| p.updated_at > mark.p && !p.device.starts_with("kobo")) {
                out.push(json!({ "ChangedReadingState": { "ReadingState": reading_state(it, Some(p)) } }));
            }
        }
        for p in places.values() {
            latest = latest.max(p.updated_at);
        }
        next.p = latest;
    }
    let mut resp = Json(Value::Array(out)).into_response();
    if let Ok(v) = HeaderValue::from_str(&next.write()) {
        resp.headers_mut().insert("x-kobo-synctoken", v);
    }
    if more {
        resp.headers_mut().insert("x-kobo-sync", HeaderValue::from_static("continue"));
    }
    Ok(resp)
}

/// The book of a Kobo id that the token's account syncs (404 otherwise).
fn book(st: &AppState, who: &Who, id: &str) -> Result<Item> {
    let edition = edition_of(id).or_else(|| id.parse().ok()).ok_or_else(|| err(404, "That book isn't in Folio's library."))?;
    let it = shelf::item(st, who, edition)?;
    if it.is_audio() {
        return Err(err(404, "That book isn't in Folio's library."));
    }
    Ok(it)
}

/// `GET /kobo/{token}/v1/library/{id}/metadata`.
async fn metadata(State(st): State<AppState>, Path((token, id)): Path<(String, String)>, headers: HeaderMap) -> Result<Json<Value>> {
    let who = owner(&st, &token)?;
    let it = book(&st, &who, &id)?;
    Ok(Json(json!([book_metadata(&it, &base(&headers, &token))])))
}

/// `GET /kobo/{token}/v1/library/{id}/state`.
async fn get_state(State(st): State<AppState>, Path((token, id)): Path<(String, String)>) -> Result<Json<Value>> {
    let who = owner(&st, &token)?;
    let it = book(&st, &who, &id)?;
    let place = playback::progress(&st.db, who.id, it.id)?;
    Ok(Json(json!([reading_state(&it, place.as_ref())])))
}

/// Reads a Kobo time (`2026-11-05T01:47:19.1234567Z`) as seconds since 1970.
pub fn parse_time(s: &str) -> Option<f64> {
    let (date, rest) = s.trim().split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let rest = rest.trim_end_matches('Z');
    let rest = rest.split(['+']).next()?;
    let mut t = rest.split(':');
    let (h, min) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec: f64 = t.next().unwrap_or("0").parse().ok()?;
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86400 + h * 3600 + min * 60) as f64 + sec)
}

/// `PUT /kobo/{token}/v1/library/{id}/state`: keep the Kobo's place as the book's place in Folio (the newest
/// wins).
async fn put_state(State(st): State<AppState>, Path((token, id)): Path<(String, String)>, Json(b): Json<Value>) -> Result<Json<Value>> {
    let who = owner(&st, &token)?;
    let it = book(&st, &who, &id)?;
    let state = b.get("ReadingStates").and_then(|s| s.get(0)).ok_or_else(|| err(400, "The Kobo sent no place. Sync again."))?;
    let kept = playback::progress(&st.db, who.id, it.id)?.unwrap_or_default();
    let bookmark = state.get("CurrentBookmark");
    let at = state
        .get("LastModified")
        .or_else(|| bookmark.and_then(|b| b.get("LastModified")))
        .and_then(Value::as_str)
        .and_then(parse_time)
        .unwrap_or_else(super::now);
    let mut place = playback::Progress { edition_id: it.id, device: "kobo".into(), updated_at: at, ..kept.clone() };
    if let Some(bm) = bookmark {
        if let Some(pc) = bm.get("ProgressPercent").and_then(Value::as_f64) {
            place.fraction = (pc / 100.0).clamp(0.0, 1.0);
        }
        let mut locator = json!({ "total_progression": place.fraction });
        if let Some(chapter) = bm.get("ContentSourceProgressPercent").and_then(Value::as_f64) {
            locator["progression"] = json!((chapter / 100.0).clamp(0.0, 1.0));
        }
        if let Some(loc) = bm.get("Location").filter(|l| l.is_object()) {
            if let Some(src) = loc.get("Source").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                locator["href"] = json!(src);
            }
            locator["kobo"] = loc.clone();
        }
        place.locator = locator;
    }
    if let Some(status) = state.get("StatusInfo").and_then(|s| s.get("Status")).and_then(Value::as_str) {
        place.finished = status == "Finished";
        if place.finished {
            place.fraction = 1.0;
        }
    }
    playback::save_progress(&st.db, who.id, it.id, place)?;
    let ok = json!({ "Result": "Success" });
    Ok(Json(json!({
        "RequestResult": "Success",
        "UpdateResults": [{ "EntitlementId": uuid(it.id), "CurrentBookmarkResult": ok, "StatisticsResult": ok, "StatusInfoResult": ok }]
    })))
}

/// `DELETE /kobo/{token}/v1/library/{id}`: the book was removed on the Kobo, so it isn't synced to it again.
async fn remove(State(st): State<AppState>, Path((token, id)): Path<(String, String)>) -> Result<StatusCode> {
    let who = owner(&st, &token)?;
    let it = book(&st, &who, &id)?;
    st.db.execute(
        "INSERT OR IGNORE INTO compat_kobo_removed (user_id, edition_id, removed_at) VALUES (?,?,?)",
        p![who.id, it.id, super::now()],
    )?;
    Ok(StatusCode::NO_CONTENT)
}

/// Anything else at `/kobo/{token}/v1/library/{name}` (`tags`, `search` …): an empty answer, so the Kobo goes on.
async fn library_other(State(st): State<AppState>, Path((token, name)): Path<(String, String)>) -> Result<Json<Value>> {
    owner(&st, &token)?;
    Ok(Json(if name == "tags" { json!([]) } else { json!({}) }))
}

/// `GET /kobo/{token}/download/{id}/{format}`: the EPUB.
async fn download(
    State(st): State<AppState>,
    Path((token, id, _format)): Path<(String, String, String)>,
    req: Request,
) -> Result<Response> {
    let who = owner(&st, &token)?;
    let it = book(&st, &who, &id)?;
    let f = it.files.iter().find(|f| f.kind == "epub").ok_or_else(|| err(404, "This book has no EPUB for the Kobo."))?;
    super::send_file(&f.path, &f.mime, Some(&f.name), req).await
}

/// The cover of the book a Kobo image id names.
async fn send_cover(st: &AppState, token: &str, image: &str, req: Request) -> Result<Response> {
    let who = owner(st, token)?;
    let it = book(st, &who, image)?;
    match &it.cover {
        Some((path, mime)) => super::send_file(path, mime, None, req).await,
        None => Err(err(404, "This book has no cover.")),
    }
}

/// Covers (`{image}/{w}/{h}[/{q}]/{grey}/image.jpg`), and everything else the Kobo asks (store pages, tags,
/// wishlists, analytics): an empty answer, so it goes on.
async fn elsewhere(State(st): State<AppState>, Path((token, rest)): Path<(String, String)>, req: Request) -> Result<Response> {
    owner(&st, &token)?;
    if rest.ends_with("/image.jpg") {
        let image = rest.split('/').next().unwrap_or("");
        return send_cover(&st, &token, image, req).await;
    }
    let v = match rest.trim_end_matches('/') {
        "v1/library/tags" | "v1/user/wishlist" | "v1/user/recommendations" => json!([]),
        "v1/analytics/gettests" => json!({ "Result": "Success", "TestKey": "", "Tests": {} }),
        "v1/user/loyalty/benefits" => json!({ "Benefits": {} }),
        _ => json!({}),
    };
    Ok(Json(v).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_name_their_edition() {
        assert_eq!(uuid(255), "00000000-0000-4000-8000-0000000000ff");
        assert_eq!(edition_of(&uuid(1234)), Some(1234));
        assert_eq!(edition_of("not-a-uuid"), None);
    }

    #[test]
    fn kobo_times_are_read() {
        assert_eq!(parse_time("2026-11-05T01:47:19Z"), Some(1_793_843_239.0));
        assert_eq!(parse_time("2026-11-05T01:47:19.5000000Z"), Some(1_793_843_239.5));
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn the_sync_mark_goes_there_and_back() {
        let m = SyncMark { t: 12.5, id: 3, p: 7.0 };
        let mut h = HeaderMap::new();
        h.insert("x-kobo-synctoken", HeaderValue::from_str(&m.write()).unwrap());
        assert_eq!(SyncMark::read(&h), m);
        assert_eq!(SyncMark::read(&HeaderMap::new()), SyncMark::default());
    }
}
