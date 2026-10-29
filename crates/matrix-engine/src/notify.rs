//! Notifications through the SDK: push rules decide what notifies (`register_notification_handler`),
//! the per-room mute is the SDK push rule (`NotificationSettings`), written only on an explicit user action.
use crate::{RoomListEvent, operations::OperationError};
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
/// Content never leaves.
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
                    let _ = alerts.send(RoomListEvent::Alert(scope));
                }
            }
        })
        .await;
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
