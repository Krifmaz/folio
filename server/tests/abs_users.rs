//! The Audiobookshelf port's user management (PARITY P2b): users, API keys, the sign-in settings and the
//! password change. Replays the recorded requests (tests/parity/abs/requests.json) in the recorded order and
//! compares every answer with the pinned original's (tests/parity/fixtures/abs).

mod abs_replay;
mod common;
mod parity;

use abs_replay::{recorded_cookies, send, set_up};
use kit_server_core::Db;
use parity::{assert_like_with, fixture};
use serde_json::{json, Value};

#[tokio::test]
async fn users_are_added_read_changed_and_removed_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-users").await;
    for name in [
        "users_create",
        "users_create_taken",
        "users_create_bad_type",
        "users_create_missing",
        "users_list",
        "user_one",
        "user_missing",
        "user_update",
        "user_update_bad_key",
        "user_update_bad_type",
    ] {
        r.like(name).await;
    }
    // the recording's root had listening progress; Folio's test root has none
    let a = r.send("user_update_root_type").await;
    let mut fx = fixture("abs", "user_update_root_type");
    fx.body["user"]["mediaProgress"] = json!([]);
    assert_like_with(&mut r.m, a.status, &a.body, &fx);
    for name in [
        "user_unlink",
        "reader_login",
        "reader_users_list",
        "reader_user_self",
        "reader_user_other",
        "reader_api_keys",
        "reader_auth_settings",
    ] {
        r.like(name).await;
    }
    // the new user's library list and tags are stored in the permissions, like the original
    let row = st.abs.one("SELECT permissions FROM users WHERE username = 'reader'", vec![]).unwrap().unwrap();
    let p: Value = serde_json::from_str(row["permissions"].as_str().unwrap()).unwrap();
    assert_eq!(p["librariesAccessible"], json!([r.ctx["library"]]));
    assert_eq!(p["itemTagsSelected"], json!(["kids"]));

    for name in ["users_delete_self", "users_delete", "users_delete_missing", "users_list_after"] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn a_password_change_signs_out_elsewhere_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-password").await;
    r.send("users_create").await;
    r.send("users_list").await;
    r.send("reader_login").await;
    let old_refresh = r.ctx["reader_refresh"].clone();
    for name in ["me_password_invalid", "me_password_wrong", "me_password_empty"] {
        r.like(name).await;
    }
    let a = r.like("me_password").await;
    assert_eq!(a.cookies, recorded_cookies("me_password"));
    assert_ne!(a.body["user"]["refreshToken"].as_str().unwrap(), old_refresh, "the session's tokens are new");
    r.like("reader_login_old_password").await;
    let a = send(&r.app, "POST", "/login", Some(json!({"username": "reader", "password": "pw2"})), &[]).await;
    assert_eq!(a.status, 200, "the new password signs in");
}

#[tokio::test]
async fn api_keys_sign_in_until_turned_off_like_the_original() {
    let (mut r, _st, _dir) = set_up("abs-api-keys").await;
    r.send("users_create").await;
    r.send("users_list").await;
    r.send("user_update").await;
    for name in [
        "api_keys_create",
        "api_keys_create_root",
        "api_keys_create_bad_name",
        "api_keys_create_bad_expiry",
        "api_keys_create_no_user",
        "api_keys_list",
        "api_key_signs_in",
        "api_key_inactive_signs_in",
        "api_keys_update",
        "api_keys_update_bad",
        "api_keys_update_missing",
        "api_key_turned_off_signs_in",
        "api_keys_delete",
        "api_keys_delete_missing",
    ] {
        r.like(name).await;
    }
}

#[tokio::test]
async fn sign_in_settings_are_checked_and_saved_like_the_original() {
    let (mut r, st, _dir) = set_up("abs-auth-settings").await;
    let before = r.send("auth_settings_after").await;
    assert_eq!(before.status, 200);
    for name in [
        "auth_settings_patch",
        "auth_settings_after",
        "status_with_message",
        "auth_settings_patch_none",
        "auth_settings_patch_bad",
        "auth_settings_reset",
    ] {
        r.like(name).await;
    }
    // the reset is stored
    let stored = server::abs::settings::load_setting(&st.abs, "server-settings").unwrap().unwrap();
    assert_eq!(stored["authLoginCustomMessage"], Value::Null);
    assert_eq!(stored["authOpenIDMobileRedirectURIs"], json!(["audiobookshelf://oauth"]));
}
