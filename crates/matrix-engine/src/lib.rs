// The Timeline futures nest deeper than the default 128 once awaited in the operations worker.
#![recursion_limit = "256"]

//! Headless backend/UI boundary with an opt-in Matrix SDK adapter surface.
//!
//! Enable `sdk` only in backend consumers; domain/UI DTOs remain SDK-free.
//! This re-export is the adapter integration point, not login/sync support.
//! It does not start a runtime or create a client on the UI thread.
#[cfg(feature = "sdk")]
pub use matrix_sdk as sdk;
#[cfg(feature = "sdk")]
pub use matrix_sdk_ui as sdk_ui;
#[cfg(feature = "sdk")]
pub mod accounts;
#[cfg(feature = "sdk")]
pub mod auth;
pub mod avatar;
#[cfg(feature = "sdk")]
pub mod avatar_cache;
#[cfg(feature = "sdk")]
pub mod recovery;
#[cfg(feature = "sdk")]
mod outbox;
#[cfg(feature = "sdk")]
pub mod room_sync;
pub mod message_match;
#[cfg(feature = "sdk")]
pub mod message_search;
#[cfg(feature = "sdk")]
pub mod notify;
#[cfg(feature = "sdk")]
pub mod room_tags;
pub mod search;
#[cfg(feature = "sdk")]
pub mod sliding;
#[cfg(feature = "sdk")]
pub mod threads;
#[cfg(feature = "sdk")]
pub mod user_search;
pub mod verification;
#[cfg(feature = "sdk")]
mod warm;

/// A server-backed room-list projection; never log remote names or identities.
#[derive(Clone, PartialEq, Eq)]
pub struct RoomSummary {
    pub scope: matrix_domain::RoomScope,
    pub name: String,
    /// Server notification count, not a locally inferred read receipt.
    pub unread: u64,
    pub info: RoomInfo,
}

/// Snapshot from SDK state after /sync. No remote payload formatting traits.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RoomInfo {
    /// Observed public peer markers only; empty means no known marker, not unread.
    pub public_read_receipts: Vec<receipts::PublicReadReceipt>,
    pub topic: Option<String>,
    /// mxc URI of the avatar to show: the room's own, else the other person's in a 1:1 DM; None means initials. Not downloaded here.
    pub avatar: Option<String>,
    /// Text of the SDK latest event when it is a message; None otherwise.
    pub preview: Option<String>,
    /// Timestamp (ms since epoch) of the SDK latest event; None when the room has none yet.
    pub last_event_ts: Option<u64>,
    /// The user has an invitation to this room and has not answered it. Listed by the SDK
    /// filter; the other fields describe the room as the invite shows it.
    pub invited: bool,
    pub joined_members: u64,
    /// The user flagged the room unread by hand (SDK marker), independent of the counters.
    pub marked_unread: bool,
    /// Server highlight (mention) count; shown with the unread badge.
    pub mentions: u64,
    pub is_space: bool,
    /// Direct m.space.child links with valid nonempty via; never autojoin.
    pub children: Vec<matrix_domain::RoomId>,
    /// None means the SDK has insufficient state, not unencrypted.
    pub encrypted: Option<bool>,
    /// Group tags of the room (`m.favourite`, `m.lowpriority`, `u.*`), sorted; see `room_tags`.
    pub tags: Vec<String>,
    /// Muted by the user's push rules (`NotificationSettings`), on every device.
    pub muted: bool,
    /// Canonical alias then alternative aliases, as the SDK room info has them (may hold
    /// empty strings from servers that send them; callers drop those).
    pub aliases: Vec<String>,
    /// Direct room: the peer's Matrix user ID from `m.direct` (SDK `direct_targets`).
    pub dm_user_id: Option<String>,
}

/// Stable-ID updates independent of the current UI filter or selection.
pub enum RoomListDiff {
    // Boxed: RoomSummary is large next to Remove(RoomScope) (clippy::large_enum_variant).
    Upsert(Box<RoomSummary>),
    Remove(RoomScope),
}

pub enum RoomListEvent {
    Diff(RoomListDiff),
    Status(RoomSyncStatus),
    /// Content-free invalidation, independent of room-summary equality or selection.
    TimelineActivity(RoomScope),
    /// Every listed room, most recent first: the `RoomListService` sorter order (unsent
    /// message, then recency, then name). Sent after the diffs it refers to.
    Order(Vec<RoomScope>),
    /// The push rules say a new message in this room notifies (content-free).
    Alert(RoomScope),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoomSyncStatus {
    Connecting,
    Ready,
    AuthenticationFailed,
    SyncFailed,
}
#[cfg(feature = "sdk")]
pub mod discovery;
mod generation;
pub mod history;
pub mod status_line;
#[cfg(any(
    all(feature = "mac-vault", target_os = "macos"),
    all(feature = "ios-vault", target_os = "ios")
))]
pub mod keychain;
pub mod media;
#[cfg(feature = "sdk")]
pub mod media_sdk;
pub mod operations;
#[cfg(feature = "sdk")]
mod fsx;
#[cfg(feature = "sdk")]
pub mod persistence;
pub mod receipts;
pub mod reconnect;
pub mod rich_text;
pub mod sending;
#[cfg(feature = "sdk")]
pub mod signin;
#[cfg(all(feature = "linux-vault", target_os = "linux"))]
pub mod vault;
#[cfg(all(feature = "windows-vault", windows))]
pub mod wincred;
#[cfg(feature = "sdk")]
pub mod video_file;
pub use generation::{GenerationExhausted, RequestTag, ViewRequests};
mod channel;
mod worker_control;
pub use channel::{Receiver, Sender, TryReceiveError, TrySendError, bounded};
use matrix_domain::{EventId, RoomScope, TimelineUpdate, TransactionId};
pub use worker_control::WorkerControl;

/// Typed UI requests. Message bodies are private data, never log these DTOs.
pub enum EngineCommand {
    LoadHistory {
        tag: RequestTag,
        before: Option<EventId>,
    },
    SendText {
        scope: RoomScope,
        transaction_id: TransactionId,
        body: String,
    },
}

/// Backend events distinguish view data from durable account/room updates.
/// Route Timeline updates even after navigating away; never log DTO bodies.
pub enum EngineEvent {
    /// Ephemeral view result; consumer must check ViewRequests::accepts.
    HistoryItem {
        tag: RequestTag,
        event_id: EventId,
        body: String,
    },
    Timeline {
        scope: RoomScope,
        update: TimelineUpdate,
    },
}
