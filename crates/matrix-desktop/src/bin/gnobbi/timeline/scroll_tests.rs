//! Scroll behaviour of the timeline, driven through the real `show` on a headless egui context.
//! Position is read from what `show` reports: `heights.visible` (rows whose bottom edge is in view: the last row there means
//! the view sits at the very end, with no slack) and `heights.pos` (first visible row + offset, set only above the bottom band).
use super::{show, Acts, Heights, Pictures};
use crate::avatar::Avatars;
use eframe::egui;
use matrix_desktop::{DemoEvent, DemoHistory};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};

const ROWS: usize = 30;
/// How far above the very end the reader sits: 0 is stuck to the end (egui follows alone), 30 is still «at the bottom» (200 px tolerance) but egui no longer follows.
const SLACKS: [f32; 2] = [0., 30.];

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!scroll:fixture.invalid") }
}

/// A plain incoming message `name` of `lines` text lines (a row is about `lines * 15 + 30` px tall).
fn msg(name: &str, lines: usize) -> DemoEvent {
    let text = (0..lines).map(|l| format!("{name} line {l}")).collect::<Vec<_>>().join("\n");
    DemoEvent {
        server_confirmed: true, redacted: false, annotation: String::new(), reactions: Vec::new(), transaction_id: None,
        id: ScopedTimelineId { scope: scope(), id: TimelineId::Server(EventId::new(format!("${name}:fixture.invalid"))) },
        text, outgoing: false, send_state: None, timestamp_ms: 0, styles: Vec::new(), mentions: Vec::new(),
        highlighted: false, thread: None, thread_root: None, reply_to: None, status: false, status_row: None, utd: false,
        video: None, audio: None, poll: None, attachment_name: None, attachment_size: None, media: None, sender: None,
        time: String::new(),
    }
}

fn eid(e: &DemoEvent) -> EventId {
    match &e.id.id {
        TimelineId::Server(id) => id.clone(),
        _ => unreachable!("fixture rows are server rows"),
    }
}

fn pill() -> matrix_engine::history::Reaction {
    matrix_engine::history::Reaction { key: "👍".into(), count: 2, senders: Vec::new(), mine: false, pending: false }
}

struct Rig {
    ctx: egui::Context,
    events: Vec<DemoEvent>,
    heights: Heights,
    pictures: Pictures,
    avatars: Avatars,
    time: f64,
}

impl Rig {
    fn new(rows: usize) -> Self {
        let mut rig = Rig { ctx: egui::Context::default(), events: (0..rows).map(|i| msg(&format!("m{i}"), 2)).collect(), heights: Heights::default(), pictures: Pictures::default(), avatars: Avatars::default(), time: 0. };
        rig.settle();
        assert!(rig.heights.ready, "the room never finished opening");
        rig
    }

    /// One frame with `input` events (a wheel turn, a pointer move).
    fn frame(&mut self, input: Vec<egui::Event>) {
        let history = DemoHistory::new(scope(), self.events.len());
        let tokens = matrix_desktop::Theme::Dark.tokens();
        self.time += 0.05;
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400., 400.))), events: input, time: Some(self.time), ..Default::default() };
        let (events, heights, pictures, avatars) = (&self.events, &mut self.heights, &self.pictures, &self.avatars);
        let mut out = self.ctx.run_ui(raw, |ui| {
            show(ui, 0, events, &history, heights, pictures, avatars, &mut Acts::default(), 3, &Default::default(), &tokens, None, None);
        });
        out.textures_delta.clear();
    }

    fn settle(&mut self) {
        for _ in 0..12 {
            self.frame(vec![]);
        }
    }

    /// Wheel over the list: positive `dy` moves the content down, i.e. the reader goes up.
    fn wheel(&mut self, dy: f32) {
        self.frame(vec![egui::Event::PointerMoved(egui::pos2(200., 200.))]);
        self.frame(vec![egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: egui::vec2(0., dy), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: Default::default() }]);
        self.settle();
    }

    fn last_in_view(&self) -> bool {
        self.heights.visible.last().is_some_and(|(i, _)| *i + 1 == self.events.len())
    }

    /// `slack` px above the very end (the view then ends below the last row: `last_in_view` is false until the next re-glue).
    fn near_bottom(&mut self, slack: f32) {
        if slack > 0. {
            self.wheel(slack);
            assert!(self.heights.at_bottom && !self.last_in_view(), "not near the bottom");
        }
    }

    /// The reader scrolled up by a wheel turn; returns where.
    fn scrolled_up(&mut self, dy: f32) -> Option<(EventId, f32)> {
        self.wheel(dy);
        assert!(self.heights.pos.is_some() && !self.last_in_view(), "the wheel did not take the reader up");
        self.heights.pos.clone()
    }

    /// A row in view, below the one at the top.
    fn row_in_view(&self) -> usize {
        self.heights.visible.get(2).expect("rows in view").0
    }

    fn assert_still(&self, before: &Option<(EventId, f32)>) {
        let (a, b) = (before.as_ref().unwrap(), self.heights.pos.as_ref().expect("the reader fell to the bottom"));
        assert_eq!(a.0, b.0, "another row is now at the top of the view");
        assert!((a.1 - b.1).abs() < 1., "the view moved by {}px", (a.1 - b.1).abs());
        assert!(!self.last_in_view(), "the reader was pulled to the end");
    }
}

/// One picture of an album of 3 from the same sender.
fn album_item(index: u8) -> DemoEvent {
    let mut e = msg(&format!("album{index}"), 0);
    e.text.clear();
    e.sender = Some(matrix_engine::history::HistorySender::new("@anna:fixture.invalid", None));
    e.attachment_name = Some(format!("foto{index}.png"));
    e.media = Some(matrix_engine::history::Media {
        kind: matrix_engine::history::MediaKind::Image,
        label: "Image".into(),
        animated: false,
        dims: Some((400, 300)),
        album: Some(matrix_engine::sending::Album { id: "a".into(), index, count: 3 }),
    });
    e
}

#[test]
fn an_album_is_one_row_and_the_view_stays_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        rig.events.extend((0..3).map(album_item));
        rig.settle();
        let n = rig.events.len();
        // The first two items take no room; the last draws the grid (three 4:3 tiles tall).
        assert_eq!(rig.heights.rows[n - 3..n - 1], [0., 0.], "slack {slack}px");
        assert!(rig.heights.rows[n - 1] > 150., "slack {slack}px: the grid is {}px", rig.heights.rows[n - 1]);
        assert!(rig.last_in_view(), "slack {slack}px: the album is below the fold");
        // One more message after it: still at the bottom, and the album's rows stay hidden.
        rig.events.push(msg("after", 2));
        rig.settle();
        assert!(rig.last_in_view() && rig.heights.rows[n - 2] == 0., "slack {slack}px");
    }
}

#[test]
fn at_the_bottom_a_new_message_keeps_the_view_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        rig.events.push(msg("new", 4));
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px: the new message is below the fold");
    }
}

#[test]
fn at_the_bottom_several_messages_at_once_keep_the_view_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        rig.events.extend((0..3).map(|i| msg(&format!("burst{i}"), 3)));
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px");
    }
}

#[test]
fn at_the_bottom_a_row_that_grows_keeps_the_view_at_the_bottom() {
    // A message that decrypts after it was drawn: same event, more text.
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        let last = rig.events.len() - 1;
        rig.events[last] = msg(&format!("m{last}"), 12);
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px: the grown row now ends below the fold");
    }
}

#[test]
fn at_the_bottom_a_growing_row_above_the_last_keeps_the_view_at_the_bottom() {
    // A link preview arrives under an earlier row that is still on screen.
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        let row = rig.events.len() - 3;
        rig.events[row] = msg(&format!("m{row}"), 8);
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px");
    }
}

#[test]
fn at_the_bottom_a_reaction_keeps_the_view_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        let last = rig.events.len() - 1;
        rig.events[last].reactions.push(pill());
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px: the pills under the last row pushed it below the fold");
    }
}

#[test]
fn at_the_bottom_the_new_messages_line_keeps_the_view_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        rig.heights.divider = Some(eid(rig.events.last().unwrap()));
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px: the line pushed the last row below the fold");
    }
}

#[test]
fn at_the_bottom_an_older_page_keeps_the_view_at_the_bottom() {
    for slack in SLACKS {
        let mut rig = Rig::new(ROWS);
        rig.near_bottom(slack);
        rig.events.splice(0..0, (0..10).map(|i| msg(&format!("old{i}"), 3)));
        rig.settle();
        assert!(rig.last_in_view(), "slack {slack}px");
    }
}

#[test]
fn scrolled_up_a_new_message_does_not_move_the_view() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(600.);
    rig.events.push(msg("new", 4));
    rig.settle();
    rig.assert_still(&before);
}

#[test]
fn scrolled_up_beyond_the_tolerance_a_new_message_does_not_pull_the_reader_down() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(400.);
    rig.events.push(msg("new", 4));
    rig.settle();
    rig.assert_still(&before);
}

#[test]
fn scrolled_up_the_new_messages_line_does_not_move_the_view() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(600.);
    let at = rig.row_in_view();
    rig.heights.divider = Some(eid(&rig.events[at]));
    rig.settle();
    rig.assert_still(&before);
}

#[test]
fn scrolled_up_a_row_that_grows_below_the_top_does_not_move_the_view() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(600.);
    let at = rig.row_in_view();
    rig.events[at] = msg(&format!("m{at}"), 9);
    rig.settle();
    rig.assert_still(&before);
}

#[test]
fn scrolled_up_a_reaction_does_not_move_the_view() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(600.);
    let at = rig.row_in_view();
    rig.events[at].reactions.push(pill());
    rig.settle();
    rig.assert_still(&before);
}

#[test]
fn scrolled_up_an_older_page_does_not_move_the_view() {
    let mut rig = Rig::new(ROWS);
    let before = rig.scrolled_up(600.);
    rig.events.splice(0..0, (0..10).map(|i| msg(&format!("old{i}"), 3)));
    rig.settle();
    rig.assert_still(&before);
}
