//! View-scoped read-only history DTOs. Opaque cursors are not event IDs.
use crate::RequestTag;
use matrix_domain::EventId;

pub struct HistoryRequest {
    pub tag: RequestTag,
    pub before: Option<String>,
    /// Room opening: saved messages first, then only what was missed.
    pub open: bool,
}

/// Event author: Matrix user ID plus display name when the room member is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySender {
    pub id: String,
    pub name: Option<String>,
    /// mxc URI of the member avatar when the room member has one; not downloaded here.
    pub avatar: Option<String>,
}

impl HistorySender {
    pub fn new(id: &str, name: Option<&str>) -> Self {
        Self {
            id: id.to_owned(),
            name: name
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned),
            avatar: None,
        }
    }

    pub fn with_avatar(mut self, avatar: Option<String>) -> Self {
        self.avatar = avatar;
        self
    }
}

/// Body of an image message row; the UI draws the picture instead of text.
// shortcut: a user typing exactly this text is shown a failed image, upgrade when rows carry a kind.
pub const IMAGE_BODY: &str = "\u{fffc}Immagine";

pub fn is_image_body(body: &str) -> bool {
    body == IMAGE_BODY
}

/// «m:ss» of a duration, as drawn on a video bubble.
pub fn duration_label(duration_ms: u64) -> String {
    let s = duration_ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// Video facts from the event `info`; the thumbnail is only a source to fetch later.
#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub duration_ms: Option<u64>,
    pub size: Option<u64>,
    #[cfg(feature = "sdk")]
    pub thumbnail: Option<matrix_sdk::ruma::events::room::MediaSource>,
}

pub struct HistoryEvent {
    pub event_id: EventId,
    pub body: String,
    pub outgoing: bool,
    pub timestamp_ms: u64,
    pub sender: HistorySender,
    pub video: Option<VideoInfo>,
    /// Bold, italic, code… of a formatted body, as byte ranges of `body` (empty for plain text).
    pub styles: Vec<crate::rich_text::StyleSpan>,
    /// Set on the root of a thread.
    pub thread: Option<ThreadInfo>,
    /// Set on a reply inside a thread: the thread's root.
    pub thread_root: Option<EventId>,
}

/// The summary under a thread root, as the Timeline keeps it (live).
// shortcut: the SDK gives the count and the latest reply only, so one participant; upgrade if it exposes all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfo {
    pub num_replies: u32,
    pub latest_sender: Option<HistorySender>,
    /// Replies by others after our own read marker of the thread (0 while the thread is unknown).
    pub unread: u32,
}

/// One thread opened: `root` is None when the room's own timeline does not hold it (the thread
/// timeline alone does not return it reliably); `replies` are oldest first.
pub struct ThreadPage {
    pub root: Option<HistoryEvent>,
    pub replies: Vec<HistoryEvent>,
    pub reactions: Vec<ReactionCount>,
    /// The thread timeline says its start was reached. Servers get this wrong when the thread is
    /// bigger than the sync window: compare `replies.len()` with `ThreadSummary::num_replies`.
    pub at_start: bool,
}

/// How many people reacted to a message with one emoji.
pub struct ReactionCount {
    pub target: EventId,
    pub key: String,
    pub count: usize,
}

pub struct HistoryPage {
    pub tag: RequestTag,
    /// Chronological order, oldest first. No body/ID/cursor Debug or logging.
    pub events: Vec<HistoryEvent>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryError {
    Cancelled,
    WrongScope,
    Unavailable,
    Transport,
}

pub struct HistoryResult {
    pub tag: RequestTag,
    pub page: Result<HistoryPage, HistoryError>,
}

#[cfg(test)]
mod sender_avatar_tests {
    use super::*;

    #[test]
    fn avatar_is_absent_unless_given() {
        assert_eq!(HistorySender::new("@a:t", None).avatar, None);
        let s = HistorySender::new("@a:t", None).with_avatar(Some("mxc://t/a".into()));
        assert_eq!(s.avatar.as_deref(), Some("mxc://t/a"));
    }
}

#[cfg(test)]
mod image_body_tests {
    use super::*;

    #[test]
    fn only_the_image_marker_is_an_image_body() {
        assert!(is_image_body(IMAGE_BODY));
        assert!(!is_image_body("ciao"));
        assert!(!is_image_body(""));
    }
}
