//! Conservative, content-free local unread alerts. No SDK, UI or OS dependencies.
use crate::{AccountId, RoomScope};

/// What the OS may show; which messages notify (and which rooms are muted) is decided by the
/// account's push rules, through the SDK.
pub struct NotificationPolicy {
    account: AccountId,
    enabled: bool,
    count_preview: bool,
}
impl NotificationPolicy {
    pub fn new(account: AccountId) -> Self {
        Self {
            account,
            enabled: false,
            count_preview: false,
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
    /// The alert for one message that the push rules say notifies in `scope`.
    pub fn alert(&self, scope: &RoomScope) -> Option<Notification> {
        (self.enabled && scope.account_id == self.account).then(|| Notification {
            count: self.count_preview.then_some(1),
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
            Some(1) => "1 nuovo messaggio".into(),
            Some(100..) => "99+ nuovi messaggi".into(),
            Some(count) => format!("{count} nuovi messaggi"),
        }
    }
}
