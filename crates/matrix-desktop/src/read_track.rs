//! First-unread navigation and progressive reading, as pure logic: where the user has read up to, which row to open on,
//! how many messages are still unread and when a receipt for a row in view may go out.
use std::time::{Duration, Instant};

/// A row has to stay in view this long before it counts as read.
pub const DWELL: Duration = Duration::from_millis(300);
/// At most one receipt per this while scrolling; the newest ripe row goes out as soon as it has passed.
pub const THROTTLE: Duration = Duration::from_millis(1500);
/// Pages of history asked for while the read marker is still not in the loaded rows.
pub const MAX_PAGES: u8 = 40;
/// Rows wanted above the first unread before the room is shown: more than the auto-paging reach (`HistorySession::AUTO_LOAD_ROWS`),
/// so no older page lands on top of the anchor after it is placed.
pub const CONTEXT_ROWS: usize = 30;

pub trait ReadRow {
    /// Server id; `None` for a local echo.
    fn event_id(&self) -> Option<&str>;
    fn own(&self) -> bool;
    /// A message as the reader sees it: not a status line, not a deleted row.
    fn message(&self) -> bool;
}

impl ReadRow for crate::DemoEvent {
    fn event_id(&self) -> Option<&str> {
        match &self.id.id {
            matrix_domain::TimelineId::Server(id) => Some(id.as_str()),
            _ => None,
        }
    }
    fn own(&self) -> bool {
        self.outgoing
    }
    fn message(&self) -> bool {
        !self.status && !self.redacted
    }
}

/// Index of the newest row any of `markers` points at.
pub fn marker_index<R: ReadRow>(rows: &[R], markers: &[impl AsRef<str>]) -> Option<usize> {
    rows.iter().rposition(|r| r.event_id().is_some_and(|id| markers.iter().any(|m| m.as_ref() == id)))
}

/// `rows[..n]` are read: up to the marker, and past the user's own newest message (writing reads what came before).
pub fn read_upto<R: ReadRow>(rows: &[R], marker: Option<usize>) -> usize {
    marker.map_or(0, |i| i + 1).max(rows.iter().rposition(|r| r.own()).map_or(0, |i| i + 1))
}

/// First row to show after `upto`: hidden rows (status, deleted) are skipped, they stay out of the count too.
pub fn first_unread<R: ReadRow>(rows: &[R], upto: usize) -> Option<usize> {
    (upto..rows.len()).find(|&i| rows[i].message() && !rows[i].own())
}

/// Messages from others after `upto`.
pub fn unread_after<R: ReadRow>(rows: &[R], upto: usize) -> u64 {
    rows.get(upto..).map_or(0, |r| r.iter().filter(|r| r.message() && !r.own()).count() as u64)
}

/// Where the n-th unread message from the end sits: the fallback when the marker row is not in the room.
fn nth_from_end<R: ReadRow>(rows: &[R], n: u64) -> Option<usize> {
    let mut left = n;
    rows.iter().rposition(|r| {
        if r.message() && !r.own() {
            left = left.saturating_sub(1);
        }
        left == 0 && n > 0
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The marker may still come with an older page.
    Wait,
    /// Open on the first unread row after `rows[..upto]`.
    At { upto: usize },
    /// Nothing visible is unread: the usual position.
    Bottom,
    /// A complete scan found no marker and there is no unread count to anchor by.
    NotFound,
}

/// Chooses the opening anchor from the markers, the loaded rows and the room list's unread count;
/// `complete` = the room's start is loaded, `exhausted` = the page budget is spent, `context` = rows wanted above the anchor.
pub fn locate<R: ReadRow>(rows: &[R], markers: &[impl AsRef<str>], unread: u64, (complete, _exhausted): (bool, bool), context: usize) -> Anchor {
    let give_up = Anchor::Wait;
    let upto = match marker_index(rows, markers) {
        Some(i) => read_upto(rows, Some(i)),
        None if complete && unread == 0 && !markers.is_empty() => return Anchor::NotFound,
        // No marker at all (never read) or gone from the room (deleted): the count says where the unread begin.
        None if markers.is_empty() || complete => match nth_from_end(rows, unread) {
            Some(i) => read_upto(rows, (i > 0).then_some(i.saturating_sub(1))),
            None if complete => read_upto(rows, None),
            None => return give_up,
        },
        None => return give_up,
    };
    if first_unread(rows, upto).is_none() {
        Anchor::Bottom
    } else if upto < context && !complete {
        Anchor::Wait
    } else {
        Anchor::At { upto }
    }
}

/// What a frame decided.
#[derive(Debug, PartialEq, Eq, Default)]
pub struct Step {
    /// Row index and event id to send the receipt for.
    pub send: Option<(usize, String)>,
    /// Unread rows are in view that did not go out yet: look again soon.
    pub waiting: bool,
}

/// Moves the read marker forward with what stays in view: dwell per row, one receipt per throttle window, never backwards.
#[derive(Default)]
pub struct ReadTracker {
    seen: Vec<(String, Instant)>,
    last_sent: Option<Instant>,
}

impl ReadTracker {
    /// The window lost focus or the room left the screen: what was in view does not count.
    pub fn reset(&mut self) {
        self.seen.clear();
    }

    /// `visible`: rows whose bottom edge is in view, `(index, event id)`; `floor`: first row not read yet.
    pub fn step(&mut self, visible: &[(usize, &str)], floor: usize, now: Instant) -> Step {
        self.seen.retain(|(id, _)| visible.iter().any(|(_, v)| v == id));
        for (_, id) in visible {
            if !self.seen.iter().any(|(s, _)| s == id) {
                self.seen.push(((*id).to_owned(), now));
            }
        }
        let ripe = visible
            .iter()
            .filter(|(i, id)| *i >= floor && self.seen.iter().any(|(s, t)| s == id && now.duration_since(*t) >= DWELL))
            .max_by_key(|(i, _)| *i);
        let open = self.last_sent.is_none_or(|t| now.duration_since(t) >= THROTTLE);
        let send = ripe.filter(|_| open).map(|(i, id)| (*i, (*id).to_owned()));
        if send.is_some() {
            self.last_sent = Some(now);
        }
        Step { waiting: visible.iter().any(|(i, _)| *i >= floor), send }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `a` = from others, `m` = own, `s` = status line, `x` = deleted; ids are the position.
    struct R(char, String);
    impl ReadRow for R {
        fn event_id(&self) -> Option<&str> { Some(&self.1) }
        fn own(&self) -> bool { self.0 == 'm' }
        fn message(&self) -> bool { !matches!(self.0, 's' | 'x') }
    }
    fn rows(s: &str) -> Vec<R> {
        s.chars().enumerate().map(|(i, c)| R(c, format!("${i}"))).collect()
    }

    #[test]
    fn anchor_is_the_first_visible_row_after_the_marker() {
        let r = rows("aaaasxaaa");
        let loc = |m: &[&str], unread, complete, exhausted| locate(&r, m, unread, (complete, exhausted), 0);
        // Marker on $3: the status line and the deleted row are skipped, the unread start at $6.
        assert_eq!(loc(&["$3"], 3, false, false), Anchor::At { upto: 4 });
        assert_eq!(first_unread(&r, 4), Some(6));
        assert_eq!(unread_after(&r, 4), 3);
        // The newest of several markers wins; one that is not loaded is ignored.
        assert_eq!(loc(&["$1", "$3", "$99"], 3, false, false), Anchor::At { upto: 4 });
        // Marker not loaded: wait through a spent chunk; once the room's start is loaded it is gone → count.
        assert_eq!(loc(&["$99"], 3, false, false), Anchor::Wait);
        assert_eq!(loc(&["$99"], 3, false, true), Anchor::Wait);
        assert_eq!(loc(&["$99"], 3, true, false), Anchor::At { upto: 6 });
        assert_eq!(loc(&["$99"], 0, true, false), Anchor::NotFound);
        assert_eq!(loc(&["$99"], 0, false, true), Anchor::Wait);
        // A redacted marker is still identifiable; count fallback skips hidden rows.
        assert_eq!(loc(&["$5"], 3, false, false), Anchor::At { upto: 6 });
        // Never read: the count places it; fewer loaded than counted and start loaded: everything is unread.
        assert_eq!(loc(&[] as &[&str], 3, false, false), Anchor::At { upto: 6 });
        assert_eq!(loc(&[] as &[&str], 50, true, false), Anchor::At { upto: 0 });
        assert_eq!(loc(&[] as &[&str], 50, false, false), Anchor::Wait);
        // Everything read: bottom.
        assert_eq!(loc(&["$8"], 0, false, false), Anchor::Bottom);
        // Only hidden rows after the marker: bottom.
        assert_eq!(locate(&rows("aasx"), &["$1"], 1, (false, false), 0), Anchor::Bottom);
        // Too little above the first unread: more history first, including across a chunk boundary.
        assert_eq!(locate(&r, &["$3"], 3, (false, false), 10), Anchor::Wait);
        assert_eq!(locate(&r, &["$3"], 3, (true, false), 10), Anchor::At { upto: 4 });
        assert_eq!(locate(&r, &["$3"], 3, (false, true), 10), Anchor::Wait);
    }

    #[test]
    fn own_message_after_the_marker_is_past_it() {
        let r = rows("aaamaa");
        assert_eq!(read_upto(&r, Some(1)), 4);
        assert_eq!(first_unread(&r, 4), Some(4));
        assert_eq!(unread_after(&r, read_upto(&r, Some(1))), 2);
        assert_eq!(read_upto(&r, None), 4);
    }

    #[test]
    fn unread_count_goes_down_as_the_marker_advances() {
        let r = rows("aaaaaaaaaa");
        let counts: Vec<_> = (0..=10).step_by(2).map(|upto| unread_after(&r, upto)).collect();
        assert_eq!(counts, [10, 8, 6, 4, 2, 0]);
        assert_eq!(unread_after(&r, 99), 0);
    }

    #[test]
    fn marker_moves_forward_after_dwell_throttled_and_never_back() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut tr = ReadTracker::default();
        let ids: Vec<String> = (0..20).map(|i| format!("${i}")).collect();
        let vis = |a: usize, b: usize| (a..b).map(|i| (i, ids[i].as_str())).collect::<Vec<_>>();
        // Rows 5..9 appear: nothing before the dwell, then the lowest goes out.
        assert_eq!(tr.step(&vis(5, 9), 5, at(0)), Step { send: None, waiting: true });
        assert_eq!(tr.step(&vis(5, 9), 5, at(299)).send, None);
        assert_eq!(tr.step(&vis(5, 9), 5, at(300)).send, Some((8, "$8".into())));
        // Scrolling on: new rows need their own dwell and the throttle holds the next receipt back.
        assert_eq!(tr.step(&vis(9, 13), 9, at(500)).send, None);
        assert_eq!(tr.step(&vis(9, 13), 9, at(900)).send, None); // ripe, but 900 < 300 + 1500
        assert_eq!(tr.step(&vis(9, 13), 9, at(1800)).send, Some((12, "$12".into())));
        // Scrolled back up above the marker: nothing, never backwards.
        assert_eq!(tr.step(&vis(2, 6), 13, at(4000)), Step { send: None, waiting: false });
        assert_eq!(tr.step(&vis(2, 6), 13, at(6000)).send, None);
        // Focus lost: the dwell starts again.
        tr.reset();
        assert_eq!(tr.step(&vis(13, 15), 13, at(8000)).send, None);
        assert_eq!(tr.step(&vis(13, 15), 13, at(8300)).send, Some((14, "$14".into())));
        // A row that scrolls out before the dwell is not read.
        assert_eq!(tr.step(&vis(15, 17), 15, at(12000)).send, None);
        assert_eq!(tr.step(&vis(17, 19), 15, at(12200)).send, None);
        assert_eq!(tr.step(&vis(17, 19), 15, at(12500)).send, Some((18, "$18".into())));
    }
}
