//! UI-local outgoing projection; payloads never logged, not persisted.
use crate::Composer;
use matrix_domain::{RoomScope, Timeline, TimelineItem, TimelineUpdate, TransactionId};
use matrix_engine::sending::{Attachment, Mention, SendRequest};
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub struct LiveDelivery {
    rooms: HashMap<RoomScope, Timeline>,
    staged: VecDeque<SendRequest>,
    failures: HashMap<(RoomScope, TransactionId), matrix_engine::sending::SendError>,
    /// Attachment sends kept until confirmed so Retry can resend the file.
    attachments: HashMap<(RoomScope, TransactionId), (String, Attachment)>,
    /// Mentions of a text send, kept until confirmed so Retry keeps the links.
    mentions: HashMap<(RoomScope, TransactionId), Vec<Mention>>,
}
impl LiveDelivery {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn items(&self, scope: &RoomScope) -> &[TimelineItem] {
        self.rooms
            .get(scope)
            .map_or(&[], |timeline| timeline.items())
    }
    /// Clear only after local acceptance. Full queue/invalid input leave editor intact.
    pub fn submit(
        &mut self,
        scope: RoomScope,
        transaction_id: TransactionId,
        draft: &mut Composer,
    ) -> bool {
        self.submit_with(scope, transaction_id, draft, &mut None)
    }
    /// The draft text is the caption of `attachment`; it is taken only when accepted.
    pub fn submit_with(
        &mut self,
        scope: RoomScope,
        transaction_id: TransactionId,
        draft: &mut Composer,
        attachment: &mut Option<Attachment>,
    ) -> bool {
        self.submit_mentioning(scope, transaction_id, draft, attachment, Vec::new())
    }
    /// Like `submit_with`; `mentions` name users in the draft text.
    pub fn submit_mentioning(
        &mut self,
        scope: RoomScope,
        transaction_id: TransactionId,
        draft: &mut Composer,
        attachment: &mut Option<Attachment>,
        mentions: Vec<Mention>,
    ) -> bool {
        if self.staged.len() >= 8
            || (draft.text().trim().is_empty() && attachment.is_none())
            || draft.marked().is_some()
            || transaction_id.as_str().is_empty()
        {
            return false;
        }
        let timeline = self
            .rooms
            .entry(scope.clone())
            .or_insert_with(|| Timeline::new(scope.clone()));
        if timeline
            .items()
            .iter()
            .any(|i| i.transaction_id.as_ref() == Some(&transaction_id))
        {
            return false;
        }
        let body = draft.text().to_owned();
        let shown = match &*attachment {
            Some(file) if body.trim().is_empty() => file.filename.clone(),
            _ => body.clone(),
        };
        if timeline
            .apply(
                &scope,
                TimelineUpdate::LocalSend {
                    transaction_id: transaction_id.clone(),
                    body: shown,
                },
            )
            .is_err()
        {
            return false;
        }
        let attachment = attachment.take();
        if let Some(file) = &attachment {
            self.attachments.insert(
                (scope.clone(), transaction_id.clone()),
                (body.clone(), file.clone()),
            );
        }
        if !mentions.is_empty() {
            self.mentions
                .insert((scope.clone(), transaction_id.clone()), mentions.clone());
        }
        self.staged.push_back(SendRequest {
            scope,
            transaction_id,
            body,
            attachment,
            mentions,
        });
        draft.enter(false);
        true
    }
    pub fn apply(&mut self, result: matrix_engine::sending::SendResult) -> bool {
        let Some(timeline) = self.rooms.get_mut(&result.scope) else {
            return false;
        };
        let key = (result.scope.clone(), result.transaction_id.clone());
        let error = result.result.as_ref().err().copied();
        let update = match result.result {
            Ok(event_id) => TimelineUpdate::Sent {
                transaction_id: result.transaction_id,
                event_id,
            },
            Err(_) => TimelineUpdate::Failed {
                transaction_id: result.transaction_id,
            },
        };
        if timeline.apply(&result.scope, update).is_err() {
            return false;
        }
        if error.is_none() {
            self.attachments.remove(&key);
            self.mentions.remove(&key);
        }
        if let Some(error) = error
            && timeline.items().iter().any(|item| {
                item.transaction_id.as_ref() == Some(&key.1)
                    && item.status == matrix_domain::SendStatus::Failed
            })
        {
            self.failures.insert(key, error);
        } else {
            self.failures.remove(&key);
        }
        true
    }
    /// Static UI copy only, no SDK error text or sensitive payloads.
    pub fn failure_label(
        &self,
        scope: &RoomScope,
        transaction_id: &TransactionId,
    ) -> Option<&'static str> {
        if !self.items(scope).iter().any(|item| {
            item.transaction_id.as_ref() == Some(transaction_id)
                && item.status == matrix_domain::SendStatus::Failed
        }) {
            return None;
        }
        Some(
            match self.failures.get(&(scope.clone(), transaction_id.clone())) {
                Some(matrix_engine::sending::SendError::Encryption) => {
                    "Cifratura non riuscita · nessun invio in chiaro"
                }
                _ => "Invio non confermato · Matrix",
            },
        )
    }
    pub fn retry(&mut self, scope: &RoomScope, transaction_id: &TransactionId) -> bool {
        if self.staged.len() >= 8 {
            return false;
        }
        let Some(timeline) = self.rooms.get_mut(scope) else {
            return false;
        };
        let key = (scope.clone(), transaction_id.clone());
        let Some((body, attachment)) = self.attachments.get(&key).cloned().map_or_else(
            || {
                timeline
                    .items()
                    .iter()
                    .find(|i| i.transaction_id.as_ref() == Some(transaction_id))
                    .map(|i| (i.body.clone(), None))
            },
            |(caption, file)| Some((caption, Some(file))),
        ) else {
            return false;
        };
        if timeline
            .apply(
                scope,
                TimelineUpdate::Retry {
                    transaction_id: transaction_id.clone(),
                },
            )
            .is_err()
        {
            return false;
        }
        self.failures
            .remove(&(scope.clone(), transaction_id.clone()));
        self.staged.push_back(SendRequest {
            scope: scope.clone(),
            transaction_id: transaction_id.clone(),
            body,
            attachment,
            mentions: self.mentions.get(&key).cloned().unwrap_or_default(),
        });
        true
    }
    pub fn flush(&mut self, sender: &mut matrix_engine::Sender<SendRequest>) {
        // At most the eight staged requests; never wait/spin on UI.
        while let Some(request) = self.staged.pop_front() {
            match sender.try_send(request) {
                Ok(()) => (),
                Err(
                    matrix_engine::TrySendError::Full(request)
                    | matrix_engine::TrySendError::Busy(request),
                ) => {
                    self.staged.push_front(request);
                    break;
                }
                Err(matrix_engine::TrySendError::Closed(request)) => {
                    self.apply(matrix_engine::sending::SendResult {
                        scope: request.scope,
                        transaction_id: request.transaction_id,
                        result: Err(matrix_engine::sending::SendError::Unavailable),
                    });
                }
            }
        }
    }
    pub fn take_request(&mut self) -> Option<SendRequest> {
        self.staged.pop_front()
    }
}
