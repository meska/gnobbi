//! Matrix pusher for the push gateway (Sygnal; APNs and FCM): every signed-in account registers the same device token on its own homeserver.
use matrix_sdk::Client;
use matrix_sdk::ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind};
use matrix_sdk::ruma::push::{HttpPusherData, PushFormat};
use matrix_domain::AccountId;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::watch;

pub const APP_ID_IOS: &str = "net.meskatech.gnobbi";
pub const APP_ID_ANDROID: &str = "net.meskatech.gnobbi.android";
const GATEWAY: &str = "https://sygnal.gnobbi.chat/_matrix/push/v1/notify";

/// Which push service delivers to this device; it picks the Sygnal app id and the payload shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushPlatform {
    Apns,
    Fcm,
}

impl PushPlatform {
    pub fn app_id(self) -> &'static str {
        match self {
            Self::Apns => APP_ID_IOS,
            Self::Fcm => APP_ID_ANDROID,
        }
    }
}

/// What the platform layer knows: the token (APNs: lowercase hex; FCM: opaque, unchanged) and the texts of the generic alert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushSettings {
    pub platform: PushPlatform,
    pub token: String,
    pub lang: String,
    /// Localized body of the generic alert (the push carries only event ids: no content, no decryption yet).
    pub body: String,
    /// The app's Notifications switch: off removes the pusher from the homeserver, on registers it again.
    pub enabled: bool,
}

static SETTINGS: OnceLock<watch::Sender<Option<PushSettings>>> = OnceLock::new();

fn channel() -> &'static watch::Sender<Option<PushSettings>> {
    SETTINGS.get_or_init(|| watch::channel(None).0)
}

/// Which store a push belongs to; the notification extension reads it from the payload (`gnobbi_account` = `<catalog>:<worker>:<dir>`, strings flat so Swift and the tap handler parse them alike).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountTag {
    /// Catalog id of the account (first part of the click key `<catalog>:<worker>:<room>`).
    pub catalog: AccountId,
    /// Store folder relative to the data dir: `accounts/<catalog>` or `profile`.
    pub dir: String,
}

/// Account tags by profile root, published by the app once the catalog knows the account; a worker registers its pusher only when it has one.
static TAGS: OnceLock<watch::Sender<HashMap<PathBuf, AccountTag>>> = OnceLock::new();

fn tags() -> &'static watch::Sender<HashMap<PathBuf, AccountTag>> {
    TAGS.get_or_init(|| watch::channel(HashMap::new()).0)
}

pub fn set_account(root: PathBuf, tag: AccountTag) {
    tags().send_if_modified(|all| all.insert(root, tag.clone()).as_ref() != Some(&tag));
}

/// Publishes (or clears) the settings; running and future workers pick them up.
pub fn set(settings: Option<PushSettings>) {
    channel().send_if_modified(|current| std::mem::replace(current, settings.clone()) != settings);
}

/// The settings now published (what the platform layer last handed over).
pub fn current() -> Option<PushSettings> {
    channel().borrow().clone()
}

/// Pure: the pusher Sygnal turns into a visible APNs alert (`default_payload` is merged under room_id/event_id).
/// `worker` is the store's account id (folder `account-<worker>`, Keychain item, second part of the click key).
pub fn pusher(s: &PushSettings, device_name: &str, tag: &AccountTag, worker: AccountId) -> Pusher {
    let mut data = HttpPusherData::new(GATEWAY.to_owned());
    data.format = Some(PushFormat::EventIdOnly);
    let account = format!("{}:{}:{}", tag.catalog.as_u64(), worker.as_u64(), tag.dir);
    let payload = match s.platform {
        PushPlatform::Apns => json!({
            "aps": { "alert": { "title": "Gnobbi", "body": s.body }, "sound": "default", "mutable-content": 1 },
            "gnobbi_account": account,
        }),
        PushPlatform::Fcm => json!({ "gnobbi_account": account }),
    };
    data.data.insert("default_payload".into(), payload);
    PusherInit {
        ids: PusherIds::new(s.token.clone(), s.platform.app_id().to_owned()),
        app_display_name: "Gnobbi".to_owned(),
        kind: PusherKind::Http(data),
        lang: s.lang.clone(),
        device_display_name: device_name.to_owned(),
        profile_tag: None,
    }
    .into()
}

/// Keeps this account's pusher equal to the published settings; a failure is one log line and a retry in a minute.
/// Runs until aborted.
pub(crate) async fn keep_registered(client: Client, device_name: &'static str, root: Option<PathBuf>, worker: AccountId) {
    let Some(root) = root else { return };
    let mut rx = channel().subscribe();
    let mut tag_rx = tags().subscribe();
    // The previous process may have registered another token (Android rotates it while the app is dead).
    let mut registered: Option<PusherIds> = load_registered(&root);
    loop {
        let current = rx.borrow_and_update().clone();
        let tag = tag_rx.borrow_and_update().get(&root).cloned();
        let retry = match current.zip(tag) {
            Some((s, _)) if !s.enabled => match registered.take() {
                Some(old) if !matches!(tokio::time::timeout(Duration::from_secs(10), client.pusher().delete(old.clone())).await, Ok(Ok(()))) => {
                    eprintln!("Gnobbi: push removal failed");
                    registered = Some(old);
                    true
                }
                _ => {
                    let _ = std::fs::remove_file(registered_file(&root));
                    false
                }
            },
            Some((s, tag)) => {
                // Token rotation: `set(.., false)` keeps the old pushkey of this user, so Sygnal would keep pushing to a dead token.
                let ids = ids_of(&s);
                if let Some(old) = registered.take_if(|o| (&o.pushkey, &o.app_id) != (&ids.pushkey, &ids.app_id)) {
                    let _ = tokio::time::timeout(Duration::from_secs(10), client.pusher().delete(old)).await;
                }
                match client.pusher().set(pusher(&s, device_name, &tag, worker), false).await {
                    Ok(()) => {
                        save_registered(&root, &ids);
                        registered = Some(ids);
                        false
                    }
                    Err(_) => {
                        eprintln!("Gnobbi: push registration failed");
                        true
                    }
                }
            }
            None => false,
        };
        tokio::select! {
            changed = rx.changed() => if changed.is_err() { return },
            changed = tag_rx.changed() => if changed.is_err() { return },
            _ = tokio::time::sleep(Duration::from_secs(60)), if retry => {}
        }
    }
}

fn registered_file(root: &std::path::Path) -> PathBuf {
    root.join("push-registered.json")
}

fn load_registered(root: &std::path::Path) -> Option<PusherIds> {
    serde_json::from_slice(&std::fs::read(registered_file(root)).ok()?).ok()
}

fn save_registered(root: &std::path::Path, ids: &PusherIds) {
    if let Ok(json) = serde_json::to_vec(ids) {
        let _ = std::fs::write(registered_file(root), json);
    }
}

/// The ids the pusher was registered with (also what removal must name).
fn ids_of(s: &PushSettings) -> PusherIds {
    PusherIds::new(s.token.clone(), s.platform.app_id().to_owned())
}

/// Before the server logout: the token is useless to the account that leaves.
pub(crate) async fn remove(client: &Client) {
    let ids = channel().borrow().as_ref().map(ids_of);
    if let Some(ids) = ids {
        let pusher = client.pusher();
        let gone = pusher.delete(ids);
        if !matches!(tokio::time::timeout(Duration::from_secs(10), gone).await, Ok(Ok(()))) {
            eprintln!("Gnobbi: push removal failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settings and tags are process-wide: tests that publish them take turns.
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn settings() -> PushSettings {
        PushSettings { platform: PushPlatform::Apns, token: "ab12".into(), lang: "it".into(), body: "Nuovo messaggio".into(), enabled: true }
    }

    fn tag() -> AccountTag {
        AccountTag { catalog: AccountId::new(3), dir: "accounts/3".into() }
    }

    #[test]
    fn pusher_json_is_what_sygnal_expects() {
        let body = serde_json::to_value(pusher(&settings(), "Gnobbi (iOS)", &tag(), AccountId::new(7))).unwrap();
        assert_eq!(
            body,
            json!({
                "pushkey": "ab12", "app_id": "net.meskatech.gnobbi", "app_display_name": "Gnobbi",
                "kind": "http", "lang": "it", "device_display_name": "Gnobbi (iOS)",
                "data": {
                    "url": "https://sygnal.gnobbi.chat/_matrix/push/v1/notify", "format": "event_id_only",
                    "default_payload": { "aps": { "alert": { "title": "Gnobbi", "body": "Nuovo messaggio" }, "sound": "default", "mutable-content": 1 },
                        "gnobbi_account": "3:7:accounts/3"
                    }
                }
            })
        );
    }

    #[test]
    fn fcm_pusher_json_keeps_token_and_drops_aps() {
        let s = PushSettings { platform: PushPlatform::Fcm, token: "Fcm:Token_1-x".into(), ..settings() };
        let body = serde_json::to_value(pusher(&s, "Gnobbi (Android)", &tag(), AccountId::new(7))).unwrap();
        assert_eq!(
            body,
            json!({
                "pushkey": "Fcm:Token_1-x", "app_id": "net.meskatech.gnobbi.android", "app_display_name": "Gnobbi",
                "kind": "http", "lang": "it", "device_display_name": "Gnobbi (Android)",
                "data": {
                    "url": "https://sygnal.gnobbi.chat/_matrix/push/v1/notify", "format": "event_id_only",
                    "default_payload": { "gnobbi_account": "3:7:accounts/3" }
                }
            })
        );
    }

    #[test]
    fn removal_uses_the_registered_app_id() {
        for (platform, app_id) in [(PushPlatform::Apns, APP_ID_IOS), (PushPlatform::Fcm, APP_ID_ANDROID)] {
            let s = PushSettings { platform, ..settings() };
            let registered = serde_json::to_value(pusher(&s, "d", &tag(), AccountId::new(1))).unwrap();
            let ids = serde_json::to_value(ids_of(&s)).unwrap();
            assert_eq!(ids, json!({ "pushkey": "ab12", "app_id": app_id }));
            assert_eq!((&registered["pushkey"], &registered["app_id"]), (&ids["pushkey"], &ids["app_id"]));
        }
    }

    #[tokio::test]
    async fn set_notifies_only_on_change() {
        let _turn = LOCK.lock().await;
        let mut rx = channel().subscribe();
        set(Some(settings()));
        assert!(rx.has_changed().unwrap());
        rx.borrow_and_update();
        set(Some(settings()));
        assert!(!rx.has_changed().unwrap());
        set(None);
    }

    #[tokio::test]
    async fn set_account_notifies_only_on_change() {
        let mut rx = tags().subscribe();
        let root = PathBuf::from("/tmp/push-test-root");
        set_account(root.clone(), tag());
        assert!(rx.has_changed().unwrap());
        rx.borrow_and_update();
        set_account(root.clone(), tag());
        assert!(!rx.has_changed().unwrap());
        set_account(root, AccountTag { catalog: AccountId::new(4), dir: "accounts/4".into() });
        assert!(rx.has_changed().unwrap());
    }

    /// Minimal homeserver: records "<path> <body>" of every request, answers login and `{}` for the rest.
    async fn mock_homeserver() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let seen = seen.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head, body) = loop {
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        buf.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&buf).into_owned();
                        if let Some((head, body)) = text.split_once("\r\n\r\n") {
                            let len = head.to_lowercase().split("content-length:").nth(1).and_then(|v| v.lines().next()?.trim().parse::<usize>().ok()).unwrap_or(0);
                            if body.len() >= len || n == 0 {
                                break (head.to_owned(), body.to_owned());
                            }
                        } else if n == 0 {
                            return;
                        }
                    };
                    let path = head.split_whitespace().nth(1).unwrap_or("").to_owned();
                    seen.lock().unwrap().push(format!("{path} {body}"));
                    let reply = if path.ends_with("/login") {
                        r#"{"user_id":"@a:t","access_token":"tok","device_id":"D"}"#
                    } else if path.ends_with("/versions") {
                        r#"{"versions":["v1.11"]}"#
                    } else {
                        "{}"
                    };
                    let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).as_bytes()).await;
                });
            }
        });
        (url, log)
    }

    #[tokio::test]
    async fn token_rotated_while_the_process_was_dead_removes_the_old_pushkey() {
        let _turn = LOCK.lock().await;
        let root = std::env::temp_dir().join(format!("gnobbi-push-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let old = PushSettings { platform: PushPlatform::Fcm, token: "token-A".into(), ..settings() };
        save_registered(&root, &ids_of(&old));
        let (url, log) = mock_homeserver().await;
        let client = Client::builder().homeserver_url(&url).build().await.unwrap();
        client.matrix_auth().login_username("@a:t", "x").send().await.unwrap();
        set(Some(PushSettings { token: "token-B".into(), ..old }));
        set_account(root.clone(), tag());
        let run = keep_registered(client, "Gnobbi (Android)", Some(root.clone()), AccountId::new(1));
        let _ = tokio::time::timeout(Duration::from_secs(3), run).await;
        set(None);
        let pushers: Vec<String> = log.lock().unwrap().iter().filter(|l| l.contains("/pushers/set")).cloned().collect();
        assert_eq!(pushers.len(), 2, "{pushers:?}");
        assert!(pushers[0].contains("token-A") && pushers[0].contains("\"kind\":null"), "{pushers:?}");
        assert!(pushers[1].contains("token-B") && pushers[1].contains("\"kind\":\"http\""), "{pushers:?}");
        assert_eq!(load_registered(&root).unwrap().pushkey, "token-B");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Switch off removes the registered pusher (named by its app id) and forgets it; on registers it again.
    #[tokio::test]
    async fn switch_off_removes_the_registered_pusher_and_on_registers_it_again() {
        let _turn = LOCK.lock().await;
        let root = std::env::temp_dir().join(format!("gnobbi-push-switch-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let on = PushSettings { platform: PushPlatform::Fcm, token: "token-A".into(), ..settings() };
        let (url, log) = mock_homeserver().await;
        let client = Client::builder().homeserver_url(&url).build().await.unwrap();
        client.matrix_auth().login_username("@a:t", "x").send().await.unwrap();
        set(Some(on.clone()));
        set_account(root.clone(), tag());
        let run = tokio::spawn(keep_registered(client, "Gnobbi (Android)", Some(root.clone()), AccountId::new(1)));
        let count = |kind: &str| log.lock().unwrap().iter().filter(|l| l.contains("/pushers/set") && l.contains(kind)).count();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(count("\"kind\":\"http\""), 1);
        set(Some(PushSettings { enabled: false, ..on.clone() }));
        tokio::time::sleep(Duration::from_millis(800)).await;
        let off: Vec<String> = log.lock().unwrap().iter().filter(|l| l.contains("\"kind\":null")).cloned().collect();
        assert!(off.len() == 1 && off[0].contains("token-A") && off[0].contains(APP_ID_ANDROID), "{off:?}");
        assert!(load_registered(&root).is_none());
        set(Some(on));
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(count("\"kind\":\"http\""), 2);
        run.abort();
        set(None);
        std::fs::remove_dir_all(&root).ok();
    }
}
