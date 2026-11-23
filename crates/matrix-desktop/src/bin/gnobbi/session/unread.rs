//! Opening a room with unread messages on the first one, and moving the read marker forward as the rows pass the viewport.
use super::*;
use matrix_desktop::read_track::{self, Anchor, ReadRow, ReadTracker};

/// Delay between pagination chunks and retries, to avoid a tight offline loop.
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);
/// Longest the saved rows stay hidden while the first unread is looked for: past it the room shows them at the bottom (the same as a missing marker).
const HIDE_MAX: std::time::Duration = std::time::Duration::from_secs(2);

/// The room on screen was opened with unread messages and is read message by message.
pub(super) struct UnreadOpen {
    scope: matrix_domain::RoomScope,
    /// The room list's count when it was opened.
    unread0: u64,
    /// The position saved the last time: kept when it is already past the first unread.
    saved: Option<(EventId, f32)>,
    /// `None` until the engine answered.
    markers: Option<Vec<EventId>>,
    /// Next allowed retry after a failed marker lookup or pagination chunk.
    retry_at: Option<std::time::Instant>,
    anchored: bool,
    opened: std::time::Instant,
    /// The newest event known read (the receipt last sent, or the marker found at opening).
    read_id: Option<String>,
    tracker: ReadTracker,
}

impl Session {
    /// A room was just selected: with unread messages the first one is located before the room shows.
    pub(super) fn start_unread(&mut self) {
        self.unread_open = None;
        self.unread_notice = None;
        let Some(room) = self.sidebar.selected().filter(|r| r.unread > 0) else { return };
        let (scope, unread0) = (room.scope.clone(), room.unread);
        self.op_queue.push(OperationCommand::ReadMarkers { scope: scope.clone() });
        // Held hidden (the row is never found) until the anchor is known.
        let saved = self.heights.restore.replace((EventId::new(""), 0.));
        self.unread_open = Some(UnreadOpen { scope, unread0, saved, markers: None, retry_at: None, anchored: false, opened: std::time::Instant::now(), read_id: None, tracker: ReadTracker::default() });
    }

    /// A room that had unread messages and has none now (read here, or by a receipt synced from another device, also after
    /// coming back from the background): its delivered alerts go. A zero never seen above zero is the cache's word: left alone.
    pub(super) fn clear_read_alerts(&mut self) {
        let rooms: Vec<_> = self.sidebar.rooms().iter().map(|r| (&r.scope, r.unread)).collect();
        let clear = matrix_desktop::notifications::alerts_to_clear(&rooms, &mut self.alerted_unread);
        matrix_desktop::notifications::clear_rooms(self.catalog_id, clear);
    }

    /// Unread messages of the account's unmuted rooms, for the app icon badge.
    pub(crate) fn unread_total(&self) -> u64 {
        self.sidebar.rooms().iter().filter(|r| !r.info.invited && !r.info.is_space && !r.info.muted()).fold(0, |n, r| n.saturating_add(r.unread))
    }

    /// While the first unread is being located nothing may mark the room read.
    pub(crate) fn unread_pending(&self) -> bool {
        self.unread_open.as_ref().is_some_and(|u| !u.anchored)
    }

    pub(super) fn unread_markers(&mut self, scope: &matrix_domain::RoomScope, markers: Result<Vec<EventId>, matrix_engine::operations::OperationError>) {
        let Some(u) = self.unread_open.as_mut().filter(|u| &u.scope == scope) else { return };
        match markers {
            Ok(m) => u.markers = Some(m),
            Err(_) => u.retry_at = Some(std::time::Instant::now() + RETRY),
        }
    }

    /// User intent cancels the search without reporting a missing marker.
    pub(crate) fn cancel_unread_search(&mut self) {
        self.unread_notice = None;
        if self.unread_pending() {
            self.unread_open = None;
            self.heights.restore = None;
        }
    }

    /// Continue in bounded chunks; errors and chunk boundaries both back off.
    pub(super) fn unread_history_step(&mut self) -> Option<matrix_engine::history::HistoryRequest> {
        let u = self.unread_open.as_mut().filter(|u| !u.anchored && u.markers.is_some())?;
        let status = self.history.status();
        if !matches!(status, matrix_desktop::HistoryStatus::More | matrix_desktop::HistoryStatus::Failed) { return None; }
        let now = std::time::Instant::now();
        if status == matrix_desktop::HistoryStatus::Failed || self.heights.restore_wait.0 >= read_track::MAX_PAGES {
            let at = *u.retry_at.get_or_insert(now + RETRY);
            if now < at { return None; }
            u.retry_at = None;
            self.heights.restore_wait.0 = 0;
        }
        self.history.load_older()
    }

    /// Once per frame: places the open, then follows what is in view. True when the sidebar changed.
    pub(super) fn drive_unread(&mut self) -> bool {
        let current = self.sidebar.selected().map(|r| (r.scope.clone(), r.unread));
        let Some(u) = self.unread_open.as_mut() else { return false };
        let Some(messages) = &self.messages else { return false };
        if current.as_ref().map(|(scope, _)| scope) != Some(&u.scope) {
            self.unread_open = None;
            return false;
        }
        let rows = messages.events();
        if !u.anchored {
            if !rows.is_empty() && u.opened.elapsed() >= HIDE_MAX {
                self.heights.restore = None;
                self.unread_notice = Some(std::time::Instant::now());
                self.unread_open = None;
                return false;
            }
            if u.markers.is_none() && u.retry_at.is_some_and(|at| std::time::Instant::now() >= at) {
                self.op_queue.push(OperationCommand::ReadMarkers { scope: u.scope.clone() });
                u.retry_at = None;
            }
            let Some(markers) = u.markers.as_ref().map(|m| m.iter().map(EventId::as_str).collect::<Vec<_>>()) else { return false };
            let status = self.history.status();
            let complete = self.history.confirmed_start();
            let unread = current.as_ref().map_or(u.unread0, |(_, count)| *count);
            match read_track::locate(rows, &markers, unread, (complete, false), read_track::CONTEXT_ROWS) {
                Anchor::Wait if status == matrix_desktop::HistoryStatus::Start && !complete => {
                    self.heights.restore = None;
                    self.unread_open = None;
                    return false;
                }
                Anchor::Wait => return false,
                Anchor::At { upto } => {
                    let first = read_track::first_unread(rows, upto).and_then(|i| rows[i].event_id());
                    let saved_past = u.saved.as_ref().and_then(|(id, _)| rows.iter().position(|e| e.id.id == TimelineId::Server(id.clone()))).is_some_and(|i| i >= upto);
                    u.anchored = true;
                    u.read_id = rows[..upto].iter().rev().find_map(|e| e.event_id()).map(str::to_owned);
                    self.heights.divider = first.map(EventId::new);
                    if saved_past {
                        self.heights.restore = u.saved.take();
                    } else {
                        self.heights.restore = first.map(|id| (EventId::new(id), 0.));
                        self.heights.restore_unread = true;
                    }
                }
                done @ (Anchor::Bottom | Anchor::NotFound) => {
                    self.heights.restore = if done == Anchor::Bottom { u.saved.take() } else { None };
                    if done == Anchor::NotFound {
                        self.unread_notice = Some(std::time::Instant::now());
                    }
                    self.unread_open = None;
                    return false;
                }
            }
        }
        let read = u.read_id.as_deref().and_then(|id| read_track::marker_index(rows, &[id]));
        let upto = read_track::read_upto(rows, read);
        let local = read_track::unread_after(rows, upto);
        let (focused, _, public) = self.reading;
        let changed = self.sidebar.set_unread(&u.scope, local);
        if !focused {
            u.tracker.reset();
            self.heights.watching = false;
            return changed;
        }
        // The frame that drew them may be older than the rows: only the rows still where they were count.
        let visible: Vec<(usize, &str)> = self.heights.visible.iter().filter(|(i, id)| rows.get(*i).and_then(|e| e.event_id()) == Some(id.as_str())).map(|(i, id)| (*i, id.as_str())).collect();
        let step = u.tracker.step(&visible, upto, std::time::Instant::now());
        self.heights.watching = step.waiting;
        if let Some((_, id)) = step.send {
            u.read_id = Some(id.clone());
            self.op_queue.push(OperationCommand::MarkReadTo { scope: u.scope.clone(), target: EventId::new(id), public });
        }
        changed
    }
}
