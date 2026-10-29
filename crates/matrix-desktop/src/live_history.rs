//! SDK/GPUI-free lifecycle for the selected read-only server timeline.
use matrix_domain::{EventId, RoomScope};
use matrix_engine::{
    ViewRequests,
    history::{HistoryEvent, HistoryRequest, HistoryResult},
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HistoryStatus {
    Empty,
    Loading,
    More,
    Start,
    Failed,
}

impl HistoryStatus {
    /// Visible strip text; the keyboard shortcut and the accessible action carry no on-screen label.
    pub fn strip_label(self) -> Option<&'static str> {
        match self {
            Self::Loading => Some("Caricamento…"),
            Self::Failed => Some("Impossibile caricare · scorri o clicca per riprovare"),
            _ => None,
        }
    }
}

/// How long after new rows a room with threads is read once more.
pub const SETTLE: std::time::Duration = std::time::Duration::from_millis(1500);

pub struct HistorySession {
    views: ViewRequests,
    pagination_tag: Option<matrix_engine::RequestTag>,
    scope: Option<RoomScope>,
    next: Option<String>,
    status: HistoryStatus,
    /// Body and thread reply count: a new reply must hand the root over again.
    seen: HashMap<EventId, (String, Option<u32>)>,
    live_views: ViewRequests,
    live_pending: bool,
    dirty: bool,
    /// One more refresh due at this time: the SDK updates a thread root's summary after the reply row.
    settle: Option<std::time::Instant>,
    live_failed: bool,
    /// Top row seen since the older page failed; a different row means the reader scrolled.
    failed_row: Option<usize>,
    live_before: Option<String>,
    live_rows: Vec<HistoryEvent>,
    live_cursors: HashSet<String>,
}
impl Default for HistorySession {
    fn default() -> Self {
        Self::new()
    }
}
impl HistorySession {
    pub fn new() -> Self {
        Self {
            views: ViewRequests::new(),
            pagination_tag: None,
            scope: None,
            next: None,
            status: HistoryStatus::Empty,
            seen: HashMap::new(),
            live_views: ViewRequests::new(),
            live_pending: false,
            dirty: false,
            settle: None,
            live_failed: false,
            failed_row: None,
            live_before: None,
            live_rows: Vec::new(),
            live_cursors: HashSet::new(),
        }
    }
    /// None cancels on removal of the selected room. Same-room sync is a no-op.
    pub fn select(&mut self, scope: Option<RoomScope>) -> Option<HistoryRequest> {
        if self.scope == scope {
            return None;
        }
        self.views.cancel();
        self.live_views.cancel();
        self.live_pending = false;
        self.dirty = false;
        self.live_failed = false;
        self.failed_row = None;
        self.live_before = None;
        self.live_rows.clear();
        self.live_cursors.clear();
        self.pagination_tag = None;
        self.scope = scope.clone();
        self.next = None;
        self.seen.clear();
        self.status = HistoryStatus::Empty;
        let tag = self.views.start(scope?).ok()?;
        self.pagination_tag = Some(tag.clone());
        self.status = HistoryStatus::Loading;
        Some(HistoryRequest {
            tag,
            before: None,
            open: true,
        })
    }
    pub fn apply(&mut self, result: HistoryResult) -> Option<Vec<HistoryEvent>> {
        if !self.views.accepts(&result.tag) {
            return None;
        }
        match result.page {
            Ok(page) if self.views.accepts(&page.tag) => {
                self.next = page.next;
                self.status = if self.next.is_some() {
                    HistoryStatus::More
                } else {
                    HistoryStatus::Start
                };
                Some(
                    page.events
                        .into_iter()
                        .filter(|event| {
                            if self.seen.contains_key(&event.event_id) {
                                return false;
                            }
                            self.seen.insert(event.event_id.clone(), (event.body.clone(), event.thread.as_ref().map(|t| t.num_replies)));
                            true
                        })
                        .collect(),
                )
            }
            _ => {
                self.status = HistoryStatus::Failed;
                None
            }
        }
    }
    /// Coalesce only the selected view; later selection fetches its own current page.
    pub fn activity(&mut self, scope: &RoomScope) {
        if self.scope.as_ref() == Some(scope) {
            self.dirty = true;
        }
    }

    /// One history job at a time, without replacing the older-history cursor.
    pub fn refresh(&mut self) -> Option<HistoryRequest> {
        if self.settle.is_some_and(|at| at <= std::time::Instant::now()) {
            self.settle = None;
            self.dirty = true;
        }
        if (!self.dirty && self.live_before.is_none())
            || self.live_pending
            || self.status == HistoryStatus::Loading
        {
            return None;
        }
        let tag = self.live_views.start(self.scope.clone()?).ok()?;
        if self.live_before.is_none() {
            self.dirty = false;
        }
        self.live_pending = true;
        self.live_failed = false;
        Some(HistoryRequest {
            tag,
            before: self.live_before.clone(),
            open: false,
        })
    }

    pub fn is_live(&self, tag: &matrix_engine::RequestTag) -> bool {
        self.live_pending && self.live_views.accepts(tag)
    }

    pub fn apply_live(&mut self, result: HistoryResult) -> Option<Vec<HistoryEvent>> {
        if !self.live_pending || !self.live_views.accepts(&result.tag) {
            return None;
        }
        self.live_pending = false;
        let page = match result.page {
            Ok(page) if self.live_views.accepts(&page.tag) => page,
            _ => {
                self.live_views.cancel();
                self.live_before = None;
                self.live_rows.clear();
                self.live_cursors.clear();
                self.live_failed = true;
                return None;
            }
        };
        self.live_views.cancel();
        // Stop at the newest loaded ID: older unseen rows belong to pagination, not append.
        let overlap = page
            .events
            .iter()
            .rposition(|event| self.seen.contains_key(&event.event_id));
        let mut rows: Vec<_> = page
            .events
            .into_iter()
            .enumerate()
            .filter(|(index, event)| {
                overlap.is_none_or(|overlap| *index > overlap)
                    || self.seen.contains_key(&event.event_id)
            })
            .map(|(_, event)| event)
            .collect();
        rows.append(&mut self.live_rows);
        self.live_rows = rows;
        if overlap.is_none()
            && let Some(next) = page.next
        {
            if !self.live_cursors.insert(next.clone()) {
                self.live_rows.clear();
                self.live_before = None;
                self.live_cursors.clear();
                self.live_failed = true;
                return None;
            }
            self.live_before = Some(next);
            return None;
        }
        self.live_before = None;
        self.live_cursors.clear();
        let published: Vec<_> = std::mem::take(&mut self.live_rows)
                .into_iter()
                .filter(|event| {
                    if self.seen.get(&event.event_id).is_some_and(|(b, n)| *b == event.body && *n == event.thread.as_ref().map(|t| t.num_replies)) {
                        return false;
                    }
                    self.seen.insert(event.event_id.clone(), (event.body.clone(), event.thread.as_ref().map(|t| t.num_replies)));
                    true
                })
                .collect();
        // Only rooms that have threads pay for the second look (shortcut: a fixed delay, no stream).
        if !published.is_empty() && self.seen.values().any(|(_, n)| n.is_some()) {
            self.settle = Some(std::time::Instant::now() + SETTLE);
        }
        Some(published)
    }

    pub fn status(&self) -> HistoryStatus {
        if self.live_failed {
            HistoryStatus::Failed
        } else {
            self.status
        }
    }
    /// Opaque lifecycle identity, never a server cursor or credential.
    pub fn pagination_tag(&self) -> Option<matrix_engine::RequestTag> {
        self.pagination_tag.clone()
    }
    /// Queued UI/assistive intents cannot advance a newer page or room.
    pub fn load_older_if_current(
        &mut self,
        tag: &matrix_engine::RequestTag,
    ) -> Option<HistoryRequest> {
        if !self.views.accepts(tag) {
            return None;
        }
        self.load_older()
    }
    /// Rows from the oldest loaded one within which scrolling starts the previous page by itself.
    pub const AUTO_LOAD_ROWS: usize = 20;

    /// Scroll-driven paging: only from a healthy `More` state, so a failed page waits for an explicit retry.
    pub fn load_older_near_top(&mut self, first_visible_row: usize) -> Option<HistoryRequest> {
        if first_visible_row > Self::AUTO_LOAD_ROWS {
            return None;
        }
        if self.status == HistoryStatus::Failed && !self.live_failed {
            // A failed page retries once per scroll, never per tick.
            if self
                .failed_row
                .replace(first_visible_row)
                .is_none_or(|row| row == first_visible_row)
            {
                return None;
            }
        } else if self.status != HistoryStatus::More {
            return None;
        }
        self.load_older()
    }

    pub fn load_older(&mut self) -> Option<HistoryRequest> {
        if self.live_failed {
            self.dirty = true;
            return None;
        }
        if self.live_pending
            || self.live_before.is_some()
            || !matches!(self.status, HistoryStatus::More | HistoryStatus::Failed)
        {
            return None;
        }
        let tag = self.views.start(self.scope.clone()?).ok()?;
        self.pagination_tag = Some(tag.clone());
        self.failed_row = None;
        self.status = HistoryStatus::Loading;
        Some(HistoryRequest {
            tag,
            before: self.next.clone(),
            open: false,
        })
    }
}
