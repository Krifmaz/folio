//! Readarr's naming (`NamingConfig` and `FileNameBuilder`): the settings for file and folder names, and the
//! author folder they give (`GetAuthorFolder`). The book file names come with the rename row of R1.
//!
//! Patterns are made of tokens like `{Author Name}`: a token may have text before and after it inside the
//! braces (`{ - Author Name}`), and the way it is written decides the case (`{author name}` gives lower case,
//! `{AUTHOR NAME}` upper case) and the separator (`{Author.Name}` puts dots for spaces).

use super::text::{clean_title, title_the};
use kit_server_core::{Db, Row, RowExt, SqliteDb};
use regex::{Captures, Regex};
use serde_json::{json, Value};
use std::sync::LazyLock;

/// A token in a pattern (`FileNameBuilder.TitleRegex`).
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\{(?<prefix>[- ._\[(]*)(?<token>(?:[a-z0-9]+)(?:(?<separator>[- ._]+)(?:[a-z0-9]+))?)(?::(?<customFormat>[a-z0-9]+))?(?<suffix>[- ._)\]]*)\}")
        .expect("a valid pattern")
});

/// How colons in names are replaced (`ColonReplacementFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colons {
    /// removed
    Delete,
    /// "-"
    Dash,
    /// " -"
    SpaceDash,
    /// " - "
    SpaceDashSpace,
    /// ": " becomes " - " and other colons "-" (the default)
    Smart,
}

/// Readarr's naming settings (the one `NamingConfig` row).
#[derive(Debug, Clone)]
pub struct Naming {
    /// whether characters a file name can't have are replaced (otherwise they are removed)
    pub replace_illegal_characters: bool,
    /// how colons are replaced
    pub colons: Colons,
    /// the pattern of an author's folder
    pub author_folder_format: String,
}

/// The default book file pattern Readarr stores with the default settings.
const STANDARD_BOOK_FORMAT: &str = "{Book Title}/{Author Name} - {Book Title}{ (PartNumber)}";

/// The naming settings; like Readarr, the defaults are stored the first time they are asked for.
pub fn config(db: &SqliteDb) -> kit_server_core::Result<Naming> {
    let row = match db.one("SELECT * FROM NamingConfig ORDER BY Id LIMIT 1", vec![])? {
        Some(r) => r,
        None => {
            db.execute(
                "INSERT INTO NamingConfig (ReplaceIllegalCharacters, AuthorFolderFormat, RenameBooks, StandardBookFormat, ColonReplacementFormat)
                 VALUES (1, '{Author Name}', 0, ?, 4)",
                vec![json!(STANDARD_BOOK_FORMAT)],
            )?;
            db.one("SELECT * FROM NamingConfig ORDER BY Id LIMIT 1", vec![])?.unwrap_or_default()
        }
    };
    Ok(Naming {
        replace_illegal_characters: row.i("ReplaceIllegalCharacters") != 0,
        colons: match row.i("ColonReplacementFormat") {
            0 => Colons::Delete,
            1 => Colons::Dash,
            2 => Colons::SpaceDash,
            3 => Colons::SpaceDashSpace,
            _ => Colons::Smart,
        },
        author_folder_format: row.s("AuthorFolderFormat"),
    })
}

/// A name without the characters a file name can't have (`CleanFileName`).
pub fn clean_file_name(name: &str, n: &Naming) -> String {
    let mut result = name.to_string();
    if n.replace_illegal_characters {
        result = match n.colons {
            Colons::Smart => result.replace(": ", " - ").replace(':', "-"),
            Colons::Delete => result.replace(':', ""),
            Colons::Dash => result.replace(':', "-"),
            Colons::SpaceDash => result.replace(':', " -"),
            Colons::SpaceDashSpace => result.replace(':', " - "),
        };
    } else {
        result = result.replace(':', "");
    }
    let bad = ["\\", "/", "<", ">", "?", "*", "|", "\""];
    let good = ["+", "+", "", "", "!", "-", "", ""];
    for (b, g) in bad.iter().zip(good) {
        result = result.replace(b, if n.replace_illegal_characters { g } else { "" });
    }
    result.trim_start_matches([' ', '.']).trim_end_matches(' ').to_string()
}

/// A folder name with runs of the same separator (`-`, space, `.`, `_`) made one, and without spaces and dots at
/// its ends (`CleanFolderName`).
pub fn clean_folder_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if prev == Some(c) && matches!(c, '-' | ' ' | '.' | '_') {
            continue;
        }
        out.push(c);
        prev = Some(c);
    }
    out.trim_matches([' ', '.']).to_string()
}

/// A token's name as Readarr compares them: without spaces, underscores and punctuation, in lower case.
fn simplify(token: &str) -> String {
    token.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// What the author tokens stand for (`AddAuthorTokens`), by simplified token name.
fn author_token(author: &Row, token: &str) -> Option<String> {
    let name = author.s("Name");
    Some(match simplify(token).as_str() {
        "authorname" => name,
        "authorcleanname" => clean_title(&name),
        "authornamethe" => title_the(&name),
        "authorsortname" => author.get("NameLastFirst").and_then(Value::as_str).unwrap_or_default().to_string(),
        "authornamefirstcharacter" => title_the(&name).chars().next().map(|f| f.to_uppercase().to_string()).unwrap_or_default(),
        "authordisambiguation" => author.get("Disambiguation").and_then(Value::as_str)?.to_string(),
        _ => return None,
    })
}

/// One token replaced (`ReplaceToken`): an unknown token gives nothing.
fn replace_token(caps: &Captures, author: &Row, n: &Naming) -> String {
    let token = &caps["token"];
    let separator = caps.name("separator").map(|m| m.as_str()).unwrap_or_default();
    let mut text = author_token(author, token).unwrap_or_default().trim().to_string();
    if token.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) {
        text = text.to_lowercase();
    } else if token.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) {
        text = text.to_uppercase();
    }
    if !separator.trim().is_empty() {
        text = text.replace(' ', separator);
    }
    text = clean_file_name(&text, n);
    if !text.trim().is_empty() {
        text = format!("{}{text}{}", &caps["prefix"], &caps["suffix"]);
    }
    text
}

/// The folder of an author (`GetAuthorFolder`): the author folder pattern with its tokens replaced, part by part.
/// `author` has the author's metadata columns (`Name`, `NameLastFirst`, `Disambiguation`).
pub fn author_folder(author: &Row, n: &Naming) -> String {
    n.author_folder_format
        .split(['\\', '/'])
        .filter(|p| !p.is_empty())
        .map(|p| clean_folder_name(&TOKEN.replace_all(p, |c: &Captures| replace_token(c, author, n))))
        .filter(|c| !c.trim().is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_folders_are_named_like_readarr() {
        let db = SqliteDb::memory();
        super::super::create(&db).unwrap();
        let n = config(&db).unwrap();
        assert_eq!(n.author_folder_format, "{Author Name}");
        assert_eq!(db.query("SELECT * FROM NamingConfig", vec![]).unwrap().len(), 1);
        config(&db).unwrap();
        assert_eq!(db.query("SELECT * FROM NamingConfig", vec![]).unwrap().len(), 1);

        let mut author = Row::new();
        author.insert("Name".into(), json!("The Author: Who?"));
        author.insert("NameLastFirst".into(), json!("Author, The"));
        assert_eq!(author_folder(&author, &n), "The Author - Who!");
        let with = |f: &str| Naming { author_folder_format: f.into(), ..n.clone() };
        assert_eq!(author_folder(&author, &with("{author name}")), "the author - who!");
        assert_eq!(author_folder(&author, &with("{Author.Name}")), "The.Author-.Who!");
        assert_eq!(author_folder(&author, &with("{Author NameThe}/{Author NameFirstCharacter}")), "Author - Who!, The/A");
        assert_eq!(author_folder(&author, &with("{Author SortName}{ (Author Disambiguation)}")), "Author, The");
        assert_eq!(clean_folder_name("a  b..c__"), "a b.c_");
    }
}
