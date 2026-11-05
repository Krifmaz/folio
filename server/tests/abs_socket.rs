//! The Audiobookshelf port's socket.io (PARITY: the socket rows): the apps connect on `/socket.io`, and a token
//! that isn't valid is answered with the original's `auth_failed`.

mod common;

use std::collections::HashMap;

/// One engine.io long-polling request; the packets the server sent.
async fn poll(http: &reqwest::Client, url: &str, send: Option<&str>) -> String {
    let r = match send {
        Some(body) => http.post(url).body(body.to_string()).send().await.unwrap(),
        None => http.get(url).send().await.unwrap(),
    };
    assert!(r.status().is_success(), "{} {}", r.status(), url);
    r.text().await.unwrap()
}

/// The socket.io packets of a poll (engine.io message packets, `4…`), without the engine's pings.
fn packets(text: &str) -> Vec<String> {
    text.split('\u{1e}').filter_map(|p| p.strip_prefix('4')).map(str::to_string).collect()
}

#[tokio::test]
async fn apps_connect_and_a_bad_token_gets_auth_failed() {
    let (app, _st, _dir) = common::server("abs-socket", HashMap::new()).await;
    let base = common::serve(app).await;
    let http = reqwest::Client::new();
    let open = poll(&http, &format!("{base}/socket.io/?EIO=4&transport=polling"), None).await;
    assert!(open.starts_with('0'), "{open}");
    let sid = serde_json::from_str::<serde_json::Value>(&open[1..]).unwrap()["sid"].as_str().unwrap().to_string();
    let url = format!("{base}/socket.io/?EIO=4&transport=polling&sid={sid}");
    poll(&http, &url, Some("40")).await;
    let connected = packets(&poll(&http, &url, None).await);
    assert!(connected.iter().any(|p| p.starts_with('0')), "{connected:?}");
    poll(&http, &url, Some(r#"42["auth","not-a-token"]"#)).await;
    let answer = packets(&poll(&http, &url, None).await);
    assert_eq!(answer, vec![r#"2["auth_failed",{"message":"Invalid token"}]"#]);
    poll(&http, &url, Some(r#"42["ping"]"#)).await;
    assert_eq!(packets(&poll(&http, &url, None).await), vec![r#"2["pong"]"#]);
}
