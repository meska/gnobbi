//! The bubble footer of a live message never shows the event id.
use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

#[test]
fn live_server_message_stamp_has_no_event_id() {
    let mut rows = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:test"),
        },
        0,
    );
    rows.prepend_server(vec![HistoryEvent {
        redacted: false,
        event_id: EventId::new("$secretid"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    let stamp = rows.events()[0].stamp();
    assert!(!stamp.contains("secretid"), "event id leaked in {stamp:?}");
    assert_eq!(stamp, rows.events()[0].time);
}
