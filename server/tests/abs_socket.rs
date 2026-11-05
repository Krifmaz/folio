//! The Audiobookshelf port's socket.io (PARITY: the socket rows): the apps connect on `/socket.io` and sign in
//! with a token (`init`, `user_online`); a token that isn't valid is answered with the original's `auth_failed`.

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

#[tokio::test]
async fn a_valid_token_signs_the_socket_in_like_the_original() {
    let (app, _st, _dir) = common::server("abs-socket-auth", HashMap::new()).await;
    let base = common::serve(app).await;
    let http = reqwest::Client::new();
    let init = http
        .post(format!("{base}/init"))
        .json(&serde_json::json!({"newRoot": {"username": "root", "password": "root"}}))
        .send()
        .await
        .unwrap();
    assert!(init.status().is_success());
    let login: serde_json::Value = http
        .post(format!("{base}/login"))
        .header("x-return-tokens", "true")
        .json(&serde_json::json!({"username": "root", "password": "root"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let user_id = login["user"]["id"].as_str().unwrap().to_string();
    let open = poll(&http, &format!("{base}/socket.io/?EIO=4&transport=polling"), None).await;
    let sid = serde_json::from_str::<serde_json::Value>(&open[1..]).unwrap()["sid"].as_str().unwrap().to_string();
    let url = format!("{base}/socket.io/?EIO=4&transport=polling&sid={sid}");
    poll(&http, &url, Some("40")).await;
    poll(&http, &url, None).await;
    let refresh = login["user"]["refreshToken"].as_str().unwrap();
    poll(&http, &url, Some(&format!(r#"42["auth","{refresh}"]"#))).await;
    assert_eq!(
        packets(&poll(&http, &url, None).await),
        vec![r#"2["auth_failed",{"message":"Invalid token"}]"#],
        "refresh tokens don't sign sockets in"
    );
    let access = login["user"]["accessToken"].as_str().unwrap();
    poll(&http, &url, Some(&format!(r#"42["auth","{access}"]"#))).await;
    let mut events: Vec<serde_json::Value> = Vec::new();
    for _ in 0..3 {
        events.extend(packets(&poll(&http, &url, None).await).iter().filter_map(|p| serde_json::from_str(&p[1..]).ok()));
        if events.iter().any(|e| e[0] == "init") {
            break;
        }
    }
    let online = events.iter().find(|e| e[0] == "user_online").expect("admins are told the user came online");
    assert_eq!(online[1]["id"], serde_json::json!(user_id));
    assert_eq!(online[1]["lastSeen"], serde_json::Value::Null, "the original sends the user before saving lastSeen");
    let init = events.iter().find(|e| e[0] == "init").expect("init");
    assert_eq!(init[1]["userId"], serde_json::json!(user_id));
    assert_eq!(init[1]["username"], "root");
    let users = init[1]["usersOnline"].as_array().expect("admins get who is online");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["connections"], 1);
    for k in ["id", "username", "type", "session", "lastSeen", "createdAt", "connections"] {
        assert!(users[0].get(k).is_some(), "{k} missing in {}", users[0]);
    }
}
