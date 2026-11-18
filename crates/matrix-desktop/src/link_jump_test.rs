//! A link to a message: the room is open, the history is read back to the event, the row is scrolled to
//! and flashes; a reply in a thread opens its thread; an event that does not exist leaves the room and says so.
//!   the ct script 'the gui-test-x11 script "" link_to_a_message_scrolls_loads_and_flashes'
use crate::link_view::LinkJump;
use crate::*;
use gpui::{AsyncWindowContext, BackgroundExecutor, WeakEntity};
use matrix_domain::EventId;
use matrix_engine::history::{HistoryEvent, HistoryPage, HistoryResult, HistorySender};

fn ev(id: &str, thread_root: Option<&str>) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted: false,
        utd: false,
        event_id: EventId::new(id),
        body: format!("testo {id}"),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@ada:fixture.test", Some("Ada")),
        video: None,
        audio: None,
        styles: Vec::new(),
        mentions: Vec::new(),
        highlighted: false,
        thread: None,
        thread_root: thread_root.map(EventId::new),
        status: false,
        attachment_name: None,
        attachment_size: None,
        media: None,
    }
}

/// A fresh room whose newest page is `first`; the older pages `older` (newest first) are served on request.
pub(crate) fn open_room(this: &mut Shell, first: Vec<HistoryEvent>, more: bool, cx: &mut Context<Shell>) {
    this.thread = None;
    this.link_notice = None;
    this.history = HistorySession::new();
    let timeline = crate::timeline_view::TimelineView::connected(this.active_scope.clone(), cx.focus_handle());
    let request = this.history.select(Some(this.active_scope.clone())).expect("history opens");
    let page = HistoryPage { tag: request.tag.clone(), events: first, next: more.then(|| "t0".to_owned()) };
    let events = this.history.apply(HistoryResult { tag: request.tag, page: Ok(page) }).unwrap();
    timeline.prepend_server(events);
    this.timeline = Some(timeline);
    this.pending_history = None;
}

/// What the engine would do with the page request the jump made: the next older page, or the start.
fn serve(this: &mut Shell, older: &mut Vec<Vec<HistoryEvent>>, always_more: bool) -> bool {
    let Some(request) = this.pending_history.take() else { return false };
    let events = if older.is_empty() { vec![] } else { older.remove(0) };
    let next = (always_more || !older.is_empty()).then(|| "t".to_owned());
    let page = HistoryPage { tag: request.tag.clone(), events, next };
    if let Some(events) = this.history.apply(HistoryResult { tag: request.tag, page: Ok(page) }) {
        this.timeline.as_ref().unwrap().prepend_server(events);
    }
    true
}

/// Ticks the link until it is carried out, serving pages; returns the pages served.
fn run(this: &mut Shell, older: &mut Vec<Vec<HistoryEvent>>, always_more: bool, cx: &mut Context<Shell>) -> usize {
    let mut served = 0;
    for _ in 0..500 {
        this.drive_link_jump(cx);
        if this.link_jump.is_none() {
            return served;
        }
        served += usize::from(serve(this, older, always_more));
    }
    panic!("the link jump never ended");
}

pub(crate) async fn scenario(this: WeakEntity<Shell>, cx: &mut AsyncWindowContext, executor: BackgroundExecutor) {
    let wait = |ms| executor.timer(std::time::Duration::from_millis(ms));
    wait(500).await;
    let page = |ids: &[&str]| ids.iter().map(|id| ev(id, None)).collect::<Vec<_>>();

    // Already loaded: no page is read; the row flashes for 2 s.
    this.update_in(cx, |this, _, cx| {
        open_room(this, page(&["$e1", "$e2", "$e3"]), false, cx);
        this.link_jump = Some(LinkJump::new(this.active_scope.clone(), "$e2"));
        assert_eq!(run(this, &mut vec![], false, cx), 0, "no older page for a loaded row");
        assert!(this.link_notice.is_none(), "found: no «non trovato»");
    })
    .unwrap();
    wait(2200).await;
    this.update_in(cx, |this, _, _| assert!(this.timeline.as_ref().unwrap().flash_ended(), "the row was marked and the flash ran out"))
        .unwrap();

    // In older pages: they are read until the row shows.
    this.update_in(cx, |this, _, cx| {
        open_room(this, page(&["$e10", "$e11"]), true, cx);
        let mut older = vec![page(&["$e7", "$e8", "$e9"]), page(&["$e4", "$e5", "$e6"]), page(&["$e1", "$e2", "$e3"])];
        this.link_jump = Some(LinkJump::new(this.active_scope.clone(), "$e5"));
        assert_eq!(run(this, &mut older, false, cx), 2, "two older pages, not the third");
        assert!(this.link_notice.is_none());
        assert!(this.timeline.as_ref().unwrap().scroll_to_event(&EventId::new("$e5")), "the row is loaded");
    })
    .unwrap();
    wait(2200).await;
    this.update_in(cx, |this, _, _| assert!(this.timeline.as_ref().unwrap().flash_ended(), "flash after the pages")).unwrap();

    // An event that does not exist: the start of the history ends it, the room stays, one line says so.
    this.update_in(cx, |this, _, cx| {
        open_room(this, page(&["$e10"]), true, cx);
        let mut older = vec![page(&["$e5"]), page(&["$e1"])];
        this.link_jump = Some(LinkJump::new(this.active_scope.clone(), "$nope"));
        assert_eq!(run(this, &mut older, false, cx), 2, "all the history is read once, no more");
        assert_eq!(this.link_notice.map(|(t, _)| t), Some("Messaggio non trovato"));
        assert!(this.timeline.is_some(), "the room stays open");
    })
    .unwrap();

    // A history that never ends is cut by the page limit.
    this.update_in(cx, |this, _, cx| {
        open_room(this, page(&["$e10"]), true, cx);
        let mut link = LinkJump::new(this.active_scope.clone(), "$nope");
        link.pages = 3;
        this.link_jump = Some(link);
        assert_eq!(run(this, &mut vec![], true, cx), 3, "the limit is 3 pages");
        assert_eq!(this.link_notice.map(|(t, _)| t), Some("Messaggio non trovato"));
    })
    .unwrap();

    // A reply of a thread: its thread opens, as from the search.
    this.update_in(cx, |this, _, cx| {
        open_room(this, vec![ev("$root", None), ev("$reply", Some("$root"))], false, cx);
        this.link_jump = Some(LinkJump::new(this.active_scope.clone(), "$reply"));
        run(this, &mut vec![], false, cx);
        assert!(this.link_notice.is_none());
        assert_eq!(this.thread.as_ref().map(|t| t.root.clone()), Some(EventId::new("$root")), "the thread of the reply is open");
    })
    .unwrap();

    // Another room is shown meanwhile: the link waits and never scrolls the wrong one.
    this.update_in(cx, |this, _, cx| {
        open_room(this, page(&["$e1", "$e2"]), false, cx);
        let other = RoomScope { account_id: this.active_scope.account_id, room_id: matrix_domain::RoomId::new("!other:fixture.test") };
        this.link_jump = Some(LinkJump::new(other, "$e2"));
        this.drive_link_jump(cx);
        assert!(this.link_jump.is_some() && this.link_notice.is_none(), "waits for its room");
        this.link_jump = None;
        cx.quit();
    })
    .unwrap();
}
