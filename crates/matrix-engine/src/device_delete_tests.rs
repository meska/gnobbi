// delete_device against a scripted fake homeserver (UIA password flow). Included by auth_tests.
use crate::verification::{DeviceDeleteError, DevicePassword};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const UIA_PASSWORD: &str = r#"{"flows":[{"stages":["m.login.password"]}],"params":{},"session":"S1"}"#;
const UIA_SSO: &str = r#"{"flows":[{"stages":["m.login.sso"]}],"params":{},"session":"S1"}"#;

/// Answers each delete_devices POST with the next scripted (status, body); records the bodies.
async fn fake(script: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        let mut script = script.into_iter();
        loop {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let text = loop {
                let n = s.read(&mut chunk).await.unwrap_or(0);
                buf.extend_from_slice(&chunk[..n]);
                let t = String::from_utf8_lossy(&buf).to_string();
                if let Some(i) = t.find("\r\n\r\n") {
                    let len = t[..i]
                        .to_lowercase()
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0usize);
                    if buf.len() >= i + 4 + len || n == 0 {
                        break t;
                    }
                } else if n == 0 {
                    break t;
                }
            };
            let (status, body) = if text.contains("delete_devices") {
                log.lock().unwrap().push(text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned());
                script.next().unwrap_or((500, "{}"))
            } else {
                (404, r#"{"errcode":"M_UNRECOGNIZED","error":"x"}"#)
            };
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(reply.as_bytes()).await;
            let _ = s.shutdown().await;
        }
    });
    (url, seen)
}

async fn session(url: &str) -> AccountSession {
    let client = matrix_sdk::Client::builder()
        .homeserver_url(url)
        .server_versions([matrix_sdk::ruma::api::MatrixVersion::V1_0])
        .build()
        .await
        .unwrap();
    let s: matrix_sdk::authentication::matrix::MatrixSession = serde_json::from_value(serde_json::json!({
        "user_id": "@a:x.test", "device_id": "CURRENT", "access_token": "tok"
    }))
    .unwrap();
    client.restore_session(s).await.unwrap();
    AccountSession { account_id: AccountId::new(1), client: Some(client), store: None }
}

fn delete(script: Vec<(u16, &'static str)>, device: &str) -> (Result<(), DeviceDeleteError>, Vec<String>) {
    runtime().block_on(async {
        let (url, seen) = fake(script).await;
        let s = session(&url).await;
        let r = s.delete_device(device, DevicePassword::new("hunter2".into())).await;
        let bodies = seen.lock().unwrap().clone();
        (r, bodies)
    })
}

#[test]
fn delete_current_device_is_refused_without_any_request() {
    let (r, bodies) = delete(vec![], "CURRENT");
    assert_eq!(r, Err(DeviceDeleteError::CurrentDevice));
    assert!(bodies.is_empty());
}

#[test]
fn delete_device_succeeds_with_session_and_password_on_second_call() {
    let (r, bodies) = delete(vec![(401, UIA_PASSWORD), (200, "{}")], "OTHER");
    assert_eq!(r, Ok(()));
    assert_eq!(bodies.len(), 2);
    assert!(!bodies[0].contains("auth"));
    assert!(bodies[1].contains("m.login.password") && bodies[1].contains("S1") && bodies[1].contains("@a:x.test"));
}

#[test]
fn delete_device_wrong_password_is_typed() {
    let again = r#"{"errcode":"M_FORBIDDEN","error":"Invalid password","flows":[{"stages":["m.login.password"]}],"params":{},"session":"S1"}"#;
    let (r, _) = delete(vec![(401, UIA_PASSWORD), (401, again)], "OTHER");
    assert_eq!(r, Err(DeviceDeleteError::WrongPassword));
}

#[test]
fn delete_device_without_password_flow_sends_no_password() {
    let (r, bodies) = delete(vec![(401, UIA_SSO)], "OTHER");
    assert_eq!(r, Err(DeviceDeleteError::Unavailable));
    assert_eq!(bodies.len(), 1);
}

#[test]
fn password_never_appears_in_debug_output() {
    let p = DevicePassword::new("hunter2".into());
    assert!(!format!("{p:?}").contains("hunter2"));
}
