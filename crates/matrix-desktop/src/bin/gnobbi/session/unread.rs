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

    /// The room was marked read up to its newest row: the open's own read id follows, or its local count lights the badge again.
    pub(super) fn unread_follow_read(&mut self, scope: &matrix_domain::RoomScope) {
        use matrix_desktop::read_track::ReadRow;
        let last = self.messages.as_ref().and_then(|m| m.events().iter().rev().find_map(|e| e.event_id().map(str::to_owned)));
        if let Some(u) = self.unread_open.as_mut().filter(|u| u.anchored && &u.scope == scope) {
            u.read_id = last.or(u.read_id.take());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
    use std::time::{Duration, Instant};

    const ROWS: usize = 6;

    /// An offline session with the room `!r` of account `account` open and `unread` rows after the read id (read id = the row before them).
    /// Focused, newest row in view, receipts public: the state of a room open on the screen.
    fn open_room(name: &str, account: u64, unread: usize) -> (Session, matrix_domain::RoomScope) {
        let root = std::env::temp_dir().join(format!("gnobbi-receipt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = AppDirs { config: root.join("c"), data: root.join("d"), cache: root.join("k") };
        let mut s = Session::start(&dirs, root.join("s"), None).expect("offline session");
        let acc = matrix_domain::AccountId::new(account);
        let scope = matrix_domain::RoomScope { account_id: acc, room_id: matrix_domain::RoomId::new("!r:t.invalid") };
        s.account = Some(acc);
        s.sidebar = matrix_desktop::Sidebar::connected(acc);
        s.sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary { scope: scope.clone(), name: "r".into(), unread: unread as u64, info: RoomInfo::default() })));
        assert!(s.sidebar.select(&scope));
        let history = DemoHistory::new(scope.clone(), ROWS);
        // The demo rows alternate own and foreign ones: read id = the row before the unread ones, counted by the real rule.
        let read_id = history.events()[ROWS - unread - 1].event_id().map(str::to_owned);
        s.messages = Some(history);
        s.unread_open = Some(UnreadOpen { scope: scope.clone(), unread0: unread as u64, saved: None, markers: None, retry_at: None, anchored: true, opened: Instant::now(), read_id, tracker: ReadTracker::default() });
        s.reading = (true, true, true);
        s.read_dirty = false;
        (s, scope)
    }

    /// What the app does on `TimelineActivity` for the open room.
    fn new_event(s: &mut Session) {
        s.read_dirty = true;
        s.read_after = Some(Instant::now() + READ_DEBOUNCE);
    }

    /// The debounce is over (time travel instead of sleeping).
    fn quiet(s: &mut Session) {
        s.read_after = Some(Instant::now() - Duration::from_millis(1));
    }

    fn badge(s: &Session, scope: &matrix_domain::RoomScope) -> u64 {
        s.sidebar.rooms().iter().find(|r| &r.scope == scope).unwrap().unread
    }

    fn sent(s: &mut Session) -> Option<OperationCommand> {
        s.op_pending.take()
    }

    #[test]
    fn open_focused_bottom_sends_one_receipt_and_the_badge_stays_off() {
        let (mut s, scope) = open_room("one", 0, 2);
        s.drive_unread();
        assert!(badge(&s, &scope) > 0, "precondition: the open counts its unread rows locally");
        new_event(&mut s);
        s.queue_read_marker();
        assert!(sent(&mut s).is_none(), "nothing goes out inside the debounce");
        quiet(&mut s);
        s.queue_read_marker();
        let cmd = sent(&mut s).expect("a receipt after the quiet time");
        assert!(matches!(&cmd, OperationCommand::MarkRoom { scope: sc, read: true, public: true, main_only: true } if *sc == scope));
        // Same frame loop, next frames: no second receipt, and the local count no longer lights the badge.
        s.queue_read_marker();
        assert!(sent(&mut s).is_none(), "one event, one receipt");
        s.drive_unread();
        assert_eq!(badge(&s, &scope), 0);
        assert_eq!(s.unread_open.as_ref().unwrap().read_id.as_deref(), s.messages.as_ref().unwrap().events().last().unwrap().event_id());
    }

    #[test]
    fn badge_counts_up_without_focus_and_the_receipt_follows_the_focus() {
        let (mut s, scope) = open_room("focus", 0, 2);
        s.reading.0 = false;
        new_event(&mut s);
        quiet(&mut s);
        s.queue_read_marker();
        s.drive_unread();
        assert!(sent(&mut s).is_none(), "no receipt without focus");
        assert!(badge(&s, &scope) > 0, "the counter keeps counting the new rows");
        assert!(s.read_dirty, "the receipt stays owed");
        s.reading.0 = true;
        s.queue_read_marker();
        assert!(matches!(sent(&mut s), Some(OperationCommand::MarkRoom { read: true, .. })));
        s.drive_unread();
        assert_eq!(badge(&s, &scope), 0);
    }

    #[test]
    fn scrolled_up_sends_nothing_and_the_receipt_comes_back_at_the_bottom() {
        let (mut s, scope) = open_room("up", 0, 2);
        s.reading.1 = false;
        new_event(&mut s);
        quiet(&mut s);
        s.queue_read_marker();
        assert!(sent(&mut s).is_none());
        s.drive_unread();
        assert!(badge(&s, &scope) > 0);
        s.reading.1 = true;
        s.queue_read_marker();
        assert!(sent(&mut s).is_some());
    }

    #[test]
    fn burst_sends_one_receipt_on_the_last_event() {
        let (mut s, scope) = open_room("burst", 0, 1);
        let mut receipts = 0;
        for _ in 0..5 {
            new_event(&mut s);
            s.queue_read_marker();
            receipts += sent(&mut s).is_some() as u32;
        }
        assert_eq!(receipts, 0, "events 500 ms apart or closer keep pushing the receipt back");
        assert!(s.heights.watching, "the next frame is asked for, or the receipt would wait for input");
        quiet(&mut s);
        s.queue_read_marker();
        receipts += sent(&mut s).is_some() as u32;
        s.queue_read_marker();
        receipts += sent(&mut s).is_some() as u32;
        assert_eq!(receipts, 1);
        s.drive_unread();
        assert_eq!(badge(&s, &scope), 0);
    }

    #[test]
    fn receipt_is_for_the_room_of_a_secondary_account() {
        let (mut s, scope) = open_room("second", 2, 2);
        assert_eq!(scope.account_id, matrix_domain::AccountId::new(2));
        new_event(&mut s);
        quiet(&mut s);
        s.queue_read_marker();
        match sent(&mut s) {
            Some(OperationCommand::MarkRoom { scope: sc, .. }) => assert_eq!(sc.account_id, matrix_domain::AccountId::new(2)),
            other => panic!("expected MarkRoom, got a command: {}", other.is_some()),
        }
    }

    #[test]
    fn divider_survives_the_receipt() {
        let (mut s, _) = open_room("divider", 0, 2);
        s.heights.divider = Some(EventId::new("$demo-5:fixture.invalid"));
        new_event(&mut s);
        quiet(&mut s);
        s.queue_read_marker();
        assert!(sent(&mut s).is_some());
        s.drive_unread();
        assert_eq!(s.heights.divider.as_ref().map(EventId::as_str), Some("$demo-5:fixture.invalid"), "«Nuovi messaggi» stays where the reading started");
    }

    #[test]
    fn busy_operation_slot_keeps_the_receipt_owed() {
        let (mut s, _) = open_room("pending", 0, 2);
        s.read_dirty = true;
        s.read_after = None;
        s.op_pending = Some(OperationCommand::ReadMarkers { scope: s.sidebar.selected().unwrap().scope.clone() });
        s.queue_read_marker();
        assert!(matches!(sent(&mut s), Some(OperationCommand::ReadMarkers { .. })), "an operation in flight is not replaced");
        assert!(s.read_dirty, "the receipt waits for the slot");
    }
}
