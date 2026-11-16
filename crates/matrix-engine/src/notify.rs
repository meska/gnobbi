//! Notifications through the SDK: push rules decide what notifies (`register_notification_handler`),
//! the per-room mute is the SDK push rule (`NotificationSettings`), written only on an explicit user action.
use crate::{RoomListEvent, operations::OperationError};
use matrix_domain::notifications::{MessagePreview, PreviewContent};
use matrix_domain::{RoomId, RoomScope};
use matrix_sdk::{
    Client, deserialized_responses::RawAnySyncOrStrippedTimelineEvent,
    notification_settings::{IsEncrypted, IsOneToOne, RoomNotificationMode},
    ruma::{MilliSecondsSinceUnixEpoch, OwnedUserId},
};
use tokio::sync::mpsc::UnboundedSender;

/// Sends the room of every notifying event of someone else sent after this call. The SDK replays
/// history through the same handler (the first sync, rooms entering the list later), so older events
/// are dropped; the cutoff is fixed at the start, so a clock skew only costs the first seconds.
/// The preview (text or media label) is built only after the push rules said this event notifies;
/// an undecrypted event carries none. It is never logged (no `Debug`).
pub(crate) async fn watch(client: &Client, account: matrix_domain::AccountId, alerts: UnboundedSender<RoomListEvent>) {
    let started = MilliSecondsSinceUnixEpoch::now();
    let own = client.user_id().map(ToOwned::to_owned);
    client
        .register_notification_handler(move |notification, room, _| {
            let (alerts, own) = (alerts.clone(), own.clone());
            async move {
                let RawAnySyncOrStrippedTimelineEvent::Sync(event) = &notification.event else { return };
                let fresh = event
                    .get_field::<MilliSecondsSinceUnixEpoch>("origin_server_ts")
                    .ok()
                    .flatten()
                    .is_some_and(|ts| ts >= started);
                if fresh && event.get_field::<OwnedUserId>("sender").ok().flatten() != own {
                    let scope = RoomScope { account_id: account, room_id: RoomId::new(room.room_id().as_str()) };
                    let sender = event.get_field::<OwnedUserId>("sender").ok().flatten();
                    let name = match &sender {
                        Some(id) => match room.get_member_no_sync(id).await {
                            Ok(Some(member)) => Some(member.name().to_owned()),
                            _ => Some(id.localpart().to_owned()),
                        },
                        None => None,
                    };
                    let direct = room.is_direct().await.unwrap_or(false);
                    let preview = MessagePreview { content: content_of(event), sender: name, direct };
                    let _ = alerts.send(RoomListEvent::Alert(scope, Some(preview)));
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
            let body = || content["body"].as_str().map(strip_reply_fallback).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;
    fn of(json: serde_json::Value) -> PreviewContent {
        content_of(&matrix_sdk::ruma::serde::Raw::new(&json).unwrap().cast_unchecked())
    }
    #[test]
    fn the_content_comes_from_the_event_type() {
        let msg = |c: serde_json::Value| serde_json::json!({"type": "m.room.message", "content": c});
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.text", "body": "ciao"}))), PreviewContent::Text(t) if t == "ciao"));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.text", "body": "> <@a:b> x\n\nrisposta"}))), PreviewContent::Text(t) if t == "risposta"));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.image", "body": "a.png"}))), PreviewContent::Image));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.audio", "body": "v", "org.matrix.msc3245.voice": {}}))), PreviewContent::Voice));
        assert!(matches!(of(msg(serde_json::json!({"msgtype": "m.audio", "body": "v"}))), PreviewContent::Audio));
        assert!(matches!(of(serde_json::json!({"type": "m.room.encrypted", "content": {"ciphertext": "x"}})), PreviewContent::Undecrypted));
        assert!(matches!(of(serde_json::json!({"type": "m.room.member"})), PreviewContent::Other));
    }
}

/// Mutes or unmutes `scope` with the account's push rules (server side, shared by every device).
pub async fn set_muted(
    client: &Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    muted: bool,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    let room = client.get_room(&id).ok_or(OperationError::Unavailable)?;
    let settings = client.notification_settings().await;
    if muted {
        settings.set_room_notification_mode(&id, RoomNotificationMode::Mute).await
    } else {
        let encrypted = IsEncrypted::from(room.encryption_state().is_encrypted());
        settings.unmute_room(&id, encrypted, IsOneToOne::from(room.active_members_count() == 2)).await
    }
    .map_err(|_| OperationError::Transport)
}
