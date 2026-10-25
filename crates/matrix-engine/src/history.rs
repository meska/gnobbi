//! View-scoped read-only history DTOs. Opaque cursors are not event IDs.
use crate::RequestTag;
use matrix_domain::EventId;

pub struct HistoryRequest {
    pub tag: RequestTag,
    pub before: Option<String>,
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

pub struct HistoryEvent {
    pub event_id: EventId,
    pub body: String,
    pub outgoing: bool,
    pub timestamp_ms: u64,
    pub sender: HistorySender,
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
