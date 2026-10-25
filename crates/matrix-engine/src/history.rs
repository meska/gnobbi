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
}

impl HistorySender {
    pub fn new(id: &str, name: Option<&str>) -> Self {
        Self {
            id: id.to_owned(),
            name: name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned),
        }
    }
}

/// Body of an image message row; the UI draws the picture instead of text.
// shortcut: a user typing exactly this text is shown a failed image, upgrade when rows carry a kind.
pub const IMAGE_BODY: &str = "\u{fffc}Immagine";

pub fn is_image_body(body: &str) -> bool {
    body == IMAGE_BODY
}

pub struct HistoryEvent {
    pub event_id: EventId,
    pub body: String,
    pub outgoing: bool,
    pub timestamp_ms: u64,
    pub sender: HistorySender,
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
mod image_body_tests {
    use super::*;

    #[test]
    fn only_the_image_marker_is_an_image_body() {
        assert!(is_image_body(IMAGE_BODY));
        assert!(!is_image_body("ciao"));
        assert!(!is_image_body(""));
    }
}
