//! Opening a room with unread messages on the first one, and moving the read marker forward as the rows pass the viewport.
use super::*;
use matrix_desktop::read_track::{self, Anchor, ReadRow, ReadTracker};

/// Longest wait for the engine and the older pages before the room shows at the bottom.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(20);

/// The room on screen was opened with unread messages and is read message by message.
pub(super) struct UnreadOpen {
    scope: matrix_domain::RoomScope,
    /// The room list's count when it was opened.
    unread0: u64,
    /// The position saved the last time: kept when it is already past the first unread.
    saved: Option<(EventId, f32)>,
    /// `None` until the engine answered.
    markers: Option<Vec<EventId>>,
    /// Opened at: a room never stays hidden for more than `PATIENCE`.
    since: std::time::Instant,
    anchored: bool,
    /// The newest event known read (the receipt last sent, or the marker found at opening).
    read_id: Option<String>,
    tracker: ReadTracker,
}

impl Session {
    /// A room was just selected: with unread messages the first one is located before the room shows.
    pub(super) fn start_unread(&mut self) {
        self.unread_open = None;
        let Some(room) = self.sidebar.selected().filter(|r| r.unread > 0) else { return };
        let (scope, unread0) = (room.scope.clone(), room.unread);
        self.op_queue.push(OperationCommand::ReadMarkers { scope: scope.clone() });
        // Held hidden (the row is never found) until the anchor is known.
        let saved = self.heights.restore.replace((EventId::new(""), 0.));
        self.unread_open = Some(UnreadOpen { scope, unread0, saved, markers: None, since: std::time::Instant::now(), anchored: false, read_id: None, tracker: ReadTracker::default() });
    }

    /// While the first unread is being located nothing may mark the room read.
    pub(super) fn unread_pending(&self) -> bool {
        self.unread_open.as_ref().is_some_and(|u| !u.anchored)
    }

    pub(super) fn unread_markers(&mut self, scope: &matrix_domain::RoomScope, markers: Result<Vec<EventId>, matrix_engine::operations::OperationError>) {
        let Some(u) = self.unread_open.as_mut().filter(|u| &u.scope == scope) else { return };
        match markers {
            Ok(m) => u.markers = Some(m),
            // Unknown marker: the room opens as it always did.
            Err(_) => {
                self.heights.restore = u.saved.take();
                self.unread_open = None;
            }
        }
    }

    /// Once per frame: places the open, then follows what is in view. True when the sidebar changed.
    pub(super) fn drive_unread(&mut self) -> bool {
        let selected = self.sidebar.selected().map(|r| r.scope.clone());
        let Some(u) = self.unread_open.as_mut() else { return false };
        let Some(messages) = &self.messages else { return false };
        if selected.as_ref() != Some(&u.scope) {
            self.unread_open = None;
            return false;
        }
        let rows = messages.events();
        let patience = u.since.elapsed() > PATIENCE;
        if !u.anchored {
            if patience && u.markers.is_none() {
                // The engine never answered: the room opens as it always did.
                self.heights.restore = u.saved.take();
                self.unread_open = None;
                return false;
            }
            let Some(markers) = u.markers.as_ref().map(|m| m.iter().map(EventId::as_str).collect::<Vec<_>>()) else { return false };
            let status = self.history.status();
            let complete = status == matrix_desktop::HistoryStatus::Start && !rows.is_empty();
            let exhausted = self.heights.restore_wait.0 >= read_track::MAX_PAGES || status == matrix_desktop::HistoryStatus::Failed || patience;
            match read_track::locate(rows, &markers, u.unread0, (complete, exhausted), read_track::CONTEXT_ROWS) {
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
                        self.notice = Some(i18n::t("Unread messages not found").to_owned());
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
