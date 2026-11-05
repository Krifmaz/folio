//! The port of `server/utils/htmlSanitizer.js`: the HTML the original lets through in text people write
//! (descriptions, the sign-in page message), with the same allowed tags, attributes and link schemes.

/// The original's `sanitize`: keeps paragraphs, lists, links (`href`, `name`, `target`; http, https and mailto
/// only), bold, italic, strike-through and line breaks; drops every other tag but keeps its text. Anything that
/// isn't a string gives `""`, like the original.
pub fn sanitize(html: Option<&str>) -> String {
    let Some(html) = html else { return String::new() };
    ammonia::Builder::default()
        .tags(["p", "ol", "ul", "li", "a", "strong", "em", "del", "br", "b", "i"].into_iter().collect())
        .generic_attributes(Default::default())
        .tag_attributes([("a", ["href", "name", "target"].into_iter().collect())].into_iter().collect())
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(None)
        .clean(html)
        .to_string()
}

/// The original's `stripAllTags`: the text without any tag, entities decoded.
pub fn strip_all_tags(html: Option<&str>) -> String {
    let Some(html) = html else { return String::new() };
    let clean = ammonia::Builder::empty().clean(html).to_string();
    decode_entities(&clean)
}

/// Decodes the HTML entities ammonia leaves in text (`&amp;`, `&lt;`, `&gt;`, `&quot;`, `&#39;`, `&nbsp;`).
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_originals_tags_and_drops_the_rest() {
        assert_eq!(sanitize(None), "");
        assert_eq!(sanitize(Some("")), "");
        assert_eq!(sanitize(Some("Hello <b>you</b>")), "Hello <b>you</b>");
        assert_eq!(sanitize(Some("<script>alert(1)</script><p>x</p>")), "<p>x</p>");
        assert_eq!(sanitize(Some("<span class=\"a\">text</span>")), "text");
        assert_eq!(sanitize(Some("<a href=\"https://x.org\" onclick=\"y\">l</a>")), "<a href=\"https://x.org\">l</a>");
        assert_eq!(sanitize(Some("<a href=\"javascript:alert(1)\">l</a>")), "<a>l</a>");
        assert_eq!(strip_all_tags(Some("<p>Tom &amp; Jerry</p>")), "Tom & Jerry");
    }
}
