//! Reopening the saved room after start/sign-in/account switch, once the sync lists it.
use matrix_domain::{AccountId, RoomId};
use std::time::{Duration, Instant};

/// How long a saved room that the sync has not listed yet is waited for; then no room is opened.
pub const RESTORE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct LastRoomRestore {
    armed_for: Option<AccountId>,
    pending: Option<(RoomId, Instant)>,
}

impl LastRoomRestore {
    /// True while a saved room is awaited: the automatic first-row selection must not overwrite it.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The user chose a room by hand: the saved one no longer applies.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    /// Call on every tick. The first call for an account arms the wait with its `saved` room.
    /// `open` selects the room if the list has it joined and shown. Never falls back to another room.
    pub fn poll(
        &mut self,
        account: AccountId,
        saved: impl FnOnce() -> Option<RoomId>,
        now: Instant,
        open: impl FnOnce(&RoomId) -> bool,
    ) -> Option<RoomId> {
        if self.armed_for != Some(account) {
            self.armed_for = Some(account);
            self.pending = saved().map(|room| (room, now + RESTORE_TIMEOUT));
        }
        let (room, deadline) = self.pending.take()?;
        if open(&room) {
            Some(room)
        } else {
            if now < deadline {
                self.pending = Some((room, deadline));
            }
            None
        }
    }
}
