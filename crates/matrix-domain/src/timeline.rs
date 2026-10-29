use crate::{EventId, RoomScope, ScopedTimelineId, TimelineId, TransactionId};

/// Projection state: acknowledgement or server echo is authoritative over failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendStatus {
    Pending,
    Failed,
    Sent,
}

/// Read-only through `Timeline::items`; transaction correlation survives promotion.
/// Message bodies are private user data: callers must not log this DTO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineItem {
    pub identity: ScopedTimelineId,
    pub transaction_id: Option<TransactionId>,
    pub body: String,
    pub status: SendStatus,
}

/// Adapter inputs, scoped by the separate mandatory `apply` argument.
/// Echo transaction IDs must be authenticated by the adapter as belonging to
/// this account's local sends; never correlate by body text or arbitrary sender.
/// Failed carries no raw transport error, avoiding secrets in domain errors.
pub enum TimelineUpdate {
    LocalSend {
        transaction_id: TransactionId,
        body: String,
    },
    Failed {
        transaction_id: TransactionId,
    },
    Retry {
        transaction_id: TransactionId,
    },
    Sent {
        transaction_id: TransactionId,
        event_id: EventId,
    },
    /// The unsent message was withdrawn from the send queue: it leaves the projection.
    Discard {
        transaction_id: TransactionId,
    },
    /// The unsent message was edited in the send queue.
    Edit {
        transaction_id: TransactionId,
        body: String,
    },
    Echo {
        transaction_id: Option<TransactionId>,
        event_id: EventId,
        body: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineError {
    ScopeMismatch,
    IdentityConflict,
    InvalidTransition,
    UnknownTransaction,
}

/// In-memory projection for one account/room, not a transport or SDK store.
/// Linear lookups, arrival ordering, text-only content; no persistence, network,
/// pagination, attempt-generation filtering or viewport anchoring is provided.
pub struct Timeline {
    scope: RoomScope,
    items: Vec<TimelineItem>,
}

impl Timeline {
    pub fn new(scope: RoomScope) -> Self {
        Self {
            scope,
            items: Vec::new(),
        }
    }
    pub fn items(&self) -> &[TimelineItem] {
        &self.items
    }
    /// Apply one update atomically; errors leave all items unchanged.
    /// Retry is valid only for Failed; Sent cannot regress to Pending/Failed.
    pub fn apply(
        &mut self,
        scope: &RoomScope,
        update: TimelineUpdate,
    ) -> Result<(), TimelineError> {
        if scope != &self.scope {
            return Err(TimelineError::ScopeMismatch);
        }
        let transaction = match &update {
            TimelineUpdate::Failed { transaction_id }
            | TimelineUpdate::Retry { transaction_id }
            | TimelineUpdate::Discard { transaction_id }
            | TimelineUpdate::Edit { transaction_id, .. }
            | TimelineUpdate::Sent { transaction_id, .. } => Some(transaction_id),
            _ => None,
        };
        if transaction.is_some_and(|tx| {
            !self
                .items
                .iter()
                .any(|i| i.transaction_id.as_ref() == Some(tx))
        }) {
            return Err(TimelineError::UnknownTransaction);
        }
        let mapping = match &update {
            TimelineUpdate::Sent {
                transaction_id,
                event_id,
            } => Some((Some(transaction_id), event_id)),
            TimelineUpdate::Echo {
                transaction_id,
                event_id,
                ..
            } => Some((transaction_id.as_ref(), event_id)),
            _ => None,
        };
        if let Some((Some(tx), event)) = mapping {
            for item in &self.items {
                if item.transaction_id.as_ref() == Some(tx)
                    && matches!(&item.identity.id, TimelineId::Server(existing) if existing != event)
                    || item.identity.id == TimelineId::Server(event.clone())
                        && item
                            .transaction_id
                            .as_ref()
                            .is_some_and(|existing| existing != tx)
                {
                    return Err(TimelineError::IdentityConflict);
                }
            }
        }
        match update {
            TimelineUpdate::Echo {
                transaction_id,
                event_id,
                body,
            } => {
                let remote = self
                    .items
                    .iter()
                    .position(|i| i.identity.id == TimelineId::Server(event_id.clone()));
                let local = transaction_id.as_ref().and_then(|tx| {
                    self.items
                        .iter()
                        .position(|i| i.transaction_id.as_ref() == Some(tx))
                });
                let index = local.or(remote);
                if let Some(index) = index {
                    let item = &mut self.items[index];
                    item.identity.id = TimelineId::Server(event_id);
                    item.status = SendStatus::Sent;
                    item.body = body;
                    if item.transaction_id.is_none() {
                        item.transaction_id = transaction_id;
                    }
                    if let Some(remote) = remote.filter(|&r| r != index) {
                        self.items.remove(remote);
                    }
                } else {
                    self.items.push(TimelineItem {
                        identity: ScopedTimelineId {
                            scope: self.scope.clone(),
                            id: TimelineId::Server(event_id),
                        },
                        transaction_id,
                        body,
                        status: SendStatus::Sent,
                    });
                }
            }
            TimelineUpdate::Sent {
                transaction_id,
                event_id,
            } => {
                if let Some(local) = self
                    .items
                    .iter()
                    .position(|i| i.transaction_id.as_ref() == Some(&transaction_id))
                {
                    let remote = self
                        .items
                        .iter()
                        .position(|i| i.identity.id == TimelineId::Server(event_id.clone()));
                    if let Some(remote) = remote.filter(|&r| r != local) {
                        self.items[local].body = self.items[remote].body.clone();
                    }
                    self.items[local].identity.id = TimelineId::Server(event_id);
                    self.items[local].status = SendStatus::Sent;
                    if let Some(remote) = remote.filter(|&r| r != local) {
                        self.items.remove(remote);
                    }
                }
            }
            TimelineUpdate::Failed { transaction_id } => {
                if let Some(item) = self
                    .items
                    .iter_mut()
                    .find(|i| i.transaction_id.as_ref() == Some(&transaction_id))
                {
                    if item.status == SendStatus::Sent {
                        return Err(TimelineError::InvalidTransition);
                    }
                    item.status = SendStatus::Failed;
                }
            }
            TimelineUpdate::Discard { transaction_id } => {
                let sent = |i: &TimelineItem| {
                    i.transaction_id.as_ref() == Some(&transaction_id)
                        && i.status == SendStatus::Sent
                };
                if self.items.iter().any(sent) {
                    return Err(TimelineError::InvalidTransition);
                }
                self.items
                    .retain(|i| i.transaction_id.as_ref() != Some(&transaction_id));
            }
            TimelineUpdate::Edit {
                transaction_id,
                body,
            } => {
                if let Some(item) = self
                    .items
                    .iter_mut()
                    .find(|i| i.transaction_id.as_ref() == Some(&transaction_id))
                {
                    if item.status == SendStatus::Sent {
                        return Err(TimelineError::InvalidTransition);
                    }
                    item.body = body;
                }
            }
            TimelineUpdate::Retry { transaction_id } => {
                if let Some(item) = self
                    .items
                    .iter_mut()
                    .find(|i| i.transaction_id.as_ref() == Some(&transaction_id))
                {
                    if item.status != SendStatus::Failed {
                        return Err(TimelineError::InvalidTransition);
                    }
                    item.status = SendStatus::Pending;
                }
            }
            TimelineUpdate::LocalSend {
                transaction_id,
                body,
            } => {
                if let Some(item) = self
                    .items
                    .iter()
                    .find(|i| i.transaction_id.as_ref() == Some(&transaction_id))
                {
                    return if item.body == body {
                        Ok(())
                    } else {
                        Err(TimelineError::IdentityConflict)
                    };
                }
                self.items.push(TimelineItem {
                    identity: ScopedTimelineId {
                        scope: self.scope.clone(),
                        id: TimelineId::Pending(transaction_id.clone()),
                    },
                    transaction_id: Some(transaction_id),
                    body,
                    status: SendStatus::Pending,
                });
            }
        }
        Ok(())
    }
}
