//! Reading XML into the shape the original's `xml2js` (default options) gives: every child element is a list
//! under its name as written ("dc:title"), attributes are under "$", text is under "_" (or the element is just
//! its text), and whitespace-only text is dropped.

use serde_json::{Map, Value};

/// One element being read: its name, attributes and the text and children met so far.
struct Open {
    /// The name as written, prefix included.
    name: String,
    /// The element's value so far (attributes under "$", children under their names).
    obj: Map<String, Value>,
    /// The text met directly inside it.
    text: String,
    /// Whether any of the text came from a CDATA section.
    cdata: bool,
}

/// Parses XML like `xml2js.parseString`; `None` when it isn't well-formed.
pub fn to_json(xml: &str) -> Option<Value> {
    let mut stack: Vec<Open> = Vec::new();
    let mut root: Option<(String, Value)> = None;
    let s = xml.trim_start_matches('\u{feff}');
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            if s[i..].starts_with("<!--") {
                i += s[i..].find("-->")? + 3;
            } else if s[i..].starts_with("<![CDATA[") {
                let end = s[i..].find("]]>")?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&s[i + 9..i + end]);
                    top.cdata = true;
                }
                i += end + 3;
            } else if s[i..].starts_with("<?") {
                i += s[i..].find("?>")? + 2;
            } else if s[i..].starts_with("<!") {
                // A doctype, possibly with an internal subset in brackets.
                let mut depth = 0;
                let mut j = i;
                while j < b.len() {
                    match b[j] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b'>' if depth <= 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                i = j + 1;
            } else if s[i..].starts_with("</") {
                let end = s[i..].find('>')?;
                let name = s[i + 2..i + end].trim();
                let open = stack.pop()?;
                if open.name != name {
                    return None;
                }
                let value = close(open);
                match stack.last_mut() {
                    Some(parent) => push_child(&mut parent.obj, name, value),
                    None => root = Some((name.to_string(), value)),
                }
                i += end + 1;
            } else {
                let (name, attrs, self_closing, len) = read_tag(&s[i..])?;
                let mut obj = Map::new();
                if !attrs.is_empty() {
                    obj.insert("$".into(), Value::Object(attrs));
                }
                let open = Open { name: name.clone(), obj, text: String::new(), cdata: false };
                i += len;
                if self_closing {
                    let value = close(open);
                    match stack.last_mut() {
                        Some(parent) => push_child(&mut parent.obj, &name, value),
                        None => root = Some((name, value)),
                    }
                } else {
                    stack.push(open);
                }
            }
        } else {
            let end = s[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&decode(&s[i..end]));
            }
            i = end;
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let (name, value) = root?;
    let mut out = Map::new();
    out.insert(name, value);
    Some(Value::Object(out))
}

/// Finishes an element (`onclosetag`): whitespace-only text is dropped, an element with nothing becomes its
/// whitespace (or ""), and one with only text becomes the text.
fn close(open: Open) -> Value {
    let mut obj = open.obj;
    let mut empty_str = String::new();
    if !open.text.is_empty() || open.cdata {
        if open.text.trim().is_empty() && !open.cdata {
            empty_str = open.text;
        } else {
            obj.insert("_".into(), Value::String(open.text));
        }
    }
    if obj.is_empty() {
        return Value::String(empty_str);
    }
    if obj.len() == 1 {
        if let Some(Value::String(t)) = obj.get("_") {
            return Value::String(t.clone());
        }
    }
    Value::Object(obj)
}

/// Adds a child element under its name in its parent.
fn push_child(parent: &mut Map<String, Value>, name: &str, value: Value) {
    match parent.get_mut(name) {
        Some(Value::Array(list)) => list.push(value),
        _ => {
            parent.insert(name.to_string(), Value::Array(vec![value]));
        }
    }
}

/// Reads an opening tag: its name, attributes, whether it closes itself and its length.
fn read_tag(s: &str) -> Option<(String, Map<String, Value>, bool, usize)> {
    let b = s.as_bytes();
    let mut i = 1;
    let start = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    let name = s[start..i].to_string();
    let mut attrs = Map::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            return None;
        }
        if b[i] == b'>' {
            return Some((name, attrs, false, i + 1));
        }
        if b[i] == b'/' && b.get(i + 1) == Some(&b'>') {
            return Some((name, attrs, true, i + 2));
        }
        let ks = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() && b[i] != b'>' && b[i] != b'/' {
            i += 1;
        }
        let key = s[ks..i].to_string();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            attrs.insert(key, Value::String(String::new()));
            continue;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *b.get(i)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let vs = i + 1;
        let ve = vs + s[vs..].find(quote as char)?;
        attrs.insert(key, Value::String(decode(&s[vs..ve])));
        i = ve + 1;
    }
}

/// Replaces XML's character references and the five named ones.
pub fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let Some(end) = rest.find(';').filter(|e| *e < 12) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shapes_like_xml2js() {
        let v = to_json(r#"<?xml version="1.0"?><package xmlns="x"><metadata><dc:title>A &amp; B</dc:title><dc:creator opf:role="aut">Ann</dc:creator><meta name="cover" content="c"/><empty/>
        </metadata></package>"#)
        .unwrap();
        assert_eq!(
            v,
            json!({"package": {"$": {"xmlns": "x"}, "metadata": [{
                "dc:title": ["A & B"],
                "dc:creator": [{"_": "Ann", "$": {"opf:role": "aut"}}],
                "meta": [{"$": {"name": "cover", "content": "c"}}],
                "empty": [""],
            }]}})
        );
        assert!(to_json("<a><b></a>").is_none());
    }
}
