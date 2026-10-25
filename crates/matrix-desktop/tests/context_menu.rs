//! Message actions open from a context menu on the bubble, only for confirmed live messages.
use matrix_desktop::{DemoHistory, DemoSendState};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn scope() -> RoomScope {
    RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") }
}

fn live_history() -> DemoHistory {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        event_id: EventId::new("$e"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", None),
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
