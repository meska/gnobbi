//! Debug mode details: server event ids and the room line; none for unsent rows.
use matrix_desktop::{DemoHistory, debug_room_line};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!r:test"),
    }
}

#[test]
fn server_message_exposes_its_event_id_and_unsent_one_does_not() {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        poll: None,
        redacted: false,
        event_id: EventId::new("$abc:test"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", None),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None, thread_root: None, reply_to: None,
        status: false, status_row: None,
        utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
    }]);
    assert_eq!(rows.events()[0].debug_id(), Some("$abc:test"));
    let mut local = DemoHistory::new(scope(), 0);
    local.submit_text("nuovo".into());
    assert_eq!(local.events()[0].debug_id(), None);
}

#[test]
fn room_line_shows_room_id_and_sync_state() {
    assert_eq!(debug_room_line(&scope(), "connesso"), "!r:test · connesso");
}
