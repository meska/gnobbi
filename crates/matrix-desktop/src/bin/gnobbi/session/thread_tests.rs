//! Thread panel, drawn through the real `info::thread` on a headless egui context: the root from the first frame,
//! the spinner until the first page, the empty and failed cases. The state is built by hand (as `open_thread` does).
use super::*;
use matrix_desktop::DemoEvent;
use matrix_domain::{AccountId, RoomId, RoomScope, ScopedTimelineId};
use matrix_engine::history::{HistoryEvent, HistorySender, ThreadInfo, ThreadPage};

const ROOT_TEXT: &str = "root of the thread";

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!thread:fixture.invalid") }
}

fn root_row() -> DemoEvent {
    DemoEvent {
        forwarded: false,
        server_confirmed: true, redacted: false, annotation: String::new(), reactions: Vec::new(), transaction_id: None,
        id: ScopedTimelineId { scope: scope(), id: TimelineId::Server(EventId::new("$root:fixture.invalid")) },
        text: ROOT_TEXT.into(), outgoing: false, send_state: None, timestamp_ms: 0, styles: Vec::new(), mentions: Vec::new(),
        highlighted: false, thread: None, thread_root: None, reply_to: None, status: false, status_row: None, utd: false,
        video: None, audio: None, poll: None, attachment_name: None, attachment_size: None, media: None, sender: None,
        time: String::new(),
    }
}

/// The state `open_thread` leaves: the root seeded (or nothing), `want` 0, first page not in.
fn state(history: Option<DemoHistory>) -> ThreadState {
    ThreadState::open(EventId::new("$root:fixture.invalid"), history)
}

fn seeded() -> Option<DemoHistory> {
    Some(DemoHistory::seeded(scope(), root_row()))
}

/// The list measures its rows before it paints them (the same for a room): the first frames hold no rows. The root must be
/// there by `SETTLED`, with no network involved; the checks on what stays on screen look at the frames after it.
const SETTLED: usize = 3;

/// What a frame drew: all the text, and the number of line paths (a spinner is one).
struct Drawn { texts: Vec<String>, paths: usize }

fn frames(th: &mut ThreadState, n: usize) -> Vec<Drawn> {
    let ctx = egui::Context::default();
    let tokens = matrix_desktop::Theme::Dark.tokens();
    let (pictures, avatars) = (crate::timeline::Pictures::default(), crate::avatar::Avatars::default());
    (0..n).map(|i| {
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420., 700.))), time: Some(i as f64 * 0.05), ..Default::default() };
        let mut close = false;
        let mut out = ctx.run_ui(raw, |ui| {
            crate::info::thread(ui, th, &pictures, &avatars, 3, None, &mut Default::default(), &mut close, &tokens);
        });
        out.textures_delta.clear();
        fn flat(s: &egui::Shape, into: &mut Vec<egui::Shape>) {
            if let egui::Shape::Vec(v) = s { v.iter().for_each(|s| flat(s, into)); } else { into.push(s.clone()); }
        }
        let mut shapes = Vec::new();
        for s in &out.shapes { flat(&s.shape, &mut shapes); }
        Drawn {
            texts: shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_owned()) } else { None }).collect(),
            paths: shapes.iter().filter(|s| matches!(s, egui::Shape::Path(_))).count(),
        }
    }).collect()
}

fn shows(d: &Drawn, text: &str) -> bool { d.texts.iter().any(|t| t.contains(text)) }

/// Frames before the first page: the root is on screen with no reply and no network, and a spinner runs over it.
#[test]
fn thread_root_is_on_screen_at_once_and_a_spinner_runs_until_the_first_page() {
    let mut waiting = state(seeded());
    let before = frames(&mut waiting, SETTLED + 2);
    let first = before.iter().position(|d| shows(d, ROOT_TEXT)).expect("the root never reached the screen");
    assert!(first < SETTLED, "the root came in frame {first}, after the list's own measuring passes");
    assert!(before[SETTLED..].iter().all(|d| shows(d, ROOT_TEXT)), "the root does not stay");
    let mut done = state(seeded());
    done.loaded = true; // `at_start` stays false: that is what the engine answers for a thread without replies
    let after = frames(&mut done, SETTLED + 2);
    assert!(after[SETTLED..].iter().all(|d| shows(d, ROOT_TEXT)), "the root is gone once the page is in");
    // The spinner is the only difference between the two states: one path more while waiting, none after.
    for (i, (w, d)) in before.iter().zip(&after).enumerate().skip(SETTLED) {
        assert!(w.paths > d.paths, "frame {i}: no spinner while the first page is awaited ({} vs {} paths)", w.paths, d.paths);
    }
}

/// Thread without replies, page in: only the root, nothing spinning (the spinner would never stop).
#[test]
fn thread_without_replies_shows_the_root_and_no_spinner() {
    // Through the real answer: the full page of a thread without replies is the root alone, `at_start` false.
    let mut th = state(seeded());
    th.pending = Some((std::time::Instant::now(), 50));
    th.page(&scope(), Ok(Box::new(page(Some(root_event(None)), Vec::new(), false))));
    th.scrolled(0);
    assert!(th.loaded && th.growing.is_none(), "the page of an empty thread asked for older replies: {:?}", th.growing);
    let mut waiting = state(seeded());
    let (done, wait) = (frames(&mut th, SETTLED + 2), frames(&mut waiting, SETTLED + 2));
    assert!(done[SETTLED..].iter().all(|d| shows(d, ROOT_TEXT)));
    assert!(done.iter().zip(&wait).skip(SETTLED).all(|(d, w)| d.paths < w.paths), "a spinner is still drawn after the page");
}

/// Nothing seeded (the room does not hold the root) and not loaded: the spinner alone, no error.
#[test]
fn thread_without_a_known_root_waits_with_a_spinner() {
    let mut th = state(None);
    let drawn = frames(&mut th, 3);
    assert!(drawn.iter().all(|d| d.paths > 0 && !shows(d, ROOT_TEXT) && !shows(d, i18n::t("Thread unavailable"))));
}

/// A failed first ask replaces the seeded root by the error line (no half thread with a stuck spinner).
#[test]
fn thread_failed_before_the_first_page_shows_the_error_not_the_root() {
    let mut th = state(seeded());
    th.failed = true;
    let drawn = frames(&mut th, 3);
    assert!(drawn.iter().all(|d| shows(d, i18n::t("Thread unavailable")) && !shows(d, ROOT_TEXT)));
    // A failed re-read once the page is in keeps what is there.
    let mut th = state(seeded());
    (th.loaded, th.failed) = (true, true);
    assert!(frames(&mut th, SETTLED + 1)[SETTLED..].iter().all(|d| shows(d, ROOT_TEXT)));
}

/// Older replies are asked for only on a loaded page: the seeded root alone (reader «at the top») must not page.
#[test]
fn thread_pages_back_only_after_the_first_page() {
    let mut th = state(seeded());
    th.scrolled(0);
    assert!((th.growing, th.want) == (None, 0), "paged before the first page: {:?} {}", th.growing, th.want);
    let mut th = state(Some(DemoHistory::new(scope(), 50)));
    (th.loaded, th.want) = (true, 50);
    th.scrolled(0);
    assert_eq!((th.growing, th.want), (Some(50), 100), "a loaded page of 50 at the top asks for 50 more");
    th.scrolled(0);
    assert_eq!((th.growing, th.want), (Some(50), 100), "one ask at a time");
}

fn root_event(replies: Option<u32>) -> HistoryEvent {
    event("$root:fixture.invalid", ROOT_TEXT, replies.map(|num_replies| ThreadInfo { num_replies, latest_sender: None, unread: 0 }))
}

fn event(id: &str, body: &str, thread: Option<ThreadInfo>) -> HistoryEvent {
    HistoryEvent {
        forwarded: false,
        poll: None, redacted: false, event_id: EventId::new(id), body: body.into(), outgoing: false, timestamp_ms: 0,
        sender: HistorySender::new("@anna:fixture.invalid", Some("Anna")), video: None, audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false, thread, thread_root: None, reply_to: None,
        status: false, status_row: None, reactions: Vec::new(), utd: false, attachment_name: None, attachment_size: None, media: None,
    }
}

fn replies(n: usize) -> Vec<HistoryEvent> {
    (0..n).map(|i| event(&format!("$reply{i}:fixture.invalid"), &format!("reply {i}"), None)).collect()
}

fn page(root: Option<HistoryEvent>, replies: Vec<HistoryEvent>, at_start: bool) -> ThreadPage {
    ThreadPage { root, replies, reactions: Vec::new(), at_start }
}

/// A read goes out asking for `want` (what `queue_thread_requests` records).
fn sent(th: &mut ThreadState, want: usize) {
    th.want = want;
    th.pending = Some((std::time::Instant::now(), want));
}

fn rows(th: &ThreadState) -> usize { th.history.as_ref().map_or(0, |h| h.events().len()) }

#[test]
fn opening_a_thread_seeds_the_root_and_asks_the_cache_first() {
    let th = state(seeded());
    assert_eq!((th.want, th.loaded, th.failed, rows(&th), th.head()), (0, false, false, 1, 1));
    let th = state(None);
    assert_eq!((th.want, th.loaded, rows(&th)), (0, false, 0));
}

/// The cache answer with nothing in it leaves the seeded root, stays «loading» and asks for the full page at once.
#[test]
fn an_empty_cache_answer_keeps_the_root_and_the_spinner_and_asks_for_the_full_page() {
    let mut th = state(seeded());
    th.next_read = std::time::Instant::now() + std::time::Duration::from_secs(5);
    sent(&mut th, 0);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(None)), Vec::new(), false))));
    assert_eq!((th.loaded, th.want, rows(&th), th.pending.is_none()), (false, 50, 1, true));
    assert!(th.next_read <= std::time::Instant::now(), "the full read must go out at once");
}

/// Some replies saved (the sync brought the newest): shown, but the spinner stays until the full page.
#[test]
fn a_partial_cache_answer_shows_the_rows_and_keeps_the_spinner() {
    let mut th = state(seeded());
    sent(&mut th, 0);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(Some(5))), replies(1), false))));
    assert_eq!((th.loaded, th.want, rows(&th)), (false, 50, 2), "root + the saved reply, still loading");
    th.scrolled(0);
    assert!(th.growing.is_none(), "no older ask before the full page");
    sent(&mut th, 50);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(Some(5))), replies(5), true))));
    assert_eq!((th.loaded, rows(&th), th.at_start), (true, 6, true));
}

/// «From the cache» is what was ASKED (`want` 0 when it left), not what `want` is when the answer comes.
#[test]
fn the_cache_answer_is_told_by_the_request_not_by_the_current_want() {
    let mut th = state(seeded());
    sent(&mut th, 50);
    th.want = 0;
    th.page(&scope(), Ok(Box::new(page(Some(root_event(None)), Vec::new(), false))));
    assert!(th.loaded, "a full answer with no replies is the whole thread");
}

#[test]
fn a_failed_read_marks_the_panel_failed_and_stops_growing() {
    let mut th = state(seeded());
    sent(&mut th, 0);
    th.growing = Some(3);
    th.page(&scope(), Err(matrix_engine::operations::OperationError::Transport));
    assert!(th.failed && th.growing.is_none() && !th.loaded);
    sent(&mut th, 50);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(None)), Vec::new(), false))));
    assert!(!th.failed && th.loaded, "the next good page clears the error");
}

/// An older page that brings nothing new stops the paging; one that brings more does not.
#[test]
fn growing_ends_stalled_only_when_the_page_brought_nothing() {
    let mut th = state(Some(DemoHistory::new(scope(), 0)));
    th.loaded = true;
    th.growing = Some(2);
    sent(&mut th, 100);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(Some(9))), replies(2), false))));
    assert!(th.stalled && th.growing.is_none());
    th.stalled = false;
    th.growing = Some(2);
    sent(&mut th, 150);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(Some(9))), replies(4), false))));
    assert!(!th.stalled && th.growing.is_none());
}

/// A read in flight is not cancelled by the next one: the panel keeps its answer, the next read waits.
#[test]
fn a_read_in_flight_is_not_restarted_but_a_different_ask_is() {
    let mut th = state(seeded());
    assert!(th.read_due(), "the first read is due at once");
    sent(&mut th, 0);
    th.next_read = std::time::Instant::now();
    assert!(!th.read_due(), "the same ask while one is in flight waits for it");
    th.want = 50;
    assert!(th.read_due(), "a different ask replaces it");
    sent(&mut th, 50);
    th.page(&scope(), Ok(Box::new(page(Some(root_event(None)), Vec::new(), false))));
    assert!(th.read_due(), "the answer is in: the read that activity asked for goes out");
    sent(&mut th, 50);
    th.pending = Some((std::time::Instant::now() - std::time::Duration::from_secs(20), 50));
    assert!(th.read_due(), "a read that never came back does not block for ever");
}

/// Activity in another room does not restart the open thread's read; the open room's does.
#[test]
fn activity_in_another_room_leaves_the_thread_read_alone() {
    let later = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut th = state(seeded());
    th.next_read = later;
    th.activity(&RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!other:fixture.invalid") }, Some(&scope()));
    assert_eq!(th.next_read, later);
    th.activity(&scope(), Some(&scope()));
    assert!(th.next_read < later);
}
