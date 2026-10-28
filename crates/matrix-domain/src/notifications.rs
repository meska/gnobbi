//! Conservative, content-free local unread alerts. No SDK, UI or OS dependencies.
use crate::{AccountId, RoomScope};
use std::collections::{HashMap, HashSet};

pub struct NotificationPolicy {
    account: AccountId,
    enabled: bool,
    count_preview: bool,
    counts: HashMap<RoomScope, u64>,
    muted: HashSet<RoomScope>,
}
impl NotificationPolicy {
    pub fn new(account: AccountId) -> Self {
        Self {
            account,
            enabled: false,
            count_preview: false,
            counts: HashMap::new(),
            muted: HashSet::new(),
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn count_preview(&self) -> bool {
        self.count_preview
    }
    pub fn set_count_preview(&mut self, preview: bool) {
        self.count_preview = preview;
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn set_muted(&mut self, scope: RoomScope, muted: bool) {
        if scope.account_id != self.account {
            return;
        }
        if muted {
            self.muted.insert(scope);
        } else {
            self.muted.remove(&scope);
        }
    }
    pub fn muted(&self, scope: &RoomScope) -> bool {
        self.muted.contains(scope)
    }
    pub fn remove(&mut self, scope: &RoomScope) {
        self.counts.remove(scope);
    }
    pub fn update(&mut self, scope: &RoomScope, unread: u64) -> Option<Notification> {
        if scope.account_id != self.account {
            return None;
        }
        let previous = self.counts.insert(scope.clone(), unread)?;
        if !self.enabled || self.muted(scope) || unread <= previous {
            return None;
        }
        Some(Notification {
            count: self.count_preview.then_some(unread - previous),
            scope: scope.clone(),
            origin: None,
        })
    }
}

/// Only fixed strings and counts may cross the OS boundary; never message data.
pub struct Notification {
    count: Option<u64>,
    /// The room the alert is about (the worker's own account ID).
    scope: RoomScope,
    /// Catalog ID and display name of the account it came from; set by the window.
    origin: Option<(AccountId, String)>,
}
impl Notification {
    /// Names the account the alert came from; its name becomes the title.
    pub fn from_account(mut self, id: AccountId, name: String) -> Self {
        self.origin = Some((id, name));
        self
    }
    /// Where a click goes: the catalog account (None when unnamed) and the room.
    pub fn target(&self) -> (Option<AccountId>, &RoomScope) {
        (self.origin.as_ref().map(|(id, _)| *id), &self.scope)
    }
    pub fn title(&self) -> &str {
        self.origin.as_ref().map_or("Gnobbi", |(_, name)| name)
    }
    pub fn body(&self) -> String {
        match self.count {
            None => "Nuovi messaggi".into(),
            Some(100..) => "99+ nuovi messaggi".into(),
            Some(count) => format!("{count} nuovi messaggi"),
        }
    }
}
