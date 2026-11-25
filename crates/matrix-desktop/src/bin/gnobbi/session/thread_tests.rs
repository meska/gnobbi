//! Thread panel, drawn through the real `info::thread` on a headless egui context: the root from the first frame,
//! the spinner until the first page, the empty and failed cases. The state is built by hand (as `open_thread` does).
use super::*;
use matrix_desktop::DemoEvent;
use matrix_domain::{AccountId, RoomId, RoomScope, ScopedTimelineId};

const ROOT_TEXT: &str = "root of the thread";

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!thread:fixture.invalid") }
}

fn root_row() -> DemoEvent {
    DemoEvent {
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
    ThreadState { root: EventId::new("$root:fixture.invalid"), views: Default::default(), history, loaded: false, at_start: false, heights: Default::default(), failed: false, draft: String::new(), attachment: None, attachment_revision: 0, voice: None, reply_to: None, menu: None, want: 0, growing: None, stalled: false, tail_visible: false, read_views: Default::default(), read_target: None, read_pending: false, read_public: None, read_at: None, next_read: std::time::Instant::now() }
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
    (done.loaded, done.at_start) = (true, true); // as a page of a thread without replies says
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
    let mut th = state(seeded());
    (th.loaded, th.at_start) = (true, true);
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
    (th.loaded, th.at_start, th.failed) = (true, true, true);
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
