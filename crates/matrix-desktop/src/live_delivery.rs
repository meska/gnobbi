//! UI-local outgoing projection; payloads never logged, not persisted.
use crate::Composer;
use matrix_domain::{RoomScope, Timeline, TimelineItem, TimelineUpdate, TransactionId};
use matrix_engine::sending::{Attachment, ControlSlot, Mention, SendControl, SendNotice, SendRequest};
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub struct LiveDelivery {
    rooms: HashMap<RoomScope, Timeline>,
    staged: VecDeque<SendRequest>,
    failures: HashMap<(RoomScope, TransactionId), matrix_engine::sending::SendError>,
    /// Attachment sends kept until confirmed so Retry can resend the file.
    attachments: HashMap<(RoomScope, TransactionId), (String, Attachment)>,
    /// Poll sends kept until confirmed so Retry sends the poll again, not its fallback text.
    polls: HashMap<(RoomScope, TransactionId), matrix_domain::PollDraft>,
    /// Mentions of a text send, kept until confirmed so Retry keeps the links.
    mentions: HashMap<(RoomScope, TransactionId), Vec<Mention>>,
    /// The worker's send queue: retry, cancel and edit of a text it holds. Empty until it is up.
    control: ControlSlot,
}
impl LiveDelivery {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_control(control: ControlSlot) -> Self {
        Self {
            control,
            ..Self::default()
        }
    }
    fn queue(&self) -> Option<std::sync::Arc<dyn SendControl>> {
        self.control.lock().ok()?.clone()
    }
    /// A message still queued or failed (not persisted: a restart would lose it).
    pub fn has_unsent(&self) -> bool {
        self.rooms.values().any(|t| t.items().iter().any(|i| i.status != matrix_domain::SendStatus::Sent))
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
    /// A poll from the dialog: shown as its fallback text until the server echoes the poll.
    pub fn submit_poll(&mut self, scope: RoomScope, transaction_id: TransactionId, draft: matrix_domain::PollDraft) -> bool {
        let body = draft.fallback_text();
        if self.staged.len() >= 8 || draft.validate().is_err() || transaction_id.as_str().is_empty() {
            return false;
        }
        let timeline = self.rooms.entry(scope.clone()).or_insert_with(|| Timeline::new(scope.clone()));
        if timeline.items().iter().any(|i| i.transaction_id.as_ref() == Some(&transaction_id)) {
            return false;
        }
        if timeline
            .apply(&scope, TimelineUpdate::LocalSend { transaction_id: transaction_id.clone(), body: body.clone() })
            .is_err()
        {
            return false;
        }
        self.polls.insert((scope.clone(), transaction_id.clone()), draft.clone());
        self.staged.push_back(SendRequest { scope, transaction_id, body, attachment: None, poll: Some(draft), mentions: Vec::new() });
        true
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
            poll: None,
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
            self.polls.remove(&key);
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
                Some(matrix_engine::sending::SendError::TooLarge) => "File troppo grande per il server",
                _ => "Invio non confermato · Matrix",
            },
        )
    }
    /// A failure trying again cannot fix (the server refused the file as too large).
    pub fn is_final_failure(&self, scope: &RoomScope, transaction_id: &TransactionId) -> bool {
        self.failures.get(&(scope.clone(), transaction_id.clone()))
            == Some(&matrix_engine::sending::SendError::TooLarge)
    }
    pub fn retry(&mut self, scope: &RoomScope, transaction_id: &TransactionId) -> bool {
        if self.is_final_failure(scope, transaction_id) {
            return false;
        }
        let queue = self.queue();
        if self.staged.len() >= 8 {
            return false;
        }
        let Some(timeline) = self.rooms.get_mut(scope) else {
            return false;
        };
        let key = (scope.clone(), transaction_id.clone());
        // A text the send queue holds is tried again there (same message, same transaction).
        if !self.attachments.contains_key(&key)
            && !self.polls.contains_key(&key)
            && let Some(queue) = queue
            && queue.retry(transaction_id)
        {
            let retried = timeline
                .apply(
                    scope,
                    TimelineUpdate::Retry {
                        transaction_id: transaction_id.clone(),
                    },
                )
                .is_ok();
            self.failures.remove(&key);
            return retried;
        }
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
            poll: self.polls.get(&key).cloned(),
            mentions: self.mentions.get(&key).cloned().unwrap_or_default(),
        });
        true
    }
    /// One thing the worker's send route delivered.
    pub fn handle(&mut self, event: matrix_engine::sending::SendEvent) -> bool {
        match event {
            matrix_engine::sending::SendEvent::Result(result) => self.apply(result),
            matrix_engine::sending::SendEvent::Notice {
                scope,
                transaction_id,
                notice,
            } => self.notice(scope, transaction_id, notice),
        }
    }
    /// Withdraw an unsent message. A text in the send queue leaves the list when the queue says
    /// so (`notice`); anything else, still staged or never queued, leaves at once.
    pub fn cancel(&mut self, scope: &RoomScope, transaction_id: &TransactionId) -> bool {
        if self.queue().is_some_and(|queue| queue.cancel(transaction_id)) {
            return true;
        }
        self.staged
            .retain(|r| !(r.scope == *scope && r.transaction_id == *transaction_id));
        self.discard(scope, transaction_id)
    }
    /// Change the text of an unsent message before it goes out.
    pub fn edit(&mut self, scope: &RoomScope, transaction_id: &TransactionId, body: &str) -> bool {
        if body.trim().is_empty() {
            return false;
        }
        if self.queue().is_some_and(|queue| queue.edit(transaction_id, body, &[])) {
            return true;
        }
        let Some(request) = self.staged.iter_mut().find(|r| {
            r.scope == *scope && r.transaction_id == *transaction_id && r.attachment.is_none()
        }) else {
            return false;
        };
        request.body = body.to_owned();
        request.mentions.clear();
        self.mentions
            .remove(&(scope.clone(), transaction_id.clone()));
        self.rooms.get_mut(scope).is_some_and(|timeline| {
            timeline
                .apply(
                    scope,
                    TimelineUpdate::Edit {
                        transaction_id: transaction_id.clone(),
                        body: body.to_owned(),
                    },
                )
                .is_ok()
        })
    }
    fn discard(&mut self, scope: &RoomScope, transaction_id: &TransactionId) -> bool {
        let key = (scope.clone(), transaction_id.clone());
        let gone = self.rooms.get_mut(scope).is_some_and(|timeline| {
            timeline
                .apply(
                    scope,
                    TimelineUpdate::Discard {
                        transaction_id: transaction_id.clone(),
                    },
                )
                .is_ok()
        });
        if gone {
            self.attachments.remove(&key);
            self.polls.remove(&key);
            self.mentions.remove(&key);
            self.failures.remove(&key);
        }
        gone
    }
    /// What the send queue reports besides a final result.
    pub fn notice(
        &mut self,
        scope: RoomScope,
        transaction_id: TransactionId,
        notice: SendNotice,
    ) -> bool {
        let key = (scope.clone(), transaction_id.clone());
        let timeline = self
            .rooms
            .entry(scope.clone())
            .or_insert_with(|| Timeline::new(scope.clone()));
        let mut apply = |update| timeline.apply(&scope, update).is_ok();
        match notice {
            SendNotice::Restored { body, failed } => {
                let tx = transaction_id.clone();
                let shown = apply(TimelineUpdate::LocalSend {
                    transaction_id: tx.clone(),
                    body,
                });
                if shown && failed {
                    apply(TimelineUpdate::Failed { transaction_id: tx });
                    self.failures
                        .insert(key, matrix_engine::sending::SendError::Transport);
                }
                shown
            }
            SendNotice::Retrying => {
                self.failures.remove(&key);
                apply(TimelineUpdate::Retry { transaction_id })
            }
            SendNotice::Edited { body } => apply(TimelineUpdate::Edit {
                transaction_id,
                body,
            }),
            SendNotice::Cancelled => self.discard(&scope, &transaction_id),
        }
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
