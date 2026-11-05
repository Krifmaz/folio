//! Test helpers: the recorded answers of the original and the comparison of tests/parity, for unit tests.

#[path = "../../tests/parity/mod.rs"]
pub mod parity;

/// The body of a recorded answer.
pub fn fixture(name: &str) -> serde_json::Value {
    parity::fixture("abs", name).body
}

/// A fresh comparison.
pub fn matcher() -> parity::Matcher {
    parity::Matcher::default()
}
