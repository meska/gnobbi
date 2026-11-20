//! Matrix pusher for the APNs gateway (Sygnal): every signed-in account registers the same device token on its own homeserver.
use matrix_sdk::Client;
use matrix_sdk::ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind};
use matrix_sdk::ruma::push::{HttpPusherData, PushFormat};
use serde_json::json;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::watch;

pub const APP_ID: &str = "net.meskatech.gnobbi";
const GATEWAY: &str = "https://sygnal.gnobbi.chat/_matrix/push/v1/notify";

/// What the platform layer knows: the APNs token (lowercase hex) and the texts of the generic alert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushSettings {
    pub token: String,
    pub lang: String,
    /// Localized body of the generic alert (the push carries only event ids: no content, no decryption yet).
    pub body: String,
}

static SETTINGS: OnceLock<watch::Sender<Option<PushSettings>>> = OnceLock::new();

fn channel() -> &'static watch::Sender<Option<PushSettings>> {
    SETTINGS.get_or_init(|| watch::channel(None).0)
}

/// Publishes (or clears) the settings; running and future workers pick them up.
pub fn set(settings: Option<PushSettings>) {
    channel().send_if_modified(|current| std::mem::replace(current, settings.clone()) != settings);
}

/// Pure: the pusher Sygnal turns into a visible APNs alert (`default_payload` is merged under room_id/event_id).
pub fn pusher(s: &PushSettings, device_name: &str) -> Pusher {
    let mut data = HttpPusherData::new(GATEWAY.to_owned());
    data.format = Some(PushFormat::EventIdOnly);
    data.data.insert(
        "default_payload".into(),
        json!({ "aps": { "alert": { "title": "Gnobbi", "body": s.body }, "sound": "default", "mutable-content": 1 } }),
    );
    PusherInit {
        ids: PusherIds::new(s.token.clone(), APP_ID.to_owned()),
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
pub(crate) async fn keep_registered(client: Client, device_name: &'static str) {
    let mut rx = channel().subscribe();
    loop {
        let current = rx.borrow_and_update().clone();
        let retry = match current {
            Some(s) => match client.pusher().set(pusher(&s, device_name), false).await {
                Ok(()) => false,
                Err(_) => {
                    eprintln!("Gnobbi: push registration failed");
                    true
                }
            },
            None => false,
        };
        tokio::select! {
            changed = rx.changed() => if changed.is_err() { return },
            _ = tokio::time::sleep(Duration::from_secs(60)), if retry => {}
        }
    }
}

/// Before the server logout: the token is useless to the account that leaves.
pub(crate) async fn remove(client: &Client) {
    let token = channel().borrow().as_ref().map(|s| s.token.clone());
    if let Some(token) = token {
        let pusher = client.pusher();
        let gone = pusher.delete(PusherIds::new(token, APP_ID.to_owned()));
        if !matches!(tokio::time::timeout(Duration::from_secs(10), gone).await, Ok(Ok(()))) {
            eprintln!("Gnobbi: push removal failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> PushSettings {
        PushSettings { token: "ab12".into(), lang: "it".into(), body: "Nuovo messaggio".into() }
    }

    #[test]
    fn pusher_json_is_what_sygnal_expects() {
        let body = serde_json::to_value(pusher(&settings(), "Gnobbi (iOS)")).unwrap();
        assert_eq!(
            body,
            json!({
                "pushkey": "ab12", "app_id": "net.meskatech.gnobbi", "app_display_name": "Gnobbi",
                "kind": "http", "lang": "it", "device_display_name": "Gnobbi (iOS)",
                "data": {
                    "url": "https://sygnal.gnobbi.chat/_matrix/push/v1/notify", "format": "event_id_only",
                    "default_payload": { "aps": { "alert": { "title": "Gnobbi", "body": "Nuovo messaggio" }, "sound": "default", "mutable-content": 1 } }
                }
            })
        );
    }

    #[tokio::test]
    async fn set_notifies_only_on_change() {
        let mut rx = channel().subscribe();
        set(Some(settings()));
        assert!(rx.has_changed().unwrap());
        rx.borrow_and_update();
        set(Some(settings()));
        assert!(!rx.has_changed().unwrap());
        set(None);
    }
}
