//! The port of the original's file name helpers (`server/utils/fileUtils.js`) that the admin pages need: making
//! a safe file name from one a person gave, and turning a typed folder into a full path like Node's
//! `Path.resolve`.

use std::path::Path;
use unicode_normalization::UnicodeNormalization;

/// A file name with the characters file systems refuse taken out (`sanitizeFilename`): the first `:` becomes
/// ` - `, `/ ? < > \ : * | "` and control characters go, names made only of dots or reserved on Windows become
/// empty, trailing dots and spaces go, runs of spaces become one, and the name is cut to 255 bytes (UTF-16).
pub fn sanitize_filename(filename: &str) -> String {
    let nfc: String = filename.nfc().collect();
    let replaced = nfc.replacen(':', " - ", 1);
    let mut s: String =
        replaced.chars().filter(|c| !matches!(c, '/' | '?' | '<' | '>' | '\\' | ':' | '*' | '|' | '"' | '\n' | '\r')).collect();
    s.retain(|c| !matches!(c as u32, 0x00..=0x1f | 0x80..=0x9f));
    if !s.is_empty() && s.chars().all(|c| c == '.') {
        s.clear();
    }
    if windows_reserved(&s) {
        s.clear();
    }
    let s = s.trim_end_matches(['.', ' ']);
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    let ext = extname(&out).to_string();
    let base = &out[..out.len() - ext.len()];
    let utf16 = |t: &str| t.encode_utf16().count() * 2;
    if utf16(base) + utf16(&ext) > 255 {
        let max = 255usize.saturating_sub(utf16(&ext));
        let mut total = 0;
        let mut trimmed = String::new();
        for c in base.chars() {
            total += c.len_utf16() * 2;
            if total > max {
                break;
            }
            trimmed.push(c);
        }
        return format!("{}{ext}", trimmed.trim());
    }
    out
}

/// Whether a name is one Windows reserves (`con`, `prn`, `aux`, `nul`, `com0`–`com9`, `lpt0`–`lpt9`, with any
/// extension).
fn windows_reserved(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or_default();
    let numbered = |p: &str| stem.strip_prefix(p).is_some_and(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit());
    matches!(stem, "con" | "prn" | "aux" | "nul") || numbered("com") || numbered("lpt")
}

/// A name's extension with its dot, like Node's `Path.extname` (empty for none or for a name that only starts
/// with a dot).
pub fn extname(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 && base[..i].chars().any(|c| c != '.') => &base[i..],
        _ => "",
    }
}

/// A folder as a full path with forward slashes (`filePathToPOSIX(Path.resolve(p))`): relative to the working
/// folder, with `.`, `..`, repeated and trailing slashes worked out.
pub fn resolve_posix(p: &str) -> String {
    let joined = if p.starts_with('/') {
        p.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new("/").to_path_buf());
        format!("{}/{p}", cwd.display())
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe_like_the_original() {
        assert_eq!(sanitize_filename("2026-11-05T1800.audiobookshelf"), "2026-11-05T1800.audiobookshelf");
        assert_eq!(sanitize_filename("a:b:c?.audiobookshelf"), "a - bc.audiobookshelf");
        assert_eq!(sanitize_filename("../../etc/passwd"), "....etcpasswd");
        assert_eq!(sanitize_filename("name.  "), "name");
        assert_eq!(sanitize_filename("con.txt"), "");
        assert_eq!(sanitize_filename("a   b"), "a b");
        let long = format!("{}.zip", "x".repeat(200));
        assert_eq!(sanitize_filename(&long).len(), 123 + 4);
    }

    #[test]
    fn paths_resolve_like_node() {
        assert_eq!(resolve_posix("/data/metadata/backups/"), "/data/metadata/backups");
        assert_eq!(resolve_posix("/data//x/../y/./z"), "/data/y/z");
        assert_eq!(resolve_posix("/"), "/");
        assert_eq!(extname("a.audiobookshelf"), ".audiobookshelf");
        assert_eq!(extname(".hidden"), "");
        assert_eq!(extname("noext"), "");
    }
}
