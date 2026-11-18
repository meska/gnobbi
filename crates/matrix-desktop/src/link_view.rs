//! Carries out what `link_open` decided, through screens that already exist: the room list, the
//! «Unisciti» dialog, the invitation row and the profile sheet. Nothing here joins, writes to a room or
//! starts a chat: the user does that with a click afterwards.
use crate::*;
use matrix_desktop::link_open::{self, Candidate, Mode, Plan, Presence};
use crate::room_search_view::{Jump, JumpStep};
use matrix_domain::MatrixLink;

/// Only mode A exists on this branch (one shown account, the others parked).
const MODE: Mode = Mode::Separate;
const NOTICE_SECONDS: u64 = 5;
// shortcut: 100 older pages read one after the other (and 30 s in all), switch to /context (SDK Room::event_with_context) when links to old messages are common or slow.
const JUMP_PAGES: u32 = 100;
const JUMP_SECONDS: u64 = 30;

/// A link to a message: waits for its room to be the shown one, then reads back to the event.
pub(crate) struct LinkJump {
    scope: RoomScope,
    event: matrix_domain::EventId,
    pub(crate) pages: u32,
    started: std::time::Instant,
}

impl LinkJump {
    pub(crate) fn new(scope: RoomScope, event: &str) -> Self {
        Self { scope, event: matrix_domain::EventId::new(event), pages: JUMP_PAGES, started: std::time::Instant::now() }
    }
}

fn presence(sidebar: &Sidebar, target: &str) -> Presence {
    // Local only: the room ID or any alias the SDK knows; a joined room wins over an invitation.
    let found = sidebar
        .rooms()
        .iter()
        .filter(|r| r.scope.room_id.as_str() == target || r.info.aliases.iter().any(|a| a == target))
        .min_by_key(|r| r.info.invited);
    match found {
        Some(r) if r.info.invited => Presence::Invited(r.scope.clone()),
        Some(r) => Presence::Joined(r.scope.clone()),
        None => Presence::Absent,
    }
}

impl Shell {
    /// The one entry point for a link from the system (and from a clicked matrix.to link): validates
    /// it again and queues it. It is carried out once the accounts are ready and no dialog is open.
    #[allow(dead_code)] // called by the platform cards (macOS open-URL, single instance)
    #[cfg(feature = "live")]
    pub(crate) fn open_matrix_link(&mut self, raw: &str, cx: &mut Context<Self>) {
        match matrix_engine::matrix_link::parse_matrix_link(raw) {
            Ok(link) => self.pending_link.push(link),
            Err(error) => {
                eprintln!("Gnobbi: ignored matrix link ({error:?})");
                self.link_notice = Some(("Link non valido", std::time::Instant::now()));
            }
        }
        cx.notify();
    }

    fn link_modal_open(&self) -> bool {
        self.new_room.is_some()
            || self.quick.is_some()
            || self.room_edit.is_some()
            || self.profile_edit.is_some()
            || self.groups_edit.is_some()
            || self.confirm_removal.is_some()
            || self.confirm_redaction.is_some()
    }

    /// The shown account is synced and no parked one is still connecting.
    fn accounts_ready(&self) -> bool {
        self.connected
            && self.active_id.is_some()
            && self.status == RoomSyncStatus::Ready
            && self.parked.values().all(|s| s.status != RoomSyncStatus::Connecting)
    }

    /// Every 200 ms: carries out the waiting link when it can, and drops an old notice.
    pub(crate) fn drain_pending_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.link_notice.is_some_and(|(_, at)| at.elapsed().as_secs() >= NOTICE_SECONDS) {
            self.link_notice = None;
            cx.notify();
        }
        if self.keys_notice.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() >= NOTICE_SECONDS) {
            self.keys_notice = None;
            cx.notify();
        }
        // A profile asks the engine, and only one command goes at a time: wait for it.
        let ready = self.accounts_ready() && self.pending_operation_command.is_none();
        let Some(link) = self.pending_link.take(ready, self.link_modal_open()) else {
            return;
        };
        window.activate_window();
        eprintln!("Gnobbi: matrix link ({})", link_open::kind(&link));
        let candidates = self.link_candidates(&link);
        match link_open::plan(&link, &candidates, MODE, self.active_id) {
            Plan::Room { account, scope } => {
                self.show_link_room(account, scope.clone(), window, cx);
                if let MatrixLink::Room { event: Some(event), .. } = &link {
                    self.link_jump = Some(LinkJump::new(scope, event));
                }
            }
            Plan::Invite { account, scope } => self.show_link_room(account, scope, window, cx),
            Plan::Join { target, via, .. } => self.open_join_for_link(&target, &via, window, cx),
            Plan::Profile { account, user_id } => {
                if self.show_link_account(account, window, cx) {
                    self.set_room_details_open(true);
                    self.open_profile(user_id, cx);
                }
            }
            // Mode C only (several accounts inside, none chosen): not built yet.
            Plan::Ask(_) => self.notify_link("Scegli l'account"),
            Plan::Notice(text) => self.notify_link(text),
        }
        cx.notify();
    }

    /// Every 200 ms: carries a link to a message to its row (reading older pages), or says it is not there.
    pub(crate) fn drive_link_jump(&mut self, cx: &mut Context<Self>) {
        let Some(mut link) = self.link_jump.take() else {
            return;
        };
        let expired = link.started.elapsed().as_secs() >= JUMP_SECONDS;
        // Not the shown room (yet): wait; whoever the user opens meanwhile is left alone.
        let step = if expired {
            JumpStep::Unreachable
        } else if self.active_scope != link.scope || self.timeline.is_none() {
            JumpStep::Waiting
        } else {
            // A reply of a thread is shown as its root, with the thread opened, as the search does.
            let jump = match self.timeline.as_ref().and_then(|t| t.thread_target(&link.event)) {
                Some((root, ..)) => Jump { open_thread: root != link.event, show: root, term: String::new() },
                None => Jump { show: link.event.clone(), open_thread: false, term: String::new() },
            };
            self.step_jump(&jump, &mut link.pages, cx)
        };
        match step {
            JumpStep::Waiting => self.link_jump = Some(link),
            JumpStep::Shown => {}
            JumpStep::Unreachable => {
                self.notify_link("Messaggio non trovato");
                cx.notify();
            }
        }
    }

    /// Usable accounts (shown first, then the parked ones by catalog ID) and what each knows of the room.
    fn link_candidates(&self, link: &MatrixLink) -> Vec<Candidate> {
        let of = |id, sidebar: &Sidebar| Candidate {
            id,
            presence: match link {
                MatrixLink::Room { target, .. } => presence(sidebar, target),
                MatrixLink::User { .. } => Presence::Absent,
            },
        };
        let mut parked: Vec<_> = self
            .parked
            .iter()
            .filter(|(id, _)| self.can_switch_to(**id))
            .map(|(id, state)| of(*id, &state.sidebar))
            .collect();
        parked.sort_by_key(|c| c.id.as_u64());
        self.active_id.into_iter().map(|id| of(id, &self.sidebar)).chain(parked).collect()
    }

    fn notify_link(&mut self, text: &'static str) {
        self.link_notice = Some((text, std::time::Instant::now()));
    }

    /// Shows `id`; false (and a line) when that account cannot be shown.
    fn show_link_account(&mut self, id: AccountId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.active_id == Some(id) {
            return true;
        }
        if !self.can_switch_to(id) {
            self.notify_link("Account non disponibile");
            return false;
        }
        self.switch_account(id, window, cx);
        self.active_id == Some(id)
    }

    fn show_link_room(&mut self, id: AccountId, scope: RoomScope, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_id == Some(id) {
            return self.open_scope(scope, window, cx);
        }
        if !self.can_switch_to(id) {
            return self.notify_link("Account non disponibile");
        }
        if let Some(state) = self.parked.get_mut(&id) {
            // Lift the search that would hide the room, as the quick search does for the shown account.
            state.sidebar.set_query("");
            state.open_room(&scope);
        }
        self.switch_account_with_room(id, Some(&scope), window, cx);
    }

    pub(crate) fn render_link_notice(&self, t: matrix_desktop::DesignTokens) -> Option<gpui::AnyElement> {
        let text: gpui::SharedString = match (self.link_notice, &self.keys_notice) {
            (Some((text, _)), _) => text.into(),
            (None, Some((text, _))) => text.clone().into(),
            _ => return None,
        };
        Some(
            div()
                .id("link-notice")
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .pb_4()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(t.surface))
                        .border_1()
                        .border_color(rgb(t.border))
                        .text_color(rgb(t.text))
                        .child(text),
                )
                .into_any_element(),
        )
    }
}
