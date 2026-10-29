//! Message actions open from a context menu on the bubble, only for confirmed live messages.
use matrix_desktop::{DemoHistory, DemoSendState};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!r:test"),
    }
}

fn live_history() -> DemoHistory {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        event_id: EventId::new("$e"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", None),
        video: None,
        styles: Vec::new(),
        thread: None,
    }]);
    rows
}

#[test]
fn confirmed_live_message_opens_context_menu() {
    assert!(live_history().events()[0].menu_available(true));
}

#[test]
fn fixture_message_has_no_context_menu() {
    assert!(!live_history().events()[0].menu_available(false));
}

#[test]
fn unsent_message_has_no_context_menu() {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.submit_text("nuovo".into());
    let event = &rows.events()[0];
    assert_eq!(event.send_state, Some(DemoSendState::Pending));
    assert!(!event.menu_available(true));
}

fn id(event: &str) -> matrix_domain::ScopedTimelineId {
    matrix_domain::ScopedTimelineId {
        scope: scope(),
        id: matrix_domain::TimelineId::Server(EventId::new(event)),
    }
}

fn two_live() -> DemoHistory {
    let mut rows = DemoHistory::new(scope(), 0);
    let event = |id: &str| HistoryEvent {
        event_id: EventId::new(id),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", None),
        video: None,
        styles: Vec::new(),
        thread: None,
    };
    rows.prepend_server(vec![event("$old"), event("$new")]);
    rows
}

#[test]
fn keyboard_menu_targets_the_newest_confirmed_message() {
    assert_eq!(two_live().keyboard_menu_target(None), Some(id("$new")));
}

#[test]
fn keyboard_menu_keeps_the_message_already_in_focus() {
    let rows = two_live();
    assert_eq!(
        rows.keyboard_menu_target(Some(&id("$old"))),
        Some(id("$old"))
    );
}

#[test]
fn keyboard_menu_ignores_unsent_and_unknown_messages() {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.submit_text("nuovo".into());
    assert_eq!(rows.keyboard_menu_target(None), None);
    assert_eq!(
        two_live().keyboard_menu_target(Some(&id("$gone"))),
        Some(id("$new"))
    );
}

#[test]
fn menu_key_and_shift_f10_open_the_menu() {
    use matrix_desktop::opens_message_menu as open;
    assert!(open("menu", false, false, false, false));
    assert!(open("f10", true, false, false, false));
    assert!(!open("f10", false, false, false, false));
    assert!(!open("f10", true, true, false, false));
    assert!(!open("a", true, false, false, false));
}
