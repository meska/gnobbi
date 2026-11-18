//! Notifications through the SDK: push rules decide what notifies (`register_notification_handler`),
//! the per-room mode is the SDK push rule (`NotificationSettings`), written only on an explicit user action.
use crate::{RoomListEvent, operations::OperationError};
use matrix_domain::notifications::{MessagePreview, NotifyKind, PreviewContent, RoomNotifyMode, should_notify};
use matrix_domain::{RoomId, RoomScope};
use matrix_sdk::{
    Client, deserialized_responses::RawAnySyncOrStrippedTimelineEvent,
    notification_settings::RoomNotificationMode,
    ruma::{MilliSecondsSinceUnixEpoch, OwnedRoomId, OwnedUserId, UserId},
};
use std::{collections::HashSet, sync::{Arc, Mutex}};
use tokio::sync::mpsc::UnboundedSender;

pub(crate) fn mode_from_sdk(mode: Option<RoomNotificationMode>) -> RoomNotifyMode {
    match mode {
        None | Some(RoomNotificationMode::AllMessages) => RoomNotifyMode::AllMessages,
        Some(RoomNotificationMode::MentionsAndKeywordsOnly) => RoomNotifyMode::MentionsAndKeywordsOnly,
        Some(RoomNotificationMode::Mute) => RoomNotifyMode::Mute,
    }
}

fn mode_to_sdk(mode: RoomNotifyMode) -> RoomNotificationMode {
    match mode {
        RoomNotifyMode::AllMessages => RoomNotificationMode::AllMessages,
        RoomNotifyMode::MentionsAndKeywordsOnly => RoomNotificationMode::MentionsAndKeywordsOnly,
        RoomNotifyMode::Mute => RoomNotificationMode::Mute,
    }
}

/// What a notifying event is; None = no alert (stripped state other than an invite for us).
fn notify_kind(event: &RawAnySyncOrStrippedTimelineEvent, highlight: bool, own: Option<&UserId>) -> Option<NotifyKind> {
    match event {
        RawAnySyncOrStrippedTimelineEvent::Stripped(raw) => {
            let invite = raw.get_field::<String>("type").ok().flatten().as_deref() == Some("m.room.member")
                && raw.get_field::<OwnedUserId>("state_key").ok().flatten().as_deref() == own
                && raw
                    .get_field::<serde_json::Value>("content")
                    .ok()
                    .flatten()
                    .is_some_and(|c| c["membership"] == "invite");
            invite.then_some(NotifyKind::Invite)
        }
        RawAnySyncOrStrippedTimelineEvent::Sync(raw) if highlight => {
            let mentioned = raw.get_field::<serde_json::Value>("content").ok().flatten().is_some_and(|c| {
                c["m.mentions"]["user_ids"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == own.map(UserId::as_str)))
            });
            Some(if mentioned { NotifyKind::Mention } else { NotifyKind::Keyword })
        }
        RawAnySyncOrStrippedTimelineEvent::Sync(_) => Some(NotifyKind::Normal),
    }
}

/// Sends the room of every notifying event of someone else sent after this call. The SDK replays
/// history through the same handler (the first sync, rooms entering the list later), so older events
/// are dropped; the cutoff is fixed at the start, so a clock skew only costs the first seconds.
/// `should_notify` (mode and low priority of the room) comes first; invites carry no timestamp and
/// alert once per room and session, never for invites already pending at the start. Content never leaves.
/// The preview (text or media label) is built only after the push rules said this event notifies;
/// an undecrypted event carries none. It is never logged (no `Debug`).
pub(crate) async fn watch(client: &Client, account: matrix_domain::AccountId, alerts: UnboundedSender<RoomListEvent>) {
    let started = MilliSecondsSinceUnixEpoch::now();
    let own = client.user_id().map(ToOwned::to_owned);
    let invited: Arc<Mutex<HashSet<OwnedRoomId>>> =
        Arc::new(Mutex::new(client.invited_rooms().iter().map(|r| r.room_id().to_owned()).collect()));
    client
        .register_notification_handler(move |notification, room, _| {
            let (alerts, own, invited) = (alerts.clone(), own.clone(), invited.clone());
            async move {
                let Some(kind) = notify_kind(&notification.event, notification.actions.iter().any(|a| a.is_highlight()), own.as_deref())
                else {
                    return;
                };
                let mode = mode_from_sdk(room.cached_user_defined_notification_mode());
                if !should_notify(mode, room.is_low_priority(), kind) {
                    return;
                }
                let notify = match &notification.event {
                    RawAnySyncOrStrippedTimelineEvent::Sync(event) => {
                        event.get_field::<MilliSecondsSinceUnixEpoch>("origin_server_ts").ok().flatten().is_some_and(|ts| ts >= started)
                            && event.get_field::<OwnedUserId>("sender").ok().flatten() != own
                    }
                    RawAnySyncOrStrippedTimelineEvent::Stripped(_) => invited.lock().unwrap().insert(room.room_id().to_owned()),
                };
                if notify {
                    let scope = RoomScope { account_id: account, room_id: RoomId::new(room.room_id().as_str()) };
                    // An invite has no message to preview.
                    let preview = match &notification.event {
                        RawAnySyncOrStrippedTimelineEvent::Sync(event) => {
                            let sender = event.get_field::<OwnedUserId>("sender").ok().flatten();
                            let name = match &sender {
                                Some(id) => match room.get_member_no_sync(id).await {
                                    Ok(Some(member)) => Some(member.name().to_owned()),
                                    _ => Some(id.localpart().to_owned()),
                                },
                                None => None,
                            };
                            let direct = room.is_direct().await.unwrap_or(false);
                            Some(MessagePreview { content: content_of(event), sender: name, direct })
                        }
                        RawAnySyncOrStrippedTimelineEvent::Stripped(_) => None,
                    };
                    let _ = alerts.send(RoomListEvent::Alert(scope, preview));
                }
            }
        })
        .await;
}

/// What a decrypted-or-not event holds, from its raw JSON (a decrypted event keeps its real type;
/// one the SDK could not decrypt is still `m.room.encrypted`).
fn content_of(event: &matrix_sdk::ruma::serde::Raw<matrix_sdk::ruma::events::AnySyncTimelineEvent>) -> PreviewContent {
    let kind = event.get_field::<String>("type").ok().flatten().unwrap_or_default();
    match kind.as_str() {
        "m.room.encrypted" => PreviewContent::Undecrypted,
        "m.sticker" => PreviewContent::Sticker,
        "m.poll.start" | "org.matrix.msc3381.poll.start" => PreviewContent::Poll,
        "m.room.message" => {
            let content = event.get_field::<serde_json::Value>("content").ok().flatten().unwrap_or_default();
            // The sender's HTML (markdown arrives as HTML) goes through the room-row renderer, which drops
            // the reply fallback; no formatted body: the plain body.
            let body = || match content["formatted_body"].as_str().filter(|_| content["format"] == "org.matrix.custom.html") {
                Some(html) => crate::rich_text::from_html(html).0,
                None => content["body"].as_str().map(strip_reply_fallback).unwrap_or_default(),
            };
            match content["msgtype"].as_str() {
                Some("m.text" | "m.notice" | "m.emote") => PreviewContent::Text(body()),
                Some("m.image") => PreviewContent::Image,
                Some("m.video") => PreviewContent::Video,
                Some("m.audio") if content.get("org.matrix.msc3245.voice").is_some() => PreviewContent::Voice,
                Some("m.audio") => PreviewContent::Audio,
                Some("m.file") => PreviewContent::File,
                Some("m.location") => PreviewContent::Location,
                _ => PreviewContent::Other,
            }
        }
        _ => PreviewContent::Other,
    }
}

/// Drops the legacy «> quoted» reply fallback that precedes the real text.
fn strip_reply_fallback(body: &str) -> String {
    if !body.starts_with("> ") {
        return body.to_owned();
    }
    body.split_once("\n\n").map_or(String::new(), |(_, rest)| rest.to_owned())
}

/// Writes the room's notification mode as an account push rule (server side, shared by every device).
/// «Tutti i messaggi» is written too (no `unmute_room`), so it is exactly that and not the room default.
/// No compensating write on failure: the server stays the truth and its `m.push_rules` reach us on sync.
pub async fn set_notification_mode(
    client: &Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    mode: RoomNotifyMode,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    client.get_room(&id).ok_or(OperationError::Unavailable)?;
    client
        .notification_settings()
        .await
        .set_room_notification_mode(&id, mode_to_sdk(mode))
        .await
        .map_err(|_| OperationError::Transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::{serde::Raw, user_id};

    fn sync(content: &str) -> RawAnySyncOrStrippedTimelineEvent {
        let json = format!(r#"{{"type":"m.room.message","sender":"@b:x","event_id":"$e","origin_server_ts":1,"content":{content}}}"#);
        RawAnySyncOrStrippedTimelineEvent::Sync(Raw::from_json_string(json).unwrap())
    }
    fn stripped(state_key: &str, membership: &str) -> RawAnySyncOrStrippedTimelineEvent {
        let json = format!(r#"{{"type":"m.room.member","sender":"@b:x","state_key":"{state_key}","content":{{"membership":"{membership}"}}}}"#);
        RawAnySyncOrStrippedTimelineEvent::Stripped(Raw::from_json_string(json).unwrap())
    }

    #[test]
    fn mode_round_trips_through_the_sdk() {
        for mode in RoomNotifyMode::ALL {
            assert_eq!(mode_from_sdk(Some(mode_to_sdk(mode))), mode);
        }
        assert_eq!(mode_from_sdk(None), RoomNotifyMode::AllMessages);
    }

    #[test]
    fn kinds() {
        let me = Some(user_id!("@me:x"));
        let plain = r#"{"msgtype":"m.text","body":"hi"}"#;
        assert_eq!(notify_kind(&sync(plain), false, me), Some(NotifyKind::Normal));
        assert_eq!(notify_kind(&sync(plain), true, me), Some(NotifyKind::Keyword));
        let mention = r#"{"body":"hi","m.mentions":{"user_ids":["@me:x"]}}"#;
        assert_eq!(notify_kind(&sync(mention), true, me), Some(NotifyKind::Mention));
        let other = r#"{"body":"hi","m.mentions":{"user_ids":["@you:x"]}}"#;
        assert_eq!(notify_kind(&sync(other), true, me), Some(NotifyKind::Keyword));
        assert_eq!(notify_kind(&stripped("@me:x", "invite"), false, me), Some(NotifyKind::Invite));
        assert_eq!(notify_kind(&stripped("@you:x", "invite"), false, me), None);
        assert_eq!(notify_kind(&stripped("@me:x", "join"), false, me), None);
    }

    fn of(json: serde_json::Value) -> PreviewContent {
        content_of(&matrix_sdk::ruma::serde::Raw::new(&json).unwrap().cast_unchecked())
    }
    #[test]
    fn the_content_comes_from_the_event_type() {
        let msg = |c: serde_json::Value| serde_json::json!({"type": "m.room.message", "content": c});
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.text", "body": "ciao"}))), PreviewContent::Text(t) if t == "ciao"));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.text", "body": "> <@a:b> x\n\nrisposta"}))), PreviewContent::Text(t) if t == "risposta"));
        let html = |b: &str, f: &str| of(msg(serde_json::json!({"msgtype": "m.text", "body": b, "format": "org.matrix.custom.html", "formatted_body": f})));
        assert!(matches!(html("**ciao** _a_ [link](u)", "<strong>ciao</strong> <em>a</em> <a href=\"u\">link</a>"), PreviewContent::Text(t) if t == "ciao a link"));
        assert!(matches!(html("x\ny", "<b>x</b><br>y"), PreviewContent::Text(t) if t == "x\ny"));
        assert!(matches!(html("> <@a:b> q\n\nrisposta", "<mx-reply><blockquote>q</blockquote></mx-reply>risposta"), PreviewContent::Text(t) if t == "risposta"));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.image", "body": "a.png"}))), PreviewContent::Image));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.audio", "body": "v", "org.matrix.msc3245.voice": {}}))), PreviewContent::Voice));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.audio", "body": "v"}))), PreviewContent::Audio));
        assert!(matches!(of(serde_json::json!({"type": "m.room.encrypted", "content": {"ciphertext": "x"}})), PreviewContent::Undecrypted));
        assert!(matches!(of(serde_json::json!({"type": "m.room.member"})), PreviewContent::Other));
    }
}
