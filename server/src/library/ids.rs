//! Book identifiers: ISBN-10 and ISBN-13 (checked by their check digit, turned into each other) and Audible
//! ASINs. Every identifier is kept in one clean form: ISBN-13 digits, ASIN in capitals.

/// The digits of an ISBN as typed (`978-0-593-13520-4`, `0-593-13520-X`): digits and a final `X` only.
fn clean(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_digit() || *c == 'x' || *c == 'X').map(|c| c.to_ascii_uppercase()).collect()
}

/// Whether `d` (10 characters, the last may be `X`) is a valid ISBN-10.
fn valid10(d: &str) -> bool {
    let b = d.as_bytes();
    if b.len() != 10 || !b[..9].iter().all(u8::is_ascii_digit) {
        return false;
    }
    let mut sum = 0u32;
    for (i, c) in b.iter().enumerate() {
        let v = match c {
            b'X' if i == 9 => 10,
            b'0'..=b'9' => u32::from(c - b'0'),
            _ => return false,
        };
        sum += v * (10 - i as u32);
    }
    sum.is_multiple_of(11)
}

/// Whether `d` (13 digits) is a valid ISBN-13 (978 or 979).
fn valid13(d: &str) -> bool {
    let b = d.as_bytes();
    if b.len() != 13 || !b.iter().all(u8::is_ascii_digit) || !(d.starts_with("978") || d.starts_with("979")) {
        return false;
    }
    let sum: u32 = b.iter().enumerate().map(|(i, c)| u32::from(c - b'0') * if i % 2 == 0 { 1 } else { 3 }).sum();
    sum.is_multiple_of(10)
}

/// The ISBN-13 of an ISBN-10 (`978` and a new check digit).
fn to13(d10: &str) -> String {
    let body = format!("978{}", &d10[..9]);
    let sum: u32 = body.bytes().enumerate().map(|(i, c)| u32::from(c - b'0') * if i % 2 == 0 { 1 } else { 3 }).sum();
    format!("{body}{}", (10 - sum % 10) % 10)
}

/// The ISBN-13 of an ISBN as typed (10 or 13 digits, with or without dashes and spaces); `None` when it is not a
/// valid ISBN.
pub fn isbn13(raw: &str) -> Option<String> {
    let d = clean(raw);
    if valid13(&d) {
        Some(d)
    } else if valid10(&d) {
        Some(to13(&d))
    } else {
        None
    }
}

/// The ISBN-10 of an ISBN-13 that has one (978…); `None` for 979… and invalid numbers.
pub fn isbn10(isbn13: &str) -> Option<String> {
    let d = clean(isbn13);
    if !valid13(&d) || !d.starts_with("978") {
        return None;
    }
    let body = &d[3..12];
    let sum: u32 = body.bytes().enumerate().map(|(i, c)| u32::from(c - b'0') * (10 - i as u32)).sum();
    let check = (11 - sum % 11) % 11;
    Some(format!("{body}{}", if check == 10 { "X".to_string() } else { check.to_string() }))
}

/// An Audible ASIN as typed, in capitals (`B08G9PRS1K`); `None` unless it is 10 letters and digits. Audible's
/// own ASINs start with `B0`; older audiobooks use ISBN-10-like numbers, which are ASINs too.
pub fn asin(raw: &str) -> Option<String> {
    let a = raw.trim().to_ascii_uppercase();
    (a.len() == 10 && a.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbns_are_checked_and_converted() {
        assert_eq!(isbn13("978-0-593-13520-4").as_deref(), Some("9780593135204"));
        assert_eq!(isbn13("0593135202").as_deref(), Some("9780593135204"));
        assert_eq!(isbn13("0-8044-2957-X").as_deref(), Some("9780804429573"));
        assert_eq!(isbn13("9780593135205"), None, "wrong check digit");
        assert_eq!(isbn13("12345"), None);
        assert_eq!(isbn10("9780593135204").as_deref(), Some("0593135202"));
        assert_eq!(isbn10("9780804429573").as_deref(), Some("080442957X"));
        assert_eq!(isbn10("9791032305690"), None, "979 has no ISBN-10");
        assert_eq!(isbn13("9791032305690").as_deref(), Some("9791032305690"));
    }

    #[test]
    fn asins_are_ten_letters_and_digits() {
        assert_eq!(asin(" b08g9prs1k ").as_deref(), Some("B08G9PRS1K"));
        assert_eq!(asin("B08G9PRS1"), None);
        assert_eq!(asin("B08G9-RS1K"), None);
    }
}
