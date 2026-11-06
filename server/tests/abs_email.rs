//! The Audiobookshelf port's mail sender (PARITY P6): the test mail and e-books sent to devices. Replays the
//! recorded answers, then sends through a small mail server started here and checks what it received.

mod abs_replay;
mod common;
mod parity;

use abs_replay::{send, set_up, Replay};
use kit_server_core::Db;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Starts a mail server on a free local port that accepts every mail without TLS or sign-in, and returns its port
/// and the mails it received (each mail's whole text, from the envelope to the end of the data).
async fn mail_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mails = Arc::new(Mutex::new(Vec::new()));
    let got = mails.clone();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else { return };
            let got = got.clone();
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 test ready\r\n").await.unwrap();
                let mut mail = String::new();
                let mut in_data = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    if in_data {
                        if line == "." {
                            in_data = false;
                            got.lock().unwrap().push(std::mem::take(&mut mail));
                            write.write_all(b"250 queued\r\n").await.unwrap();
                        } else {
                            mail.push_str(&line);
                            mail.push('\n');
                        }
                        continue;
                    }
                    let verb = line.split(' ').next().unwrap_or_default().to_ascii_uppercase();
                    let answer: &[u8] = match verb.as_str() {
                        "EHLO" | "HELO" => b"250 test\r\n",
                        "DATA" => {
                            in_data = true;
                            b"354 go on\r\n"
                        }
                        "QUIT" => {
                            write.write_all(b"221 bye\r\n").await.unwrap();
                            return;
                        }
                        _ => {
                            mail.push_str(&line);
                            mail.push('\n');
                            b"250 ok\r\n"
                        }
                    };
                    write.write_all(answer).await.unwrap();
                }
            });
        }
    });
    (port, mails)
}

/// Sends a request as root.
async fn as_root(r: &Replay, method: &str, path: &str, body: Value) -> abs_replay::Answer {
    let auth = [("Authorization".to_string(), format!("Bearer {}", r.ctx["token"]))];
    send(&r.app, method, path, Some(body), &auth).await
}

#[tokio::test]
async fn mail_is_sent_like_the_original() {
    let (mut r, st, dir) = set_up("abs-email").await;
    for name in ["p6e_guest_create", "p6e_guest_login", "email_test_guest", "email_test_no_server", "p6e_guest_delete"] {
        r.like(name).await;
    }

    let (port, mails) = mail_server().await;
    let settings =
        json!({"host": "127.0.0.1", "port": port, "secure": false, "fromAddress": "folio@example.org", "testAddress": "me@example.org"});
    assert_eq!(as_root(&r, "PATCH", "/api/emails/settings", settings).await.status, 200);
    let a = as_root(&r, "POST", "/api/emails/test", json!({})).await;
    assert_eq!((a.status, a.content_type.as_str()), (200, "text/plain"), "{}", a.body);
    {
        let got = mails.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].contains("MAIL FROM:<folio@example.org>"), "{}", got[0]);
        assert!(got[0].contains("RCPT TO:<me@example.org>"), "{}", got[0]);
        assert!(got[0].contains("Subject: Test email from Audiobookshelf"), "{}", got[0]);
        assert!(got[0].contains("Success!"), "{}", got[0]);
    }

    let book = dir.join("A book.epub");
    std::fs::write(&book, b"the book").unwrap();
    let now = "2026-11-05 18:00:00.000 +00:00";
    let ebook = json!({"metadata": {"filename": "A book.epub", "path": book.display().to_string()}});
    st.abs
        .execute(
            "INSERT INTO books (id, title, ebookFile, createdAt, updatedAt) VALUES ('b1', 'A book', ?, ?, ?)",
            vec![json!(ebook.to_string()), json!(now), json!(now)],
        )
        .unwrap();
    st.abs
        .execute(
            "INSERT INTO libraryItems (id, mediaId, mediaType, libraryId, createdAt, updatedAt) VALUES ('li-b1', 'b1', 'book', ?, ?, ?)",
            vec![json!(r.ctx["library"]), json!(now), json!(now)],
        )
        .unwrap();
    let devices = json!({"ereaderDevices": [{"name": "Kindle", "email": "k@kindle.example"}]});
    assert_eq!(as_root(&r, "POST", "/api/emails/ereader-devices", devices).await.status, 200);
    let a = as_root(&r, "POST", "/api/emails/send-ebook-to-device", json!({"deviceName": "Kindle", "libraryItemId": "li-b1"})).await;
    assert_eq!(a.status, 200, "{}", a.body);
    let got = mails.lock().unwrap();
    assert_eq!(got.len(), 2);
    let mail = &got[1];
    assert!(mail.contains("RCPT TO:<k@kindle.example>"), "{mail}");
    assert!(mail.contains("Subject: Here is your Ebook!"), "{mail}");
    assert!(mail.contains("<div dir=\"auto\"></div>"), "{mail}");
    assert!(mail.contains("application/epub+zip"), "{mail}");
    assert!(mail.contains("A book.epub"), "{mail}");
}

#[tokio::test]
async fn a_mail_server_that_refuses_is_reported() {
    let (r, _st, _dir) = set_up("abs-email-refused").await;
    let (port, _mails) = mail_server().await;
    let settings = json!({"host": "127.0.0.1", "port": port, "secure": false, "fromAddress": "not an address"});
    as_root(&r, "PATCH", "/api/emails/settings", settings).await;
    let a = as_root(&r, "POST", "/api/emails/test", json!({})).await;
    assert_eq!(a.status, 400);
    assert!(!a.body.as_str().unwrap_or_default().is_empty(), "{}", a.body);
}
