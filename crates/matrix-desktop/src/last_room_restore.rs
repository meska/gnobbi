//! Reopening the saved room after start/sign-in/account switch, once the sync lists it.
use matrix_domain::{AccountId, RoomId};

#[derive(Default)]
pub struct LastRoomRestore {
    armed_for: Option<AccountId>,
    pending: Option<RoomId>,
    /// The user chose a room before the first arm: that arm must not wait for the saved one.
    manual: bool,
    /// The room reopened by the restore, until the user chooses one: if the server then removes it
    /// (left/banned while the app was closed, seen only on the first sync), no other row takes over.
    opened: Option<RoomId>,
}

impl LastRoomRestore {
    /// True while a saved room is awaited: the automatic first-row selection must not overwrite it.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The user chose a room by hand: the saved one no longer applies.
    pub fn cancel(&mut self) {
        self.pending = None;
        self.opened = None;
        self.manual = self.armed_for.is_none();
    }

    /// True once, when `removed` is the room this restore opened and the user has not chosen since:
    /// it is awaited again (it may come back with the next sync), so no other row takes over meanwhile.
    pub fn lost(&mut self, removed: &RoomId) -> bool {
        let lost = self.opened.take_if(|r| r == removed);
        let found = lost.is_some();
        if found {
            self.pending = lost;
        }
        found
    }

    /// A different account is now shown: forget everything so the next arm waits for its room.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Arms the wait for `account` (once). Call before anything may open or save a room.
    pub fn arm(&mut self, account: AccountId, saved: impl FnOnce() -> Option<RoomId>) {
        if self.armed_for != Some(account) {
            self.armed_for = Some(account);
            let manual = std::mem::take(&mut self.manual);
            self.pending = saved().filter(|_| !manual);
        }
    }

    /// Call on every tick. The first call for an account arms the wait with its `saved` room.
    /// `open` selects the room if the list has it joined and shown. Never falls back to another room.
    pub fn poll(
        &mut self,
        account: AccountId,
        saved: impl FnOnce() -> Option<RoomId>,
        open: impl FnOnce(&RoomId) -> bool,
    ) -> Option<RoomId> {
        self.arm(account, saved);
        let room = self.pending.take()?;
        if open(&room) {
            self.opened = Some(room.clone());
            Some(room)
        } else {
            self.pending = Some(room);
            None
        }
    }
}
