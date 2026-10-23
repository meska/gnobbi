//! In-memory, account/room-scoped editor sessions. Never persisted or logged.
use crate::Composer;
use matrix_domain::RoomScope;
use std::collections::HashMap;

pub struct RoomDrafts {
    active: RoomScope,
    saved: HashMap<RoomScope, Composer>,
}

impl RoomDrafts {
    pub fn new(active: RoomScope) -> Self {
        Self {
            active,
            saved: HashMap::new(),
        }
    }

    /// Local privacy cleanup after confirmed logout; never touches other accounts.
    /// The caller must also clear its room/timeline/media projections and routes.
    pub fn forget_account(&mut self, account: matrix_domain::AccountId, editor: &mut Composer) {
        self.saved.retain(|scope, _| scope.account_id != account);
        if self.active.account_id == account {
            *editor = Composer::default();
        }
    }

    pub fn switch(&mut self, scope: RoomScope, editor: &mut Composer) {
        if scope == self.active {
            return;
        }
        editor.unmark();
        let previous = std::mem::take(editor);
        self.saved.insert(self.active.clone(), previous);
        *editor = self.saved.remove(&scope).unwrap_or_default();
        self.active = scope;
    }
}
