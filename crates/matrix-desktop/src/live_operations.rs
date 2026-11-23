//! View inspection is cancellable; accepted writes will remain room-scoped.
use crate::i18n;
use matrix_engine::history::Reaction;
use std::time::{Duration, Instant};
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
    /// Taps on a reaction pill, shown at once and until the server shows the same (or the send fails).
    wishes: Vec<Wish>,
}

/// A reaction the user asked for and the timeline does not show yet: `add` it, or take it back.
#[derive(Clone)]
pub struct Wish {
    pub target: EventId,
    pub key: String,
    pub add: bool,
    request: OperationRequest,
    /// When the server confirmed it; the wish then lives a few seconds more, for the timeline to catch up.
    done: Option<Instant>,
    since: Instant,
}

/// How long a confirmed wish stays, and the longest an unanswered one does.
const WISH_AFTER_DONE: Duration = Duration::from_secs(8);
const WISH_MAX: Duration = Duration::from_secs(30);

/// One reaction per user: adding `key` takes our other pills back (the engine redacts them), as `pending`.
fn drop_mine_but(out: &mut Vec<Reaction>, key: &str) {
    let mut i = 0;
    while i < out.len() {
        if out[i].key != key && out[i].mine {
            if out[i].count <= 1 {
                out.remove(i);
                continue;
            }
            (out[i].count, out[i].mine, out[i].pending) = (out[i].count - 1, false, true);
        }
        i += 1;
    }
}

/// `server` with the wishes for `target` laid over it: an emoji added shows with us counted, one taken back without us; both as
/// `pending` (dimmed). A wish the server already agrees with changes nothing.
pub fn overlay(wishes: &[Wish], target: &EventId, server: &[Reaction]) -> Vec<Reaction> {
    let mut out = server.to_vec();
    for w in wishes.iter().filter(|w| w.target == *target) {
        if w.add {
            drop_mine_but(&mut out, &w.key);
        }
        match (out.iter().position(|r| r.key == w.key), w.add) {
            (Some(at), true) if !out[at].mine => {
                let r = &mut out[at];
                (r.count, r.mine, r.pending) = (r.count + 1, true, true);
            }
            (None, true) => out.push(Reaction { key: w.key.clone(), count: 1, senders: Vec::new(), mine: true, pending: true }),
            (Some(at), false) if out[at].mine => {
                if out[at].count <= 1 {
                    out.remove(at);
                } else {
                    let r = &mut out[at];
                    (r.count, r.mine, r.pending) = (r.count - 1, false, true);
                }
            }
            _ => {}
        }
    }
    out
}

impl LiveOperations {
    /// The user tapped the pill `key` of `target` (`mine`: it is theirs now): asks the engine to add or take it back. No message
    /// selection and no operations panel: the answer only settles the wish (a failed send drops it, so the pill rolls back).
    pub fn react_direct(&mut self, scope: RoomScope, target: EventId, key: &str, mine: bool, transaction_id: TransactionId) -> bool {
        if self.command.is_some() || key.trim().is_empty() || key.len() > 32768 || transaction_id.as_str().is_empty() || self.wishes.iter().any(|w| w.target == target && w.key == key) {
            return false;
        }
        let operation = if mine { Operation::Unreact { key: key.to_owned() } } else { Operation::React { key: key.to_owned() } };
        let request = OperationRequest { scope, target: target.clone(), transaction_id, operation };
        self.command = Some(OperationCommand::Apply(request.clone()));
        self.wishes.push(Wish { target, key: key.to_owned(), add: !mine, request, done: None, since: Instant::now() });
        true
    }
    pub fn wishes(&self) -> &[Wish] {
        &self.wishes
    }
    /// Forgets the wishes that settled or went stale.
    pub fn tick(&mut self, now: Instant) {
        self.wishes.retain(|w| w.done.map_or(now.saturating_duration_since(w.since) < WISH_MAX, |d| now.saturating_duration_since(d) < WISH_AFTER_DONE));
    }

    /// Shared visible/semantic status; accepted writes outlive selection dismissal.
    /// Accessible-only idle text; never drawn on screen.
    pub fn notice(&self, confirming: bool) -> &'static str {
        if self.error == Some(OperationError::Transport) {
            i18n::t("Operation not confirmed · Retry uses the same ID (even if you change room)")
        } else if self.error.is_some() {
            i18n::t("Operation refused or unavailable")
        } else if self.is_pending() {
            i18n::t("Operation pending · no server confirmation")
        } else if self.inspection_error.is_some() {
            i18n::t("Actions not available for this event")
        } else if self.details.is_none() {
            i18n::t("Checking permissions…")
        } else if confirming {
            i18n::t("Delete the selected message? This cannot be undone.")
        } else {
            i18n::t("Reply/edit: write in the draft, then use the button. Enter sends normally.")
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
            Operation::React { key } | Operation::Unreact { key } => Some(key),
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
            Operation::React { .. } | Operation::Unreact { .. } => details.permissions.react,
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
    /// Menu: choosing an emoji again removes it (`mine`: the row already carries ours).
    pub fn toggle_reaction(&mut self, key: &str, mine: bool, transaction_id: TransactionId) -> bool {
        let key = key.to_owned();
        self.submit(if mine { Operation::Unreact { key } } else { Operation::React { key } }, transaction_id)
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
            | OperationResult::ReadMarkers { .. }
            | OperationResult::ReadAdvanced { .. }
            | OperationResult::NotificationModeSet { .. }
            | OperationResult::TagsEdited { .. }
            | OperationResult::RoomStarted { .. }
            | OperationResult::SpaceCreated { .. }
            | OperationResult::SpaceChoices { .. }
            | OperationResult::SpaceChanged { .. }
            | OperationResult::SpacePlaced { .. }
            | OperationResult::SpaceChildren { .. }
            | OperationResult::BannedMembers { .. }
            | OperationResult::MemberPermissions { .. }
            | OperationResult::Join { .. }
            | OperationResult::RoomEdited { .. }
            | OperationResult::Aliases { .. }
            | OperationResult::InviteAnswered { .. }
            | OperationResult::RoomSearch { .. }
            | OperationResult::ServerProbed { .. }
            | OperationResult::RoomLeft { .. }
            | OperationResult::SpaceLeft { .. }
            | OperationResult::SpaceDeleted { .. }
            | OperationResult::Profile { .. }
            | OperationResult::Ignored { .. }
            | OperationResult::MemberRights { .. }
            | OperationResult::Moderated { .. }
            | OperationResult::Thread { .. }
            | OperationResult::ThreadRead { .. }
            | OperationResult::ThreadSent { .. }
            | OperationResult::StickerPacks { .. }
            | OperationResult::StickerSent { .. }
            | OperationResult::StickerEdited { .. }
            | OperationResult::Members { .. }
            | OperationResult::MessageSearch { .. }
            | OperationResult::AllMessageSearch { .. }
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
            OperationResult::Applied { request, result } if self.wishes.iter().any(|w| w.request == request) => {
                match result {
                    Ok(_) => self.wishes.iter_mut().filter(|w| w.request == request).for_each(|w| w.done = Some(Instant::now())),
                    // Refused or lost: the pill goes back to what the server shows.
                    Err(_) => self.wishes.retain(|w| w.request != request),
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
