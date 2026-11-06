//! The Audiobookshelf port's logs (PARITY P6): today's messages for the log page, who may read them, and the
//! log settings. Replays the recorded requests and compares the answers with the pinned original's; the messages
//! themselves are Folio's, so they are compared by shape.

mod abs_replay;
mod common;
mod parity;

use abs_replay::set_up;
use serde_json::Value;

/// The keys of one message, sorted.
fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[tokio::test]
async fn logs_answer_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-logs").await;
    for name in ["p6l_guest_create", "p6l_guest_login"] {
        r.like(name).await;
    }
    st.abs_server.logs.log(server::abs::logs::level::WARN, "test.rs:1", "A message for the log page");
    let a = r.send("logger_data").await;
    let fx = parity::fixture("abs", "logger_data");
    assert_eq!((a.status, a.content_type.as_str()), (fx.status, fx.content_type.as_str()));
    let logs = a.body["currentDailyLogs"].as_array().unwrap();
    let last = logs.last().unwrap();
    assert_eq!(last["message"], "A message for the log page");
    assert_eq!(last["levelName"], "WARN");
    assert_eq!(keys(last), keys(&fx.body["currentDailyLogs"][0]));
    r.like("logger_data_guest").await;

    r.like("settings_log_level").await;
    assert_eq!(st.abs_server.logs.level(), server::abs::logs::level::DEBUG);
    st.abs_server.logs.log(server::abs::logs::level::DEBUG, "test.rs:2", "Now kept");
    r.like("settings_log_level_reset").await;
    assert_eq!(st.abs_server.logs.level(), server::abs::logs::level::INFO);
    st.abs_server.logs.log(server::abs::logs::level::DEBUG, "test.rs:3", "Not kept any more");
    let a = r.send("logger_data").await;
    let messages: Vec<&str> = a.body["currentDailyLogs"].as_array().unwrap().iter().filter_map(|l| l["message"].as_str()).collect();
    assert!(messages.contains(&"Now kept"));
    assert!(!messages.contains(&"Not kept any more"));
    r.like("p6l_guest_delete").await;
}
