//! Writing XML by hand for the Atom feeds: text made safe, elements and links. The feeds are small and flat, so
//! a string builder is all they need.

/// `s` made safe inside XML text and attribute values.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // characters XML 1.0 does not allow at all are left out
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// An XML document being written.
#[derive(Debug, Default)]
pub struct Xml {
    /// the text so far
    out: String,
}

impl Xml {
    /// Adds `s` as it is (already XML).
    pub fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// Adds `<name>text</name>` with the text made safe.
    pub fn el(&mut self, name: &str, text: &str) {
        self.out.push_str(&format!("<{name}>{}</{name}>\n", esc(text)));
    }

    /// Adds an Atom `<link>` with its relation, address, media type and, when not empty, title.
    pub fn link(&mut self, rel: &str, href: &str, kind: &str, title: &str) {
        let title = if title.is_empty() { String::new() } else { format!(" title=\"{}\"", esc(title)) };
        self.out.push_str(&format!("<link rel=\"{}\" href=\"{}\" type=\"{}\"{title}/>\n", esc(rel), esc(href), esc(kind)));
    }

    /// The finished document.
    pub fn done(self) -> String {
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_made_safe() {
        assert_eq!(esc("Tom & \"Jerry\" <1>\u{1}"), "Tom &amp; &quot;Jerry&quot; &lt;1&gt;");
    }
}
