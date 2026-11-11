//! Carries out what `link_open` decided, through screens that already exist: the room list, the
//! «Unisciti» dialog, the invitation row and the profile sheet. Nothing here joins, writes to a room or
//! starts a chat: the user does that with a click afterwards.
use crate::*;
use matrix_desktop::link_open::{self, Candidate, Mode, Plan, Presence};
use matrix_domain::MatrixLink;

/// Only mode A exists on this branch (one shown account, the others parked).
const MODE: Mode = Mode::Separate;
const NOTICE_SECONDS: u64 = 5;

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
        // A profile asks the engine, and only one command goes at a time: wait for it.
        let ready = self.accounts_ready() && self.pending_operation_command.is_none();
        let Some(link) = self.pending_link.take(ready, self.link_modal_open()) else {
            return;
        };
        window.activate_window();
        eprintln!("Gnobbi: matrix link ({})", link_open::kind(&link));
        let candidates = self.link_candidates(&link);
        match link_open::plan(&link, &candidates, MODE, self.active_id) {
            Plan::Room { account, scope } | Plan::Invite { account, scope } => {
                self.show_link_room(account, scope, window, cx)
            }
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
            // Lift what would hide the room, as the quick search does for the shown account.
            state.sidebar.set_filter(state.sidebar.filter().with_unread(false));
            state.sidebar.set_query("");
            state.sidebar.keep_visible(scope.clone());
            state.open_room(&scope);
        }
        self.switch_account_with_room(id, Some(&scope), window, cx);
    }

    pub(crate) fn render_link_notice(&self, t: matrix_desktop::DesignTokens) -> Option<gpui::AnyElement> {
        let (text, _) = self.link_notice?;
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
