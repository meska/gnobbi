//! Scoped message operations. Payloads intentionally do not implement Debug.
use matrix_domain::{EventId, RoomScope, TransactionId};

#[derive(Clone, PartialEq, Eq)]
pub enum Operation {
    Reply { body: String },
    Edit { body: String },
    React { key: String },
    Redact,
}
#[derive(Clone, PartialEq, Eq)]
pub struct OperationRequest {
    pub scope: RoomScope,
    pub transaction_id: TransactionId,
    pub target: EventId,
    pub operation: Operation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationError {
    WrongScope,
    InvalidInput,
    Unavailable,
    Forbidden,
    Transport,
}

pub enum OperationCommand {
    Inspect {
        tag: crate::RequestTag,
        target: EventId,
    },
    Apply(OperationRequest),
    /// Joined members for the @ completion.
    Members {
        scope: RoomScope,
    },
    /// Room-level read marker from the room menu; `public` is the «Conferme di lettura» setting.
    MarkRoom {
        scope: RoomScope,
        read: bool,
        public: bool,
    },
    /// New direct chat with `user_id` (reuses an existing one).
    StartDm {
        user_id: String,
    },
    /// New room; `public` joins by anyone and lists it in the directory.
    CreateRoom {
        name: String,
        public: bool,
    },
}
pub enum OperationResult {
    RoomStarted {
        result: Result<RoomScope, OperationError>,
    },
    Members {
        scope: RoomScope,
        members: Result<Vec<Member>, OperationError>,
    },
    RoomMarked {
        scope: RoomScope,
        result: Result<(), OperationError>,
    },
    Inspected {
        tag: crate::RequestTag,
        target: EventId,
        details: Result<MessageDetails, OperationError>,
    },
    Applied {
        request: OperationRequest,
        result: Result<EventId, OperationError>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    /// Display name, or the user id when the member has none.
    pub name: String,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct MessagePermissions {
    pub reply: bool,
    pub edit: bool,
    pub redact: bool,
    pub react: bool,
}
pub struct MessageDetails {
    pub body: String,
    pub permissions: MessagePermissions,
}

#[cfg(feature = "sdk")]
pub(crate) async fn details(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    target: &EventId,
) -> Result<MessageDetails, OperationError> {
    use matrix_sdk::ruma::{
        EventId as MatrixEventId, RoomId,
        events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, MessageLikeEventType,
            SyncMessageLikeEvent,
        },
    };
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = RoomId::parse(scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    let target = MatrixEventId::parse(target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let event = room
            .event(&target, None)
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(message),
        ))) = event.raw().deserialize()
        else {
            return Err(OperationError::Unavailable);
        };
        let user = client.user_id().ok_or(OperationError::Unavailable)?;
        let levels = room
            .power_levels()
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let own = user == message.sender;
        let text = matches!(
            message.content.msgtype,
            matrix_sdk::ruma::events::room::message::MessageType::Text(_)
        );
        let send = levels.user_can_send_message(user, MessageLikeEventType::RoomMessage);
        Ok(MessageDetails {
            body: message.content.body().to_owned(),
            permissions: MessagePermissions {
                reply: send,
                edit: own && text && send,
                redact: if own {
                    levels.user_can_redact_own_event(user)
                } else {
                    levels.user_can_redact_event_of_other(user)
                },
                react: levels.user_can_send_message(user, MessageLikeEventType::Reaction),
            },
        })
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// Read: receipts for the newest stored event, then the SDK unread flag off. Unread: the SDK unread flag on.
#[cfg(feature = "sdk")]
pub(crate) async fn mark_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    read: bool,
    public: bool,
) -> Result<(), OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    if read {
        // shortcut: only events already in the event cache; a never-opened room just loses its flag.
        let (rows, _, _) = crate::room_sync::stored_events(client, &id)
            .await
            .map_err(|_| OperationError::Unavailable)?;
        if let Some(event) = rows.last().and_then(|e| e.event_id()) {
            room.send_multiple_receipts(crate::receipts::marker_receipts(event.to_owned(), public))
                .await
                .map_err(|_| OperationError::Transport)?;
        }
    }
    room.set_unread_flag(!read)
        .await
        .map_err(|_| OperationError::Transport)
}

/// Direct chat: the SDK writes `m.direct`; an existing DM with the user is returned instead of a duplicate.
#[cfg(feature = "sdk")]
pub async fn start_dm(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    user_id: &str,
) -> Result<RoomScope, OperationError> {
    let user = matrix_sdk::ruma::UserId::parse(user_id.trim())
        .map_err(|_| OperationError::InvalidInput)?;
    let room = match client.get_dm_room(&user) {
        Some(room) => room,
        None => timeout_create(client.create_dm(&user)).await?,
    };
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

#[cfg(feature = "sdk")]
pub async fn create_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    name: &str,
    public: bool,
) -> Result<RoomScope, OperationError> {
    use matrix_sdk::ruma::api::client::room::{
        Visibility,
        create_room::v3::{Request, RoomPreset},
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let mut request = Request::new();
    request.name = Some(name.to_owned());
    (request.preset, request.visibility) = if public {
        (Some(RoomPreset::PublicChat), Visibility::Public)
    } else {
        (Some(RoomPreset::PrivateChat), Visibility::Private)
    };
    let room = timeout_create(client.create_room(request)).await?;
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

#[cfg(feature = "sdk")]
async fn timeout_create(
    create: impl std::future::Future<Output = matrix_sdk::Result<matrix_sdk::Room>>,
) -> Result<matrix_sdk::Room, OperationError> {
    tokio::time::timeout(std::time::Duration::from_secs(20), create)
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|_| OperationError::Transport)
}

#[cfg(feature = "sdk")]
pub async fn members(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
) -> Result<Vec<Member>, OperationError> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    // Lazy loading leaves the list partial until the full membership is fetched.
    let joined = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        room.members(matrix_sdk::RoomMemberships::JOIN),
    )
    .await
    .map_err(|_| OperationError::Transport)?
    .map_err(|_| OperationError::Transport)?;
    Ok(joined
        .into_iter()
        .map(|m| Member {
            user_id: m.user_id().to_string(),
            name: m
                .display_name()
                .map_or_else(|| m.user_id().to_string(), str::to_owned),
        })
        .collect())
}

#[cfg(feature = "sdk")]
#[derive(Default)]
pub(crate) struct OperationJournal {
    requests: Vec<(OperationRequest, Option<EventId>)>,
}

#[cfg(feature = "sdk")]
pub(crate) async fn perform(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    state: &tokio::sync::Mutex<OperationJournal>,
    request: &OperationRequest,
) -> Result<EventId, OperationError> {
    if request.scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let mut state = state.lock().await;
    let existing = state
        .requests
        .iter()
        .position(|(original, _)| original.transaction_id == request.transaction_id);
    let index = if let Some(index) = existing {
        let (original, event) = &state.requests[index];
        if original != request {
            return Err(OperationError::InvalidInput);
        }
        if let Some(event) = event {
            return Ok(event.clone());
        }
        if matches!(request.operation, Operation::Redact) {
            let event = recover_redaction(client, request).await?;
            state.requests[index].1 = Some(event.clone());
            return Ok(event);
        }
        Some(index)
    } else if matches!(
        request.operation,
        Operation::React { .. } | Operation::Redact
    ) {
        if request.transaction_id.as_str().is_empty()
            || matches!(&request.operation, Operation::React { key } if key.trim().is_empty() || key.len() > 32768)
        {
            return Err(OperationError::InvalidInput);
        }
        // shortcut: 128 reaction/redaction intents per session; use a durable journal before lifting this cap.
        if state.requests.len() >= 128 {
            return Err(OperationError::Unavailable);
        }
        state.requests.push((request.clone(), None));
        Some(state.requests.len() - 1)
    } else {
        None
    };
    let event = perform_uncached(client, account, request).await?;
    if let Some(index) = index {
        state.requests[index].1 = Some(event.clone());
    }
    Ok(event)
}

#[cfg(feature = "sdk")]
async fn perform_uncached(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    request: &OperationRequest,
) -> Result<EventId, OperationError> {
    use matrix_sdk::ruma::{EventId as MatrixEventId, RoomId};
    if request.scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let room_id =
        RoomId::parse(request.scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    let target =
        MatrixEventId::parse(request.target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    if request.transaction_id.as_str().is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let room = client
        .get_room(&room_id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    if matches!(request.operation, Operation::Redact) {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            room.redact(&target, None, Some(request.transaction_id.as_str().into())),
        )
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|error| {
            if matches!(
                error.client_api_error_kind(),
                Some(matrix_sdk::ruma::api::error::ErrorKind::Forbidden)
            ) {
                OperationError::Forbidden
            } else {
                OperationError::Transport
            }
        })?;
        return Ok(EventId::new(response.event_id.as_str()));
    }
    let body = match &request.operation {
        Operation::Reply { body } | Operation::Edit { body } => body,
        Operation::React { key } => key,
        Operation::Redact => unreachable!(),
    };
    if body.trim().is_empty() || body.len() > 32768 {
        return Err(OperationError::InvalidInput);
    }
    if matches!(request.operation, Operation::Edit { .. }) {
        use matrix_sdk::ruma::events::{
            AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
        };
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(5), room.event(&target, None))
                .await
                .map_err(|_| OperationError::Transport)?
                .map_err(|_| OperationError::Unavailable)?;
        let Ok(AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncMessageLikeEvent::Original(original),
        ))) = event.raw().deserialize()
        else {
            return Err(OperationError::Unavailable);
        };
        if client.user_id() != Some(original.sender.as_ref()) {
            return Err(OperationError::Forbidden);
        }
    }
    let content = match &request.operation {
        Operation::Redact => unreachable!(),
        Operation::React { key } => serde_json::json!({
            "m.relates_to": {"rel_type":"m.annotation", "event_id":target, "key":key}
        }),
        Operation::Reply { .. } => serde_json::json!({
            "msgtype": "m.text", "body": body,
            "m.relates_to": {"m.in_reply_to": {"event_id": target}}
        }),
        Operation::Edit { .. } => serde_json::json!({
            "msgtype": "m.text", "body": format!("* {body}"),
            "m.new_content": {"msgtype":"m.text", "body": body},
            "m.relates_to": {"rel_type":"m.replace", "event_id": target}
        }),
    };
    let event_type = if matches!(request.operation, Operation::React { .. }) {
        "m.reaction"
    } else {
        "m.room.message"
    };
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        room.send_raw(event_type, content)
            .with_transaction_id(request.transaction_id.as_str().into()),
    )
    .await
    .map_err(|_| OperationError::Transport)?;
    match response {
        Ok(response) => Ok(EventId::new(response.response.event_id.as_str())),
        Err(error)
            if matches!(request.operation, Operation::React { .. })
                && matches!(
                    error.client_api_error_kind(),
                    Some(matrix_sdk::ruma::api::error::ErrorKind::DuplicateAnnotation)
                ) =>
        {
            recover_reaction(client, &room, request).await
        }
        Err(_) => Err(OperationError::Transport),
    }
}

#[cfg(feature = "sdk")]
async fn recover_redaction(
    client: &matrix_sdk::Client,
    request: &OperationRequest,
) -> Result<EventId, OperationError> {
    use matrix_sdk::ruma::{EventId as MatrixEventId, RoomId};
    let room_id =
        RoomId::parse(request.scope.room_id.as_str()).map_err(|_| OperationError::WrongScope)?;
    let target =
        MatrixEventId::parse(request.target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    let room = client
        .get_room(&room_id)
        .filter(|room| room.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    let user = client.user_id().ok_or(OperationError::Unavailable)?;
    // A deleted row is not proof of this request; an uncertain retry never writes again.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let original = room
            .event(&target, None)
            .await
            .map_err(|_| OperationError::Transport)?;
        let original: serde_json::Value = serde_json::from_str(original.raw().json().get())
            .map_err(|_| OperationError::Transport)?;
        let id = original["unsigned"]["redacted_because"]["event_id"]
            .as_str()
            .ok_or(OperationError::Transport)?;
        let id = MatrixEventId::parse(id).map_err(|_| OperationError::Transport)?;
        let event = room
            .event(&id, None)
            .await
            .map_err(|_| OperationError::Transport)?;
        let event: serde_json::Value = serde_json::from_str(event.raw().json().get())
            .map_err(|_| OperationError::Transport)?;
        let redacts = event["content"].get("redacts").unwrap_or(&event["redacts"]);
        if original["event_id"].as_str() != Some(target.as_str())
            || event["event_id"].as_str() != Some(id.as_str())
            || event["type"] != "m.room.redaction"
            || event["sender"].as_str() != Some(user.as_str())
            || event["room_id"]
                .as_str()
                .is_some_and(|id| id != room_id.as_str())
            || redacts.as_str() != Some(target.as_str())
            || event["unsigned"]["transaction_id"].as_str() != Some(request.transaction_id.as_str())
        {
            return Err(OperationError::Transport);
        }
        Ok(EventId::new(id.as_str()))
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

#[cfg(feature = "sdk")]
async fn recover_reaction(
    client: &matrix_sdk::Client,
    room: &matrix_sdk::Room,
    request: &OperationRequest,
) -> Result<EventId, OperationError> {
    use matrix_sdk::{
        room::{IncludeRelations, RelationsOptions},
        ruma::events::relation::RelationType,
    };
    let Operation::React { key } = &request.operation else {
        return Err(OperationError::InvalidInput);
    };
    let target = matrix_sdk::ruma::EventId::parse(request.target.as_str())
        .map_err(|_| OperationError::InvalidInput)?;
    let user = client.user_id().ok_or(OperationError::Unavailable)?;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut from = None;
        let mut cursors = std::collections::BTreeSet::new();
        let mut found = None;
        loop {
            let page = room
                .relations(
                    target.clone(),
                    RelationsOptions {
                        from,
                        limit: Some(25u32.into()),
                        include_relations: IncludeRelations::RelationsOfType(
                            RelationType::Annotation,
                        ),
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| OperationError::Transport)?;
            for event in page.chunk {
                let value: serde_json::Value = serde_json::from_str(event.raw().json().get())
                    .map_err(|_| OperationError::Transport)?;
                if value["sender"].as_str() != Some(user.as_str())
                    || value["unsigned"]["transaction_id"].as_str()
                        != Some(request.transaction_id.as_str())
                {
                    continue;
                }
                let relation = &value["content"]["m.relates_to"];
                if value["room_id"]
                    .as_str()
                    .is_some_and(|id| id != request.scope.room_id.as_str())
                    || value["type"] != "m.reaction"
                    || relation["rel_type"] != "m.annotation"
                    || relation["event_id"].as_str() != Some(request.target.as_str())
                    || relation["key"].as_str() != Some(key.as_str())
                {
                    return Err(OperationError::InvalidInput);
                }
                let id = value["event_id"]
                    .as_str()
                    .ok_or(OperationError::Transport)?;
                matrix_sdk::ruma::EventId::parse(id).map_err(|_| OperationError::Transport)?;
                if found
                    .as_ref()
                    .is_some_and(|previous: &EventId| previous.as_str() != id)
                {
                    return Err(OperationError::Transport);
                }
                found = Some(EventId::new(id));
            }
            let Some(next) = page.next_batch_token else {
                break;
            };
            if !cursors.insert(next.clone()) {
                return Err(OperationError::Transport);
            }
            from = Some(next);
        }
        found.ok_or(OperationError::Transport)
    })
    .await
    .map_err(|_| OperationError::Transport)?
}
#[cfg(all(test, feature = "sdk"))]
mod tests {
    use super::*;
    #[test]
    fn full_reaction_journal_keeps_confirmed_identity_without_accepting_new_writes() {
        use matrix_domain::{AccountId, RoomId};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let client = matrix_sdk::Client::builder()
                .homeserver_url("https://example.invalid")
                .server_versions(vec![matrix_sdk::ruma::api::MatrixVersion::V1_0])
                .build()
                .await
                .unwrap();
            let request = OperationRequest {
                scope: RoomScope {
                    account_id: AccountId::new(52),
                    room_id: RoomId::new("!room:example.invalid"),
                },
                transaction_id: TransactionId::new("0"),
                target: EventId::new("$target"),
                operation: Operation::React { key: "👍".into() },
            };
            let state = tokio::sync::Mutex::new(OperationJournal {
                requests: (0..128)
                    .map(|n| {
                        (
                            OperationRequest {
                                transaction_id: TransactionId::new(n.to_string()),
                                ..request.clone()
                            },
                            Some(EventId::new("$confirmed")),
                        )
                    })
                    .collect(),
            });
            assert_eq!(
                perform(&client, AccountId::new(52), &state, &request).await,
                Ok(EventId::new("$confirmed"))
            );
            let new = OperationRequest {
                transaction_id: TransactionId::new("new"),
                ..request
            };
            assert_eq!(
                perform(&client, AccountId::new(52), &state, &new).await,
                Err(OperationError::Unavailable)
            );
            assert_eq!(state.lock().await.requests.len(), 128);
        });
    }
}
