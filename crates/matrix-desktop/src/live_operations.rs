//! View inspection is cancellable; accepted writes will remain room-scoped.
use matrix_domain::{EventId, Poll, RoomScope, ScopedTimelineId, TimelineId, TransactionId};
use matrix_engine::{
    ViewRequests,
    operations::{
        MessageDetails, Operation, OperationCommand, OperationError, OperationRequest,
        OperationResult,
    },
};

pub struct ConfirmedOperation {
    pub request: OperationRequest,
    pub event_id: EventId,
}

#[derive(Default)]
pub struct LiveOperations {
    views: ViewRequests,
    selection: Option<ScopedTimelineId>,
    selection_tag: Option<matrix_engine::RequestTag>,
    details: Option<MessageDetails>,
    command: Option<OperationCommand>,
    inspection: Option<OperationCommand>,
    error: Option<OperationError>,
    inspection_error: Option<OperationError>,
    pending: Option<OperationRequest>,
    /// Vote in flight: kept apart from `pending`/`error` so the operations panel (message menu) never shows for a vote.
    voting: Option<OperationRequest>,
    /// Poll whose last vote was refused; cleared by the next vote.
    vote_failed: Option<EventId>,
    confirmed: Vec<ConfirmedOperation>,
}
impl LiveOperations {
    /// Accessible-only description of the idle panel; never drawn on screen.
    pub const IDLE_NOTICE: &'static str =
        "Risposta/modifica: scrivi nel draft, poi usa il pulsante. Enter invia normalmente.";
    /// Shared visible/semantic status; accepted writes outlive selection dismissal.
    pub fn notice(&self, confirming: bool) -> &'static str {
        if self.error == Some(OperationError::Transport) {
            "Operazione non confermata · Riprova usa lo stesso ID (anche cambiando stanza)"
        } else if self.error.is_some() {
            "Operazione rifiutata o non disponibile"
        } else if self.is_pending() {
            "Operazione in attesa · nessuna conferma server"
        } else if self.inspection_error.is_some() {
            "Azioni non disponibili per questo evento"
        } else if self.details.is_none() {
            "Verifica permessi…"
        } else if confirming {
            "Eliminare il messaggio selezionato? Questa azione non si annulla."
        } else {
            Self::IDLE_NOTICE
        }
    }
    pub fn select(&mut self, id: ScopedTimelineId) -> bool {
        let TimelineId::Server(target) = &id.id else {
            return false;
        };
        let Ok(tag) = self.views.start(id.scope.clone()) else {
            return false;
        };
        self.selection_tag = Some(tag.clone());
        self.inspection = Some(OperationCommand::Inspect {
            tag,
            target: target.clone(),
        });
        self.selection = Some(id);
        self.details = None;
        self.inspection_error = None;
        true
    }
    /// «Chiudi sondaggio» is offered for an own open poll, when the end event may be sent.
    pub fn can_end_poll(&self, poll: Option<&Poll>) -> bool {
        !self.is_pending() && self.details().is_some_and(|d| d.permissions.redact) && poll.is_some_and(|p| p.can_end)
    }
    pub fn clear_selection(&mut self) {
        self.views.cancel();
        self.selection_tag = None;
        self.selection = None;
        self.details = None;
        self.inspection = None;
    }
    pub fn submit(&mut self, operation: Operation, transaction_id: TransactionId) -> bool {
        if self.pending.is_some() || self.command.is_some() || transaction_id.as_str().is_empty() {
            return false;
        }
        let text = match &operation {
            Operation::Reply { body } | Operation::Edit { body } => Some(body),
            Operation::React { key } => Some(key),
            Operation::Redact | Operation::Vote { .. } | Operation::EndPoll => None,
        };
        if text.is_some_and(|text| text.trim().is_empty() || text.len() > 32768) {
            return false;
        }
        let Some(id) = &self.selection else {
            return false;
        };
        let TimelineId::Server(target) = &id.id else {
            return false;
        };
        let Some(details) = &self.details else {
            return false;
        };
        let allowed = match &operation {
            Operation::Reply { .. } => details.permissions.reply,
            Operation::Edit { .. } => details.permissions.edit,
            Operation::React { .. } => details.permissions.react,
            Operation::Redact => details.permissions.redact,
            // shortcut: reuse react/redact rights; the poll UI card refines them.
            Operation::Vote { .. } => details.permissions.react,
            Operation::EndPoll => details.permissions.redact,
        };
        if !allowed {
            return false;
        }
        let request = OperationRequest {
            scope: id.scope.clone(),
            target: target.clone(),
            transaction_id,
            operation,
        };
        self.command = Some(OperationCommand::Apply(request.clone()));
        self.pending = Some(request);
        self.error = None;
        true
    }
    /// Vote on option `ix` of `poll`: the new answers come from `Poll::toggle`, the state from the Timeline.
    /// Needs no message selection; the server decides whether the vote is allowed.
    pub fn vote(&mut self, scope: RoomScope, target: EventId, poll: &Poll, ix: usize, transaction_id: TransactionId) -> bool {
        // a vote and another operation share the single `command` slot: never overwrite one not yet sent
        if self.voting.is_some() || self.command.is_some() || transaction_id.as_str().is_empty() {
            return false;
        }
        let Some(answers) = poll.answers.get(ix).and_then(|a| poll.toggle(&a.id)) else {
            return false;
        };
        let request = OperationRequest { scope, target, transaction_id, operation: Operation::Vote { answers } };
        self.command = Some(OperationCommand::Apply(request.clone()));
        self.voting = Some(request);
        self.vote_failed = None;
        true
    }
    pub fn vote_failed(&self, target: &EventId) -> bool {
        self.vote_failed.as_ref() == Some(target)
    }
    pub fn toggle_reaction(&mut self, key: &str, transaction_id: TransactionId) -> bool {
        if self.pending.is_some() || self.command.is_some() || transaction_id.as_str().is_empty() {
            return false;
        }
        let Some(selected) = &self.selection else {
            return false;
        };
        let existing = self.confirmed.iter().rev().find(|op| {
            op.request.scope == selected.scope
                && TimelineId::Server(op.request.target.clone()) == selected.id
                && op.request.operation
                    == (Operation::React {
                        key: key.to_owned(),
                    })
                && !self.confirmed.iter().any(|redact| {
                    redact.request.scope == op.request.scope
                        && redact.request.target == op.event_id
                        && redact.request.operation == Operation::Redact
                })
        });
        if let Some(existing) = existing {
            let request = OperationRequest {
                scope: selected.scope.clone(),
                target: existing.event_id.clone(),
                transaction_id,
                operation: Operation::Redact,
            };
            self.command = Some(OperationCommand::Apply(request.clone()));
            self.pending = Some(request);
            self.error = None;
            true
        } else {
            self.submit(
                Operation::React {
                    key: key.to_owned(),
                },
                transaction_id,
            )
        }
    }
    pub fn submit_draft(
        &mut self,
        edit: bool,
        transaction_id: TransactionId,
        draft: &mut crate::Composer,
    ) -> bool {
        if draft.marked().is_some() {
            return false;
        }
        let body = draft.text().to_owned();
        let operation = if edit {
            Operation::Edit { body }
        } else {
            Operation::Reply { body }
        };
        if !self.submit(operation, transaction_id) {
            return false;
        }
        draft.enter(false);
        true
    }
    /// Accepted write retained for an explicit transport retry; never a new draft.
    pub fn retry_request(&self) -> Option<&OperationRequest> {
        (self.error == Some(OperationError::Transport))
            .then_some(self.pending.as_ref())
            .flatten()
    }
    pub fn retry(&mut self) -> bool {
        if self.error != Some(OperationError::Transport) || self.command.is_some() {
            return false;
        }
        let Some(request) = &self.pending else {
            return false;
        };
        self.command = Some(OperationCommand::Apply(request.clone()));
        self.error = None;
        true
    }
    pub fn error(&self) -> Option<OperationError> {
        self.error
    }
    pub fn inspection_error(&self) -> Option<OperationError> {
        self.inspection_error
    }
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub fn confirmed(&self) -> &[ConfirmedOperation] {
        &self.confirmed
    }
    pub fn selection(&self) -> Option<&ScopedTimelineId> {
        self.selection.as_ref()
    }
    pub fn selection_tag(&self) -> Option<&matrix_engine::RequestTag> {
        self.selection_tag.as_ref()
    }
    pub fn details(&self) -> Option<&MessageDetails> {
        self.details.as_ref()
    }
    pub fn take_command(&mut self) -> Option<OperationCommand> {
        self.command.take().or_else(|| self.inspection.take())
    }
    pub fn apply(&mut self, result: OperationResult) -> bool {
        match result {
            // Room markers are fire and forget: the next sync projects the outcome.
            OperationResult::RoomMarked { .. }
            | OperationResult::TagsEdited { .. }
            | OperationResult::RoomStarted { .. }
            | OperationResult::SpaceCreated { .. }
            | OperationResult::SpaceChoices { .. }
            | OperationResult::SpaceChanged { .. }
            | OperationResult::Join { .. }
            | OperationResult::RoomEdited { .. }
            | OperationResult::InviteAnswered { .. }
            | OperationResult::RoomSearch { .. }
            | OperationResult::ServerProbed { .. }
            | OperationResult::RoomLeft { .. }
            | OperationResult::Profile { .. }
            | OperationResult::Ignored { .. }
            | OperationResult::MemberRights { .. }
            | OperationResult::Moderated { .. }
            | OperationResult::Thread { .. }
            | OperationResult::ThreadRead { .. }
            | OperationResult::ThreadSent { .. }
            | OperationResult::Members { .. }
            | OperationResult::MessageSearch { .. }
            | OperationResult::UserSearch { .. } => false,
            OperationResult::Inspected {
                tag,
                target,
                details,
            } => {
                if !self.views.accepts(&tag)
                    || !self.selection.as_ref().is_some_and(|id| {
                        id.scope == *tag.scope() && id.id == TimelineId::Server(target.clone())
                    })
                {
                    return false;
                }
                match details {
                    Ok(details) => self.details = Some(details),
                    Err(error) => self.inspection_error = Some(error),
                }
                true
            }
            OperationResult::Applied { request, result } if self.voting.as_ref() == Some(&request) => {
                self.voting = None;
                match result {
                    Ok(event_id) => self.confirmed.push(ConfirmedOperation { request, event_id }),
                    // A vote is never retried by the panel: the next click is a new vote.
                    Err(_) => self.vote_failed = Some(request.target),
                }
                true
            }
            OperationResult::Applied { request, result } => {
                if !self.pending.as_ref().is_some_and(|pending| {
                    pending.scope == request.scope
                        && pending.target == request.target
                        && pending.transaction_id == request.transaction_id
                        && pending.operation == request.operation
                }) {
                    return false;
                }
                match result {
                    Ok(event_id) => {
                        self.pending = None;
                        self.error = None;
                        self.confirmed
                            .push(ConfirmedOperation { request, event_id });
                    }
                    Err(error) => {
                        self.error = Some(error);
                        if error != OperationError::Transport {
                            self.pending = None;
                        }
                    }
                }
                true
            }
        }
    }
}
