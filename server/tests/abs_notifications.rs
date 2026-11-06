//! The Audiobookshelf port's notifications (PARITY P6): the Apprise settings, adding, changing and removing
//! notifications, the test event with its failure count and queue, and the per-notification test. Replays the
//! recorded requests in the recorded order and compares every answer with the pinned original's.

mod abs_replay;
mod common;
mod parity;

use abs_replay::set_up;
use serde_json::json;

#[tokio::test]
async fn notifications_are_set_up_fired_and_removed_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-notifications").await;
    r.m.volatile.push("lastFiredAt".into());
    for name in
        ["p6n_guest_create", "p6n_guest_login", "notifications", "notification_data", "notifications_guest", "notification_data_guest"]
    {
        r.like(name).await;
    }
    for name in [
        "notifications_create",
        "notifications_create_no_event",
        "notifications_create_no_urls",
        "notification_test_unconfigured",
        "notifications_update",
        "notifications_after_update",
        "notifications_fire_test_fail",
        "notifications_after_fail",
        // a second event within the delay waits in the queue, so nothing changes yet
        "notifications_fire_test_fail2",
        "notifications_after_fail2",
        "notification_update",
        "notification_update_missing",
        // nothing listens on the recorded Apprise address, so sending fails
        "notification_test",
        "notifications_after_test",
        "notification_delete",
        "notification_delete_missing",
        "notifications_reset",
        "p6n_guest_delete",
    ] {
        r.like(name).await;
    }
    let stored = server::abs::settings::load_setting(&st.abs, "notification-settings").unwrap().unwrap();
    assert_eq!((stored["appriseApiUrl"].clone(), stored["notifications"].clone()), (json!(null), json!([])));
}

#[tokio::test]
async fn a_notification_that_keeps_failing_is_turned_off() {
    let (mut r, _st, _dir) = set_up("abs-notifications-off").await;
    r.send("notifications_create").await;
    r.send("notifications_update").await;
    // two failures in a row reach the recorded limit of 2; the queue's delay is waited out between them
    r.send("notifications_fire_test_fail").await;
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    r.send("notifications_fire_test_fail").await;
    let a = r.send("notifications_after_fail").await;
    let n = &a.body["settings"]["notifications"][0];
    assert_eq!(
        (n["numConsecutiveFailedAttempts"].clone(), n["numTimesFired"].clone(), n["enabled"].clone()),
        (json!(2), json!(2), json!(false)),
        "{n}"
    );
}

#[tokio::test]
async fn notifications_reach_apprise_with_their_texts_filled_in() {
    use axum::routing::post;
    use std::sync::{Arc, Mutex};
    // a stand-in Apprise that keeps what it is sent
    let got: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
    let keep = got.clone();
    let apprise = axum::Router::new().route(
        "/notify",
        post(move |axum::Json(v): axum::Json<serde_json::Value>| {
            let keep = keep.clone();
            async move {
                keep.lock().unwrap().push(v);
                "ok"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, apprise).await.unwrap() });

    let (mut r, _st, _dir) = set_up("abs-notifications-send").await;
    r.send("notifications_create").await;
    let token = r.ctx["token"].clone();
    let auth = [("Authorization".to_string(), format!("Bearer {token}"))];
    let url = format!("http://{addr}/notify");
    let a = abs_replay::send(&r.app, "PATCH", "/api/notifications", Some(json!({"appriseApiUrl": url})), &auth).await;
    assert_eq!(a.status, 200);
    let id = r.ctx["notification_id"].clone();
    let a = abs_replay::send(&r.app, "GET", &format!("/api/notifications/{id}/test"), None, &auth).await;
    assert_eq!(a.status, 200, "{}", a.body);
    let a = abs_replay::send(&r.app, "GET", "/api/notifications/test", None, &auth).await;
    assert_eq!(a.status, 200);
    let sent = got.lock().unwrap().clone();
    let expected = json!({"urls": ["json://localhost"], "title": "Test v2.37.2", "body": "Body v2.37.2 {{podcastTitle}} v2.37.2"});
    assert_eq!(sent, vec![expected.clone(), expected]);
    let a = abs_replay::send(&r.app, "GET", "/api/notifications", None, &auth).await;
    let n = &a.body["settings"]["notifications"][0];
    assert_eq!((n["numTimesFired"].clone(), n["lastAttemptFailed"].clone()), (json!(1), json!(false)));
}
