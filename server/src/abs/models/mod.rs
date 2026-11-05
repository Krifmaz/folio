//! Audiobookshelf's models: one row type per table ([`rows`], generated from the original's schema) and the
//! small typed access every port module shares: find, list, insert, update and delete a row, and the date format
//! the original stores.
//!
//! Behaviour of a model (its JSON for the API, its rules) lives in a file of its own next to this one, owned by
//! the track that ports that part (docs/TRACKS.md).

pub mod book;
pub mod library_item;
pub mod media_progress;
pub mod rows;
pub mod user;

pub use rows::*;

use kit_server_core::{err, Db, Result, Row};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};

/// A table of the original's database and how its columns are stored.
pub trait Table: Serialize + DeserializeOwned + Default {
    /// The table's name, as in the original.
    const TABLE: &'static str;
    /// Every column, in the original's order.
    const COLUMNS: &'static [&'static str];
    /// The columns stored as JSON text.
    const JSON_COLUMNS: &'static [&'static str];
    /// The columns stored as 0 or 1.
    const BOOL_COLUMNS: &'static [&'static str];
}

/// A database row as a typed value: JSON columns parsed, 0/1 columns as booleans.
pub fn from_row<T: Table>(mut row: Row) -> Result<T> {
    for c in T::JSON_COLUMNS {
        if let Some(Value::String(s)) = row.get(*c) {
            let v = serde_json::from_str(s).unwrap_or(Value::Null);
            row.insert((*c).to_string(), v);
        }
    }
    for c in T::BOOL_COLUMNS {
        if let Some(n) = row.get(*c).and_then(Value::as_i64) {
            row.insert((*c).to_string(), Value::Bool(n != 0));
        }
    }
    serde_json::from_value(Value::Object(row)).map_err(|e| err(500, format!("A row of {} doesn't read as expected: {e}", T::TABLE)))
}

/// The values to store for each column of a typed row, in [`Table::COLUMNS`] order.
pub fn to_params<T: Table>(value: &T) -> Result<Vec<Value>> {
    let Value::Object(map) = serde_json::to_value(value).map_err(|e| err(500, e.to_string()))? else {
        return Err(err(500, "A row must be an object."));
    };
    Ok(T::COLUMNS.iter().map(|c| column_value::<T>(&map, c)).collect())
}

/// One column's stored value: JSON as text, booleans as 0/1, nothing as NULL.
fn column_value<T: Table>(map: &Map<String, Value>, column: &str) -> Value {
    match map.get(column) {
        None | Some(Value::Null) => Value::Null,
        Some(v) if T::JSON_COLUMNS.contains(&column) => Value::String(v.to_string()),
        Some(Value::Bool(b)) => Value::from(i64::from(*b)),
        Some(v) => v.clone(),
    }
}

/// The quoted column list and `?` placeholders of a table.
fn columns_sql<T: Table>() -> (String, String) {
    let cols = T::COLUMNS.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ");
    let marks = vec!["?"; T::COLUMNS.len()].join(", ");
    (cols, marks)
}

/// The row whose `column` equals `value`, if any.
pub fn find_by<T: Table>(db: &dyn Db, column: &str, value: impl Into<Value>) -> Result<Option<T>> {
    let sql = format!("SELECT * FROM `{}` WHERE `{column}` = ? LIMIT 1", T::TABLE);
    db.one(&sql, vec![value.into()])?.map(from_row).transpose()
}

/// The row with this `id`, if any.
pub fn find<T: Table>(db: &dyn Db, id: &str) -> Result<Option<T>> {
    find_by(db, "id", id)
}

/// Every row matching `filter` (SQL after `WHERE`, `1` for all), in the order given by `order` (SQL after
/// `ORDER BY`, or empty).
pub fn list<T: Table>(db: &dyn Db, filter: &str, order: &str, params: Vec<Value>) -> Result<Vec<T>> {
    let order = if order.is_empty() { String::new() } else { format!(" ORDER BY {order}") };
    let sql = format!("SELECT * FROM `{}` WHERE {filter}{order}", T::TABLE);
    db.query(&sql, params)?.into_iter().map(from_row).collect()
}

/// Stores a new row.
pub fn insert<T: Table>(db: &dyn Db, value: &T) -> Result<()> {
    let (cols, marks) = columns_sql::<T>();
    db.execute(&format!("INSERT INTO `{}` ({cols}) VALUES ({marks})", T::TABLE), to_params(value)?)?;
    Ok(())
}

/// Stores every column of an existing row, found by `key_column` (`id` for most tables).
pub fn update_by<T: Table>(db: &dyn Db, key_column: &str, value: &T) -> Result<()> {
    let params = to_params(value)?;
    let key = T::COLUMNS
        .iter()
        .position(|c| *c == key_column)
        .map(|i| params[i].clone())
        .ok_or_else(|| err(500, format!("{} has no column {key_column}.", T::TABLE)))?;
    let sets = T::COLUMNS.iter().map(|c| format!("`{c}` = ?")).collect::<Vec<_>>().join(", ");
    let mut all = params;
    all.push(key);
    db.execute(&format!("UPDATE `{}` SET {sets} WHERE `{key_column}` = ?", T::TABLE), all)?;
    Ok(())
}

/// Stores every column of an existing row, found by its `id`.
pub fn update<T: Table>(db: &dyn Db, value: &T) -> Result<()> {
    update_by(db, "id", value)
}

/// Removes the row with this `id`.
pub fn delete<T: Table>(db: &dyn Db, id: &str) -> Result<()> {
    db.execute(&format!("DELETE FROM `{}` WHERE `id` = ?", T::TABLE), vec![Value::from(id)])?;
    Ok(())
}

/// A new id in the original's form: a random UUID v4.
pub fn new_id() -> String {
    let b: [u8; 16] = rand::random();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]) & 0x0fff,
        (u16::from_be_bytes([b[8], b[9]]) & 0x3fff) | 0x8000,
        u64::from_be_bytes([0, 0, b[10], b[11], b[12], b[13], b[14], b[15]])
    )
}

/// Milliseconds since 1970, the unit the original's API uses for every time.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A time in milliseconds as the original stores it: `YYYY-MM-DD HH:MM:SS.SSS +00:00` (UTC).
pub fn date_text(ms: i64) -> String {
    let (days, rest) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s, milli) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}.{milli:03} +00:00")
}

/// Now, as the original stores it.
pub fn date_now() -> String {
    date_text(now_ms())
}

/// A stored time back in milliseconds since 1970: the original's `YYYY-MM-DD HH:MM:SS.SSS +HH:MM`, also with a
/// `T` or `Z`, without the fraction or without the zone. `None` when it isn't a date.
pub fn date_ms(text: &str) -> Option<i64> {
    let t = text.trim();
    let num = |a: usize, b: usize| t.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13).unwrap_or(0), num(14, 16).unwrap_or(0), num(17, 19).unwrap_or(0));
    let mut rest = t.get(19..).unwrap_or("");
    let mut milli = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        milli = format!("{:0<3}", &digits[..digits.len().min(3)]).parse().unwrap_or(0);
        rest = &frac[digits.len()..];
    }
    let rest = rest.trim();
    let offset = if rest.is_empty() || rest == "Z" {
        0
    } else {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let z = rest.trim_start_matches(['+', '-']).replace(':', "");
        let (zh, zm) = (z.get(0..2)?.parse::<i64>().ok()?, z.get(2..4).unwrap_or("0").parse::<i64>().ok()?);
        sign * (zh * 60 + zm) * 60_000
    };
    Some((days_from_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + s) * 1000 + milli - offset)
}

/// Days since 1970-01-01 of a calendar date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The calendar date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip_in_the_originals_format() {
        assert_eq!(date_text(0), "1970-01-01 00:00:00.000 +00:00");
        assert_eq!(date_text(1_793_901_840_437), "2026-11-05 18:04:00.437 +00:00");
        assert_eq!(date_ms("2026-11-05 18:04:00.437 +00:00"), Some(1_793_901_840_437));
        assert_eq!(date_ms("2026-11-05T19:04:00.437+01:00"), Some(1_793_901_840_437));
        assert_eq!(date_ms("2026-11-05T18:04:00Z"), Some(1_793_901_840_000));
        assert_eq!(date_ms("2024-02-29"), Some(1_709_164_800_000));
        assert_eq!(date_ms("not a date"), None);
    }

    #[test]
    fn rows_round_trip_with_json_and_bool_columns() {
        let db = kit_server_core::SqliteDb::memory();
        crate::abs::create(&db).unwrap();
        let mut u = User {
            id: Some(new_id()),
            username: Some("root".into()),
            kind: Some("root".into()),
            is_active: Some(true),
            permissions: Some(serde_json::json!({"download": true, "librariesAccessible": []})),
            created_at: Some(date_now()),
            updated_at: Some(date_now()),
            ..User::default()
        };
        insert(&db, &u).unwrap();
        let raw = db.one("SELECT isActive, permissions, type FROM users", vec![]).unwrap().unwrap();
        assert_eq!(raw["isActive"], serde_json::json!(1));
        assert_eq!(raw["type"], serde_json::json!("root"));
        assert!(raw["permissions"].is_string(), "JSON columns are stored as text like the original's");
        let back: User = find(&db, u.id.as_deref().unwrap()).unwrap().unwrap();
        assert_eq!(back, u);
        u.is_locked = Some(false);
        u.username = Some("admin".into());
        update(&db, &u).unwrap();
        let all: Vec<User> = list(&db, "`username` = ?", "", vec!["admin".into()]).unwrap();
        assert_eq!(all, vec![u.clone()]);
        delete::<User>(&db, u.id.as_deref().unwrap()).unwrap();
        assert!(find::<User>(&db, u.id.as_deref().unwrap()).unwrap().is_none());
    }

    #[test]
    fn ids_are_uuid_v4() {
        let id = new_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }
}
