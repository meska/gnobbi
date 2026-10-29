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
        topic: Option<String>,
        public: bool,
    },
    /// Group (room tag) change asked for by the user in «Modifica»; the only writer of tags.
    EditTags(TagEdit),
    /// «Silenzia»: the room push rule; the outcome comes back as `RoomMarked` and the next sync projects it.
    SetMuted {
        scope: RoomScope,
        muted: bool,
    },
}
/// One explicit change of the personal groups. `tag` is the full tag name (`m.favourite`,
/// `m.lowpriority` or `u.<name>`); a group exists as long as one room carries its tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEdit {
    Assign { room_id: String, tag: String, on: bool },
    /// Moves every room of user group `from` to `to` (both `u.*`).
    Rename { from: String, to: String },
    /// Removes user group `tag` from every room.
    Delete { tag: String },
}
pub enum OperationResult {
    TagsEdited {
        result: Result<(), OperationError>,
    },
    RoomStarted {
        result: Result<RoomScope, OperationError>,
    },
    Members {
        scope: RoomScope,
        members: Result<Vec<Member>, OperationError>,
        /// The server's upload limit in bytes (media config); `None` when it could not be read.
        upload_limit: Option<u64>,
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
const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// The room's Timeline (the one the history uses) and the message item for `target`; pages back a
/// little when the message is not loaded yet.
#[cfg(feature = "sdk")]
async fn locate(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    target: &matrix_sdk::ruma::EventId,
    page: bool,
) -> Result<
    (
        matrix_sdk::Room,
        std::sync::Arc<matrix_sdk_ui::timeline::Timeline>,
        Option<matrix_sdk_ui::timeline::EventTimelineItem>,
    ),
    OperationError,
> {
    if scope.account_id != account {
        return Err(OperationError::WrongScope);
    }
    let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str())
        .map_err(|_| OperationError::WrongScope)?;
    let room = client
        .get_room(&id)
        .filter(|r| r.state() == matrix_sdk::RoomState::Joined)
        .ok_or(OperationError::Unavailable)?;
    let timeline = timelines
        .get(&room)
        .await
        .map_err(|_| OperationError::Unavailable)?;
    let mut item = timeline.item_by_event_id(target).await;
    // shortcut: about 3 s of polling (sync or pagination apply in the timeline's own task), one
    // page of 25 back every fifth step; then the message counts as not found.
    for step in 0..20 {
        if item.is_some() || !page {
            break;
        }
        if step % 5 == 4 {
            let _ = timeline.paginate_backwards(25).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        item = timeline.item_by_event_id(target).await;
    }
    Ok((room, timeline, item))
}

#[cfg(feature = "sdk")]
pub(crate) async fn details(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    scope: &RoomScope,
    target: &EventId,
) -> Result<MessageDetails, OperationError> {
    use matrix_sdk::ruma::events::MessageLikeEventType;
    let target = matrix_sdk::ruma::EventId::parse(target.as_str())
        .map_err(|_| OperationError::InvalidInput)?;
    tokio::time::timeout(WAIT, async {
        let (room, _, item) = locate(client, timelines, account, scope, &target, true).await?;
        let item = item.ok_or(OperationError::Unavailable)?;
        let message = item
            .content()
            .as_message()
            .ok_or(OperationError::Unavailable)?;
        let user = client.user_id().ok_or(OperationError::Unavailable)?;
        // The SDK has no permission check: power levels stay ours.
        let levels = room
            .power_levels()
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let send = levels.user_can_send_message(user, MessageLikeEventType::RoomMessage);
        Ok(MessageDetails {
            body: message.body().to_owned(),
            permissions: MessagePermissions {
                reply: send && item.can_be_replied_to(),
                edit: send && item.is_editable(),
                redact: if item.is_own() {
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
    timelines: &crate::room_sync::Timelines,
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
        use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
        let timeline = timelines
            .get(&room)
            .await
            .map_err(|_| OperationError::Unavailable)?;
        let kind = if public { ReceiptType::Read } else { ReceiptType::ReadPrivate };
        timeline.mark_as_read(kind).await.map_err(|_| OperationError::Transport)?;
        timeline
            .mark_as_read(ReceiptType::FullyRead)
            .await
            .map_err(|_| OperationError::Transport)?;
    }
    room.set_unread_flag(!read)
        .await
        .map_err(|_| OperationError::Transport)
}

/// Direct chat: the SDK writes `m.direct`; an existing DM with the user is returned instead of a duplicate.
/// A user the server does not know stops here (`Unavailable`) so no orphan room is created.
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
        None => timeout_create(async {
            // Only NotFound stops: other profile failures (private profiles, network) fall through to create_dm.
            if let Err(e) = client.account().fetch_user_profile_of(&user).await
                && is_not_found(&e)
            {
                return Err(e);
            }
            client.create_dm(&user).await
        })
        .await?,
    };
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

/// Private rooms are born encrypted (like the SDK's DMs); public ones are not.
#[cfg(feature = "sdk")]
pub async fn create_room(
    client: &matrix_sdk::Client,
    account: matrix_domain::AccountId,
    name: &str,
    topic: Option<&str>,
    public: bool,
) -> Result<RoomScope, OperationError> {
    use matrix_sdk::ruma::{
        api::client::room::{
            Visibility,
            create_room::v3::{Request, RoomPreset},
        },
        events::{InitialStateEvent, room::encryption::RoomEncryptionEventContent},
    };
    let name = name.trim();
    if name.is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let mut request = Request::new();
    request.name = Some(name.to_owned());
    request.topic = topic.map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned);
    (request.preset, request.visibility) = if public {
        (Some(RoomPreset::PublicChat), Visibility::Public)
    } else {
        request.initial_state = vec![
            InitialStateEvent::with_empty_state_key(
                RoomEncryptionEventContent::with_recommended_defaults(),
            )
            .to_raw_any(),
        ];
        (Some(RoomPreset::PrivateChat), Visibility::Private)
    };
    let room = timeout_create(client.create_room(request)).await?;
    Ok(RoomScope {
        account_id: account,
        room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
    })
}

#[cfg(feature = "sdk")]
use matrix_sdk::ruma::api::error::ErrorKind;

/// Synapse answers 404 `M_UNKNOWN` ("No row found") for a missing profile, Tuwunel `M_NOT_FOUND`.
#[cfg(feature = "sdk")]
fn is_not_found(error: &matrix_sdk::Error) -> bool {
    matches!(error.client_api_error_kind(), Some(ErrorKind::NotFound))
        || error.as_client_api_error().is_some_and(|e| e.status_code == 404)
}

/// Server answer by Matrix error code; no client API error means the network failed.
#[cfg(feature = "sdk")]
fn classify(error: &matrix_sdk::Error) -> OperationError {
    if is_not_found(error) {
        OperationError::Unavailable
    } else if matches!(error.client_api_error_kind(), Some(ErrorKind::Forbidden)) {
        OperationError::Forbidden
    } else {
        OperationError::Transport
    }
}

#[cfg(feature = "sdk")]
async fn timeout_create(
    create: impl std::future::Future<Output = matrix_sdk::Result<matrix_sdk::Room>>,
) -> Result<matrix_sdk::Room, OperationError> {
    tokio::time::timeout(std::time::Duration::from_secs(20), create)
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|e| classify(&e))
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
    /// Request, its confirmed event, and the transaction ID the SDK send queue gave a redaction.
    requests: Vec<(
        OperationRequest,
        Option<EventId>,
        Option<matrix_sdk::ruma::OwnedTransactionId>,
    )>,
}

#[cfg(feature = "sdk")]
pub(crate) async fn perform(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
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
        .position(|(original, ..)| original.transaction_id == request.transaction_id);
    let mut retry = false;
    let index = if let Some(index) = existing {
        let (original, event, _) = &state.requests[index];
        if original != request {
            return Err(OperationError::InvalidInput);
        }
        if let Some(event) = event {
            return Ok(event.clone());
        }
        retry = true;
        index
    } else {
        if request.transaction_id.as_str().is_empty()
            || matches!(&request.operation, Operation::React { key } if key.trim().is_empty() || key.len() > 32768)
        {
            return Err(OperationError::InvalidInput);
        }
        // shortcut: 128 unconfirmed-or-kept intents per session; replies and edits evict a confirmed
        // one instead of refusing. Use a durable journal (P283a) before lifting this cap.
        if state.requests.len() >= 128 {
            let confirmed = state.requests.iter().position(|(_, e, _)| e.is_some());
            match (&request.operation, confirmed) {
                (Operation::Reply { .. } | Operation::Edit { .. }, Some(old)) => {
                    state.requests.remove(old);
                }
                _ => return Err(OperationError::Unavailable),
            }
        }
        state.requests.push((request.clone(), None, None));
        state.requests.len() - 1
    };
    let mut queued = state.requests[index].2.clone();
    let event = perform_uncached(client, timelines, account, request, retry, &mut queued).await;
    state.requests[index].2 = queued;
    let event = event?;
    state.requests[index].1 = Some(event.clone());
    Ok(event)
}

/// The id of the next event the send queue reports as sent that satisfies `want` (read back from
/// the server: the queue chooses its own transaction ids).
#[cfg(feature = "sdk")]
async fn sent_event(
    updates: &mut tokio::sync::broadcast::Receiver<matrix_sdk::send_queue::SendQueueUpdate>,
    room: &matrix_sdk::Room,
    want: impl Fn(&serde_json::Value) -> bool,
) -> Result<EventId, OperationError> {
    use matrix_sdk::send_queue::RoomSendQueueUpdate::SentEvent;
    use tokio::sync::broadcast::error::RecvError;
    tokio::time::timeout(WAIT, async {
        loop {
            let update = match updates.recv().await {
                Ok(update) => update,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return Err(OperationError::Transport),
            };
            let SentEvent { event_id, .. } = update.update else {
                continue;
            };
            if update.room_id != room.room_id() {
                continue;
            }
            let Ok(event) = room.event(&event_id, None).await else {
                continue;
            };
            let event: serde_json::Value = serde_json::from_str(event.raw().json().get())
                .map_err(|_| OperationError::Transport)?;
            if want(&event) {
                return Ok(EventId::new(event_id.as_str()));
            }
        }
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

#[cfg(feature = "sdk")]
async fn perform_uncached(
    client: &matrix_sdk::Client,
    timelines: &crate::room_sync::Timelines,
    account: matrix_domain::AccountId,
    request: &OperationRequest,
    retry: bool,
    queued: &mut Option<matrix_sdk::ruma::OwnedTransactionId>,
) -> Result<EventId, OperationError> {
    use matrix_sdk::{room::edit::EditedContent, ruma::EventId as MatrixEventId};
    let target =
        MatrixEventId::parse(request.target.as_str()).map_err(|_| OperationError::InvalidInput)?;
    if request.transaction_id.as_str().is_empty() {
        return Err(OperationError::InvalidInput);
    }
    let text = match &request.operation {
        Operation::Reply { body } | Operation::Edit { body } => Some(body),
        Operation::React { key } => Some(key),
        Operation::Redact => None,
    };
    if text.is_some_and(|text| text.trim().is_empty() || text.len() > 32768) {
        return Err(OperationError::InvalidInput);
    }
    let user = client.user_id().ok_or(OperationError::Unavailable)?;
    let redact = matches!(request.operation, Operation::Redact);
    let (room, timeline, mut item) =
        locate(client, timelines, account, &request.scope, &target, !redact).await?;
    // A reaction is no Timeline item: look further only for a message.
    if redact && item.is_none() && !is_reaction(&room, &target).await {
        item = locate(client, timelines, account, &request.scope, &target, true)
            .await?
            .2;
        if item.is_none() {
            return Err(OperationError::Unavailable);
        }
    }
    let relation = |e: &serde_json::Value| e["content"]["m.relates_to"].clone();
    match &request.operation {
        Operation::Redact => {
            let Some(item) = item else {
                return redact_reaction(&room, user, request, retry).await;
            };
            // The SDK has no permission check: power levels stay ours.
            let levels = room
                .power_levels()
                .await
                .map_err(|_| OperationError::Unavailable)?;
            let allowed = if item.is_own() {
                levels.user_can_redact_own_event(user)
            } else {
                levels.user_can_redact_event_of_other(user)
            };
            if !allowed {
                return Err(OperationError::Forbidden);
            }
            // An uncertain retry never queues a second redaction: it only looks for the first.
            if !retry {
                use matrix_sdk::send_queue::{
                    LocalEcho, LocalEchoContent, RoomSendQueueUpdate::NewLocalEvent,
                };
                let mut updates = client.send_queue().subscribe();
                timeline
                    .redact(&item.identifier(), None)
                    .await
                    .map_err(|_| OperationError::Transport)?;
                // The SDK picks the transaction ID of the queued redaction; it is the proof later.
                while let Ok(update) = updates.try_recv() {
                    if let NewLocalEvent(LocalEcho {
                        transaction_id,
                        content: LocalEchoContent::Redaction { redacts, .. },
                    }) = update.update
                        && redacts == target
                    {
                        *queued = Some(transaction_id);
                    }
                }
            }
            // Only a retry needs the proof: the first attempt queued this very redaction. A server
            // that omits `unsigned.transaction_id` never confirms a retry (it stays Transport).
            let proof = queued.as_ref().filter(|_| retry).map(|t| t.as_str());
            tokio::time::timeout(WAIT, async {
                loop {
                    if let Ok(id) = redaction_of(&room, user, &target, proof).await {
                        return id;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                }
            })
            .await
            .map_err(|_| OperationError::Transport)
        }
        Operation::React { key } => {
            let item = item.ok_or(OperationError::Unavailable)?;
            let reacted = item
                .content()
                .reactions()
                .is_some_and(|r| r.get(key.as_str()).is_some_and(|by| by.contains_key(user)));
            if retry {
                // The SDK shows no event ID for a reaction it already confirmed.
                if let Ok(id) = own_reaction(&room, user, &target, key).await {
                    return Ok(id);
                }
            } else if reacted {
                return Err(OperationError::InvalidInput);
            }
            let mut updates = client.send_queue().subscribe();
            let id = item.identifier();
            let added = timeline
                .toggle_reaction(&id, key)
                .await
                .map_err(|_| OperationError::Transport)?;
            // `false`: it removed a queued echo of the first attempt; add it again.
            if !added {
                timeline
                    .toggle_reaction(&id, key)
                    .await
                    .map_err(|_| OperationError::Transport)?;
            }
            sent_event(&mut updates, &room, |e| {
                let r = relation(e);
                e["type"] == "m.reaction"
                    && r["rel_type"] == "m.annotation"
                    && r["event_id"].as_str() == Some(target.as_str())
                    && r["key"].as_str() == Some(key.as_str())
            })
            .await
        }
        Operation::Edit { body } => {
            let item = item.ok_or(OperationError::Unavailable)?;
            if item.content().as_message().is_none() {
                return Err(OperationError::Unavailable);
            }
            if !item.is_own() {
                return Err(OperationError::Forbidden);
            }
            if !item.is_editable() {
                return Err(OperationError::Unavailable);
            }
            let mut updates = client.send_queue().subscribe();
            let content = crate::sending::markdown_content(body, &[]);
            timeline
                .edit(
                    &item.identifier(),
                    EditedContent::RoomMessage(content.into()),
                )
                .await
                .map_err(|_| OperationError::Transport)?;
            sent_event(&mut updates, &room, |e| {
                let r = relation(e);
                r["rel_type"] == "m.replace" && r["event_id"].as_str() == Some(target.as_str())
            })
            .await
        }
        Operation::Reply { body } => {
            if item.is_none() {
                return Err(OperationError::Unavailable);
            }
            let mut updates = client.send_queue().subscribe();
            let content = crate::sending::markdown_content(body, &[]);
            timeline
                .send_reply(content.into(), target.clone())
                .await
                .map_err(|_| OperationError::Transport)?;
            sent_event(&mut updates, &room, |e| {
                relation(e)["m.in_reply_to"]["event_id"].as_str() == Some(target.as_str())
            })
            .await
        }
    }
}

#[cfg(feature = "sdk")]
async fn is_reaction(room: &matrix_sdk::Room, id: &matrix_sdk::ruma::EventId) -> bool {
    let Ok(Ok(event)) = tokio::time::timeout(WAIT, room.event(id, None)).await else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(event.raw().json().get())
        .is_ok_and(|event| event["type"] == "m.reaction")
}

/// A reaction is not a Timeline item, so the SDK cannot address it by event ID: removing one by
/// its ID stays a plain redaction with our transaction ID, recoverable with that ID as proof.
#[cfg(feature = "sdk")]
async fn redact_reaction(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    request: &OperationRequest,
    retry: bool,
) -> Result<EventId, OperationError> {
    let target = matrix_sdk::ruma::EventId::parse(request.target.as_str())
        .map_err(|_| OperationError::InvalidInput)?;
    if retry {
        return redaction_of(room, user, &target, Some(request.transaction_id.as_str())).await;
    }
    let event = tokio::time::timeout(WAIT, room.event(&target, None))
        .await
        .map_err(|_| OperationError::Transport)?
        .map_err(|_| OperationError::Unavailable)?;
    let event: serde_json::Value =
        serde_json::from_str(event.raw().json().get()).map_err(|_| OperationError::Unavailable)?;
    if event["type"] != "m.reaction" {
        return Err(OperationError::Unavailable);
    }
    let response = tokio::time::timeout(
        WAIT,
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
    Ok(EventId::new(response.event_id.as_str()))
}

/// The redaction of `target` as stored by the server, if this user made it. With `transaction`,
/// only the one sent with that transaction ID counts. A deleted row alone is no proof.
#[cfg(feature = "sdk")]
async fn redaction_of(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    target: &matrix_sdk::ruma::EventId,
    transaction: Option<&str>,
) -> Result<EventId, OperationError> {
    use matrix_sdk::ruma::EventId as MatrixEventId;
    tokio::time::timeout(WAIT, async {
        let original = room
            .event(target, None)
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
                .is_some_and(|id| id != room.room_id().as_str())
            || redacts.as_str() != Some(target.as_str())
            || transaction.is_some_and(|t| event["unsigned"]["transaction_id"].as_str() != Some(t))
        {
            return Err(OperationError::Transport);
        }
        Ok(EventId::new(id.as_str()))
    })
    .await
    .map_err(|_| OperationError::Transport)?
}

/// The user's own reaction `key` on `target` as stored by the server (the SDK shows no event ID for
/// one it already confirmed).
#[cfg(feature = "sdk")]
async fn own_reaction(
    room: &matrix_sdk::Room,
    user: &matrix_sdk::ruma::UserId,
    target: &matrix_sdk::ruma::EventId,
    key: &str,
) -> Result<EventId, OperationError> {
    use matrix_sdk::{
        room::{IncludeRelations, RelationsOptions},
        ruma::events::relation::RelationType,
    };
    tokio::time::timeout(WAIT, async {
        let mut from = None;
        // shortcut: 20 pages of 25 annotations, then the reaction counts as not found.
        for _ in 0..20 {
            let page = room
                .relations(
                    target.to_owned(),
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
                if value["sender"].as_str() == Some(user.as_str())
                    && value["type"] == "m.reaction"
                    && value["content"]["m.relates_to"]["key"].as_str() == Some(key)
                    && let Some(id) = value["event_id"].as_str()
                {
                    return Ok(EventId::new(id));
                }
            }
            from = page.next_batch_token;
            if from.is_none() {
                break;
            }
        }
        Err(OperationError::Transport)
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
                            None,
                        )
                    })
                    .collect(),
            });
            assert_eq!(
                perform(&client, &crate::room_sync::Timelines::default(), AccountId::new(52), &state, &request).await,
                Ok(EventId::new("$confirmed"))
            );
            let new = OperationRequest {
                transaction_id: TransactionId::new("new"),
                ..request
            };
            assert_eq!(
                perform(&client, &crate::room_sync::Timelines::default(), AccountId::new(52), &state, &new).await,
                Err(OperationError::Unavailable)
            );
            assert_eq!(state.lock().await.requests.len(), 128);
        });
    }
}
