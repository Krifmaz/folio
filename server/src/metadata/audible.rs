//! Audible's public catalogue (`api.audible.<store>/1.0/catalog`): the audiobooks of an author and the books of a
//! series, so followed authors and series bring their new audiobooks. Only ASINs and the facts needed to choose
//! come from here; the edition itself is always read from Audnexus.

use super::{get_json, js, Providers};
use crate::library::text;
use kit_server_core::Result;
use serde_json::Value;

/// One audiobook in the catalogue: enough to decide whether it is wanted before asking Audnexus.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Product {
    /// the ASIN
    pub asin: String,
    /// the title
    pub title: String,
    /// the authors' names
    pub authors: Vec<String>,
    /// the language, two letters
    pub language: String,
    /// whether it is abridged; `None` when not said
    pub abridged: Option<bool>,
    /// whether it is an audiobook (not a podcast, a show or a bundle of parts)
    pub book: bool,
}

/// The Audible catalogue's address for a store (`us` → `https://api.audible.com`).
pub fn store_url(region: &str) -> String {
    let tld = match region {
        "uk" => "co.uk",
        "ca" => "ca",
        "au" => "com.au",
        "de" => "de",
        "fr" => "fr",
        "es" => "es",
        "it" => "it",
        "in" => "in",
        "jp" => "co.jp",
        _ => "com",
    };
    format!("https://api.audible.{tld}")
}

/// The base address: the `audible_url` setting, or the store's own.
fn base(p: &Providers) -> String {
    if p.audible_url.is_empty() {
        store_url(&p.region)
    } else {
        p.audible_url.clone()
    }
}

/// The audiobooks Audible lists for the author `name`, newest first (up to 50).
pub async fn by_author(p: &Providers, name: &str) -> Result<Vec<Product>> {
    let req = p.http.get(format!("{}/1.0/catalog/products", base(p))).query(&[
        ("author", name),
        ("num_results", "50"),
        ("products_sort_by", "-ReleaseDate"),
        ("response_groups", "contributors,product_attrs,product_desc"),
    ]);
    let v = get_json("Audible", req).await?.unwrap_or(Value::Null);
    Ok(products(&v))
}

/// The ASINs of the books of the series `series_asin`, in order.
pub async fn series_books(p: &Providers, series_asin: &str) -> Result<Vec<String>> {
    let req = p.http.get(format!("{}/1.0/catalog/products/{series_asin}", base(p))).query(&[("response_groups", "relationships")]);
    let v = get_json("Audible", req).await?.unwrap_or(Value::Null);
    Ok(children(&v))
}

/// The products of a catalogue answer.
pub fn products(v: &Value) -> Vec<Product> {
    v.get("products")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| {
                    let delivery = js(x, "content_delivery_type");
                    Product {
                        asin: js(x, "asin"),
                        title: js(x, "title"),
                        authors: x
                            .get("authors")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().map(|p| js(p, "name")).collect())
                            .unwrap_or_default(),
                        language: text::language(&js(x, "language")),
                        abridged: match js(x, "format_type").to_lowercase().as_str() {
                            "abridged" => Some(true),
                            "unabridged" => Some(false),
                            _ => None,
                        },
                        book: delivery.is_empty() || delivery == "SinglePartBook" || delivery == "MultiPartBook",
                    }
                })
                .filter(|p| !p.asin.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The children of a series answer (`product.relationships` of type series), by their place.
pub fn children(v: &Value) -> Vec<String> {
    let mut kids: Vec<(f64, String)> = v
        .pointer("/product/relationships")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|r| js(r, "relationship_to_product") == "child" && js(r, "relationship_type") == "series")
                .map(|r| (js(r, "sort").parse().or_else(|_| js(r, "sequence").parse()).unwrap_or(f64::MAX), js(r, "asin")))
                .filter(|(_, a)| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    kids.sort_by(|a, b| a.0.total_cmp(&b.0));
    kids.into_iter().map(|k| k.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_answers_are_read() {
        let v: Value = serde_json::from_str(include_str!("../../tests/fixtures/audible_author.json")).unwrap();
        let p = products(&v);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0].asin, "B0NEWBOOK1");
        assert_eq!((p[0].language.as_str(), p[0].abridged, p[0].book), ("en", Some(false), true));
        assert_eq!(p[2].abridged, Some(true));
        let s: Value = serde_json::from_str(include_str!("../../tests/fixtures/audible_series.json")).unwrap();
        assert_eq!(children(&s), vec!["B003ZWFO7E", "B0SECOND01"]);
        assert_eq!(store_url("uk"), "https://api.audible.co.uk");
        assert_eq!(store_url("us"), "https://api.audible.com");
    }
}
