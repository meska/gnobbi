//! Durable, account/room-scoped sends. No view cancellation or payload logging.
use matrix_domain::{EventId, RoomScope, TransactionId};

#[derive(Clone)]
pub struct SendRequest {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    WrongScope,
    InvalidInput,
    Unavailable,
    /// SDK crypto/session policy failure; never fall back to plaintext.
    Encryption,
    Transport,
}

pub struct SendResult {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub result: Result<EventId, SendError>,
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    #[test]
    fn fresh_transaction_ids_do_not_repeat_across_sends() {
        let first = super::fresh_transaction_id();
        let second = super::fresh_transaction_id();
        assert!(!first.as_str().is_empty());
        assert_ne!(first, second);
    }
}

/// SDK generates random transaction identity; callers retain it for all retries.
#[cfg(feature = "sdk")]
pub fn fresh_transaction_id() -> TransactionId {
    TransactionId::new(matrix_sdk::ruma::TransactionId::new().as_str())
}

#[cfg(feature = "sdk")]
pub(crate) async fn send_text(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: &SendRequest,
) -> Result<EventId, SendError> {
    use matrix_sdk::ruma::{RoomId, events::room::message::RoomMessageEventContent};
    if request.scope.account_id != account {
        return Err(SendError::WrongScope);
    }
    if request.body.trim().is_empty() || request.transaction_id.as_str().is_empty() {
        return Err(SendError::InvalidInput);
    }
    let id = RoomId::parse(request.scope.room_id.as_str()).map_err(|_| SendError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(SendError::Unavailable)?;
    let transaction = request.transaction_id.as_str().into();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        room.send(RoomMessageEventContent::text_plain(&request.body))
            .with_transaction_id(transaction),
    )
    .await
    .map_err(|_| SendError::Transport)?
    .map_err(|error| match error {
        matrix_sdk::Error::NoOlmMachine
        | matrix_sdk::Error::CryptoStoreError(_)
        | matrix_sdk::Error::OlmError(_)
        | matrix_sdk::Error::MegolmError(_)
        | matrix_sdk::Error::DecryptorError(_)
        | matrix_sdk::Error::SignatureError(_) => SendError::Encryption,
        _ => SendError::Transport,
    })?;
    Ok(EventId::new(response.response.event_id.as_str()))
}
