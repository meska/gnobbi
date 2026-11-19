//! UI-, transport- and SDK-independent domain contracts.

pub mod design;
pub mod notifications;
mod poll;
pub mod role;
pub use poll::{Poll, PollAnswer, PollDraft, PollDraftError};
mod sender_label;
mod timeline;
pub use sender_label::{SENDER_LABEL_GAP_MS, shows_sender_label};
pub use timeline::{SendStatus, Timeline, TimelineError, TimelineItem, TimelineUpdate};

/// Whitespace, control, backslash, zero-width and bidi characters: never allowed in a URL the user
/// is shown or that opens something (shared by web links and Matrix links).
pub fn has_unsafe_url_char(s: &str) -> bool {
    s.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(c, '\\' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
    })
}

/// Timeline identity explicitly tagged as local pending or server-assigned.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TimelineId {
    Pending(TransactionId),
    Server(EventId),
}

/// Identity-only DTO, also used by the separate timeline projection.
/// Equality and hashing include account, room and identity kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopedTimelineId {
    pub scope: RoomScope,
    pub id: TimelineId,
}

/// Explicit local account and room scope for cross-boundary DTO identities.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RoomScope {
    pub account_id: AccountId,
    pub room_id: RoomId,
}

/// Opaque room identifier supplied by an adapter, preserved verbatim.
///
/// This is a typed transport value, not a Matrix syntax validator. Network
/// adapters must validate untrusted identifiers before constructing it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RoomId(String);

impl RoomId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque server event identifier. No syntax validation or normalization.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventId(String);

impl EventId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Caller-allocated pending-send identity, not a server event identifier.
/// No generation, escaping or syntax validation is performed here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TransactionId(String);

impl TransactionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque local account identity, not a Matrix user ID or credential.
///
/// The caller allocates values in its own account registry. Every `u64` value
/// is supported; this type does not parse or validate server-side identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(u64);

impl AccountId {
    /// Wrap a caller-allocated local identity without transforming it.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the exact local identity value.
    pub const fn as_u64(&self) -> u64 {
        self.0
    }
}

/// A validated `matrix:` / matrix.to link; see `matrix_engine::matrix_link::parse_matrix_link`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixLink {
    /// `target` is `#alias:server` or `!id:server`.
    Room {
        target: String,
        via: Vec<String>,
        event: Option<String>,
    },
    User {
        user_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    Invalid,
}
