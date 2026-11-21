//! Progressive reading: where the user has read up to, and a receipt on one event in the middle of a room.
use crate::operations::OperationError;
use matrix_domain::{EventId, RoomScope};
use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType as Stored};

fn room_of(client: &matrix_sdk::Client, account: matrix_domain::AccountId, scope: &RoomScope) -> Result<matrix_sdk::Room, OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    client.get_room(&id).filter(|r| r.state() == matrix_sdk::RoomState::Joined).ok_or(OperationError::Unavailable)
}

/// `m.fully_read`, the Timeline's newest own receipt (it counts own messages) and the stored own main/unthreaded receipts.
pub(crate) async fn markers(client: &matrix_sdk::Client, timelines: &crate::room_sync::Timelines, account: matrix_domain::AccountId, scope: &RoomScope) -> Result<Vec<EventId>, OperationError> {
    use matrix_sdk::ruma::events::fully_read::FullyReadEventContent;
    let room = room_of(client, account, scope)?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?.to_owned();
    let mut out: Vec<String> = Vec::new();
    if let Ok(Some(raw)) = room.account_data_static::<FullyReadEventContent>().await && let Ok(e) = raw.deserialize() {
        out.push(e.content.event_id.to_string());
    }
    for kind in [Stored::Read, Stored::ReadPrivate] {
        for thread in [ReceiptThread::Main, ReceiptThread::Unthreaded] {
            if let Ok(Some((id, _))) = room.load_user_receipt(kind.clone(), &thread, &me).await {
                out.push(id.to_string());
            }
        }
    }
    if let Ok(timeline) = timelines.get(&room).await && let Some(id) = timeline.latest_user_read_receipt_timeline_event_id(&me).await {
        out.push(id.to_string());
    }
    Ok(out.into_iter().map(EventId::new).collect())
}

/// Receipt `main` on `target` (public or private) and `m.fully_read`; the SDK Timeline skips a fully_read that would not move forward.
pub(crate) async fn advance(client: &matrix_sdk::Client, timelines: &crate::room_sync::Timelines, account: matrix_domain::AccountId, scope: &RoomScope, target: &EventId, public: bool) -> Result<bool, OperationError> {
    use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
    let room = room_of(client, account, scope)?;
    let id = matrix_sdk::ruma::EventId::parse(target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    let me = client.user_id().ok_or(OperationError::Unavailable)?.to_owned();
    let (kind, stored) = if public { (ReceiptType::Read, Stored::Read) } else { (ReceiptType::ReadPrivate, Stored::ReadPrivate) };
    let have = room.load_user_receipt(stored, &ReceiptThread::Main, &me).await.ok().flatten();
    let sent = have.is_none_or(|(have, _)| have != id);
    if sent {
        room.send_single_receipt(kind, ReceiptThread::Main, id.clone()).await.map_err(|_| OperationError::Transport)?;
    }
    let timeline = timelines.get(&room).await.map_err(|_| OperationError::Unavailable)?;
    let _ = timeline.send_single_receipt(ReceiptType::FullyRead, id).await;
    Ok(sent)
}
