//! The Audiobookshelf port's server settings (PARITY P6): the settings, the sorting words, the schedule check,
//! the task list, custom metadata providers and the email settings with e-reader devices. Replays the recorded
//! requests in the recorded order and compares every answer with the pinned original's.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{set_up, Replay};
use kit_server_core::Db;
use serde_json::{json, Value};

/// Adds the guest account the recording used for the "not an admin" answers.
async fn with_guest(r: &mut Replay) {
    for name in ["p6_guest_create", "p6_guest_login"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn server_settings_change_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-admin-settings").await;
    with_guest(&mut r).await;
    for name in ["settings_patch", "settings_patch_bad_origins", "settings_patch_empty", "settings_patch_guest", "settings_patch_reset"] {
        r.like(name).await;
    }
    let stored = server::abs::settings::load_setting(&st.abs, "server-settings").unwrap().unwrap();
    assert_eq!(stored["tokenSecret"].as_str().map(str::len), Some(st.abs_server.tokens.secret().len()), "the secret can't be changed");

    // a book whose title starts with a new sorting word gets a new sort title
    let now = "2026-11-05 18:00:00.000 +00:00";
    st.abs
        .execute(
            "INSERT INTO books (id, title, titleIgnorePrefix, createdAt, updatedAt) VALUES ('b1', 'An Example', 'An Example', ?, ?)",
            vec![json!(now), json!(now)],
        )
        .unwrap();
    let a = r.send("sorting_prefixes").await;
    assert_eq!(a.body["rowsUpdated"], 1, "{}", a.body);
    assert_eq!(a.body["serverSettings"]["sortingPrefixes"], json!(["the", "a", "an"]));
    let row = st.abs.one("SELECT titleIgnorePrefix FROM books WHERE id = 'b1'", vec![]).unwrap().unwrap();
    assert_eq!(row["titleIgnorePrefix"], "Example");
    st.abs.execute("DELETE FROM books WHERE id = 'b1'", vec![]).unwrap();
    for name in [
        "sorting_prefixes",
        "sorting_prefixes_empty",
        "sorting_prefixes_blank",
        "sorting_prefixes_not_list",
        "sorting_prefixes_guest",
        "sorting_prefixes_reset",
    ] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn schedules_and_tasks_answer_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-admin-cron").await;
    with_guest(&mut r).await;
    for name in [
        "cron_valid",
        "cron_valid_names",
        "cron_valid_seconds",
        "cron_bad_minute",
        "cron_bad_short",
        "cron_bad_step",
        "cron_missing",
        "cron_guest",
        "tasks",
        "tasks_queue",
        "tasks_guest",
    ] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn metadata_providers_are_added_and_removed_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-admin-providers").await;
    with_guest(&mut r).await;
    for name in [
        "providers",
        "providers_create",
        "providers_create_missing",
        "providers_create_bad_url",
        "providers_create_podcast",
        "providers_list",
        "providers_guest",
    ] {
        r.like(name).await;
    }
    // a library that uses the provider goes back to Google Books when it is removed
    let slug = format!("custom-{}", r.ctx["provider_id"]);
    st.abs.execute("UPDATE libraries SET provider = ? WHERE id = ?", vec![json!(slug), json!(r.ctx["library"])]).unwrap();
    r.like("providers_delete").await;
    let row = st.abs.one("SELECT provider FROM libraries WHERE id = ?", vec![json!(r.ctx["library"])]).unwrap().unwrap();
    assert_eq!(row["provider"], "google");
    for name in ["providers_delete_missing", "providers_delete_podcast"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn email_settings_and_ereader_devices_change_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-admin-email").await;
    with_guest(&mut r).await;
    for name in [
        "email_settings",
        "email_settings_guest",
        "email_settings_patch",
        "email_settings_patch_same",
        "email_settings_patch_nan_port",
        "ereader_devices_missing",
        "ereader_devices_incomplete",
        "ereader_devices",
    ] {
        r.like(name).await;
    }
    // the guest sees the devices open to guests and to them
    let a = r.send("guest_authorize_devices").await;
    let names: Vec<&str> = a.body["ereaderDevices"].as_array().unwrap().iter().filter_map(|d| d["name"].as_str()).collect();
    assert_eq!(names, ["Shared", "Mine"]);
    for name in [
        "ereader_devices_same",
        "guest_authorize_devices",
        "send_ebook_unknown_device",
        "send_ebook_guest_forbidden",
        "send_ebook_missing_item",
        "email_settings_after",
        "email_settings_reset",
        "p6_guest_delete",
    ] {
        r.like(name).await;
    }
    let stored = server::abs::settings::load_setting(&st.abs, "email-settings").unwrap().unwrap();
    assert_eq!(stored["ereaderDevices"], json!([]));
    assert_eq!(stored["host"], Value::Null);
}
