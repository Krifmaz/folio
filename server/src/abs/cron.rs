//! The port of the original's schedule format (`server/libs/nodeCron`, node-cron 3): reading a cron expression
//! into the minutes, hours and days it runs, and checking one the way the original does
//! (`POST /api/validate-cron`, backup and podcast schedules). Five fields (minute hour day month weekday) or six
//! (with seconds first); numbers, `*`, ranges `1-5`, lists `1,2`, steps `*/10` and English month and weekday
//! names. Odd inputs are judged exactly like the original, quirks included (a trailing `5x` reads as `5`).

/// Whether the original accepts this schedule (`cron.validate`).
pub fn validate(expression: &str) -> bool {
    let Some(fields) = convert(expression) else { return false };
    const LIMITS: [(i64, i64); 6] = [(0, 59), (0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    LIMITS.iter().zip(&fields).all(|((min, max), field)| valid_field(field, *min, *max))
}

/// One field after conversion (`isValidExpression`): every listed value a number within the limits.
fn valid_field(field: &str, min: i64, max: i64) -> bool {
    field.split(',').all(|option| {
        let in_range = parse_int(option).is_none_or(|n| n >= min && n <= max);
        let digits = !option.is_empty() && option.chars().all(|c| c.is_ascii_digit());
        in_range
            && (digits
                || option == "*"
                || option.strip_prefix("*/").is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())))
    })
}

/// The expression with every field turned into a list of numbers (`convertExpression`), or `None` where the
/// original throws. Fields after the sixth are kept as they are, only checked for a bad step.
pub fn convert(expression: &str) -> Option<Vec<String>> {
    let collapsed = collapse_spaces(expression);
    let mut fields: Vec<String> = collapsed.split(' ').map(str::to_string).collect();
    if fields.len() == 5 {
        fields.insert(0, "0".into());
    }
    if fields.len() < 6 {
        return None;
    }
    fields[4] = month_names(&fields[4]);
    fields[5] = weekday_names(&fields[5]);
    for (i, range) in ["0-59", "0-59", "0-23", "1-31", "1-12", "0-6"].iter().enumerate() {
        fields[i] = fields[i].replacen('*', range, 1);
    }
    for (i, field) in fields.iter_mut().enumerate() {
        if i < 6 {
            *field = expand_ranges(field)?;
        }
        *field = steps(field)?;
        if i < 6 {
            *field =
                field.split(',').map(|n| parse_int(n).map(|v| v.to_string()).unwrap_or_else(|| "NaN".into())).collect::<Vec<_>>().join(",");
        }
    }
    Some(fields)
}

/// Runs of two or more whitespace characters become one space; the ends are trimmed (`removeSpaces`).
fn collapse_spaces(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    for c in s.chars() {
        if c.is_whitespace() {
            run.push(c);
        } else {
            if !run.is_empty() {
                out.push_str(if run.chars().count() >= 2 { " " } else { &run });
                run.clear();
            }
            out.push(c);
        }
    }
    out.push_str(if run.chars().count() >= 2 { " " } else { &run });
    out.trim().to_string()
}

/// Replaces every case-insensitive occurrence of `name` with `with`.
fn replace_ci(s: &str, name: &str, with: &str) -> String {
    let lower = s.to_lowercase();
    if lower.len() != s.len() {
        return s.to_string();
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(name) {
        out.push_str(&s[i..i + pos]);
        out.push_str(with);
        i += pos + name.len();
    }
    out.push_str(&s[i..]);
    out
}

/// Month names to numbers, full names first (`monthNamesConversion`).
fn month_names(field: &str) -> String {
    const FULL: [&str; 12] =
        ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
    const SHORT: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let mut f = field.to_string();
    for names in [FULL, SHORT] {
        for (i, n) in names.iter().enumerate() {
            f = replace_ci(&f, n, &(i + 1).to_string());
        }
    }
    f
}

/// Weekday names to numbers, Sunday 0; the first `7` becomes `0` (`weekDayNamesConversion`).
fn weekday_names(field: &str) -> String {
    const FULL: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];
    const SHORT: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    let mut f = field.replacen('7', "0", 1);
    for names in [FULL, SHORT] {
        for (i, n) in names.iter().enumerate() {
            f = replace_ci(&f, n, &i.to_string());
        }
    }
    f
}

/// The first `a-b` in a field: its start, the two numbers' text, and its length.
fn first_range(s: &str) -> Option<(usize, usize, String, String)> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'-' && b[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                return Some((start, j, s[start..i].to_string(), s[i + 1..j].to_string()));
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Every `a-b` written out as `a,a+1,…,b` (either way round) (`convertRanges`). `None` for a range too long to
/// ever be valid in the six checked fields (the original would spend its memory writing it out).
fn expand_ranges(field: &str) -> Option<String> {
    let mut f = field.to_string();
    while let Some((_, _, a, b)) = first_range(&f) {
        let (x, y) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
        let (lo, hi) = (x.min(y), x.max(y));
        if hi - lo > 10_000 {
            return None;
        }
        let list = (lo..=hi).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        // the original replaces the first occurrence of the matched text, which is this one
        f = f.replacen(&format!("{a}-{b}"), &list, 1);
    }
    Some(f)
}

/// A step `values/n` keeps the values divisible by `n` (`convertSteps`); `None` when `n` isn't a number.
fn steps(field: &str) -> Option<String> {
    let Some((values, divider)) = field.rsplit_once('/') else { return Some(field.to_string()) };
    if values.is_empty() || divider.is_empty() || !divider.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Some(field.to_string());
    }
    // the original throws when the step isn't a number at all
    js_number(divider)?;
    let d = parse_int(divider);
    let kept: Vec<String> = values
        .split(',')
        .filter_map(parse_int)
        .filter(|v| d.is_some_and(|d| d != 0 && v % d == 0))
        .map(|v| if v == 0 { "0".to_string() } else { v.to_string() })
        .collect();
    Some(kept.join(","))
}

/// JavaScript's `parseInt(s, 10)`: leading spaces, an optional sign and the leading digits; `None` for NaN.
fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim_start();
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let n: i64 = digits.parse().unwrap_or(i64::MAX);
    Some(if neg { -n } else { n })
}

/// Whether JavaScript's `Number(s)` is a number (`!isNaN(s)`) for a word of letters, digits and `_`.
fn js_number(s: &str) -> Option<f64> {
    let lower = s.to_lowercase();
    for (prefix, radix) in [("0x", 16), ("0b", 2), ("0o", 8)] {
        if let Some(d) = lower.strip_prefix(prefix) {
            return u64::from_str_radix(d, radix).ok().map(|v| v as f64);
        }
    }
    if s == "Infinity" {
        return Some(f64::INFINITY);
    }
    if !s.chars().all(|c| c.is_ascii_digit() || c == 'e' || c == 'E') {
        return None;
    }
    s.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_schedules_the_original_accepts() {
        for ok in [
            "30 1 * * *",
            "0 */6 * jan-mar mon-fri",
            "*/10 * * * * *",
            "0 * * * *",
            "0 0 1 1 7",
            "0  0   * *  *",
            "5x * * * *",
            "0 0 * * * * 1/2",
            "0 0 1-31/2 * *",
            "0 0 * * Sunday",
        ] {
            assert!(validate(ok), "{ok}");
        }
    }

    #[test]
    fn refuses_the_schedules_the_original_refuses() {
        for bad in [
            "",
            "61 * * * *",
            "* * *",
            "*/x * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "a b c d e",
            "0 0 * * 8",
            "0 0 * * * * 1/x",
            "*,* * * * *",
        ] {
            assert!(!validate(bad), "{bad}");
        }
    }
}
