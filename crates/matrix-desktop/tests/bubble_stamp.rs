//! Bubble footer: the time plus a one-glyph delivery state, no state words.
use matrix_desktop::{DemoSendState, bubble_stamp};

#[test]
fn pending_outgoing_shows_clock_glyph_not_words() {
    let s = bubble_stamp("10:42", true, Some(DemoSendState::Pending));
    assert_eq!(s, "10:42 ◷");
}

#[test]
fn failed_outgoing_shows_alert_glyph_not_words() {
    assert_eq!(
        bubble_stamp("10:42", true, Some(DemoSendState::Failed)),
        "10:42 !"
    );
}

#[test]
fn confirmed_outgoing_shows_check() {
    assert_eq!(bubble_stamp("10:42", true, None), "10:42 ✓");
}

#[test]
fn incoming_shows_only_time() {
    assert_eq!(bubble_stamp("10:42", false, None), "10:42");
}

#[test]
fn peer_receipt_marks_that_event_and_older_ones_with_the_double_check() {
    use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
    use matrix_engine::{
        history::{HistoryEvent, HistorySender},
        receipts::PublicReadReceipt,
    };
    let event = |id: &str, ts| HistoryEvent {
        event_id: EventId::new(id),
        body: id.into(),
        outgoing: true,
        timestamp_ms: ts,
        sender: HistorySender::new("@me:test", None),
        video: None,
        styles: Vec::new(),
    };
    let mut rows = matrix_desktop::DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!r:test"),
        },
        0,
    );
    rows.prepend_server(vec![event("$a", 0), event("$b", 1), event("$c", 2)]);
    let read = |ids: &[&str]| -> Vec<bool> {
        let receipts: Vec<_> = ids
            .iter()
            .map(|id| PublicReadReceipt {
                event_id: EventId::new(*id),
                readers: 1,
            })
            .collect();
        let upto = rows.peer_read_index(&receipts);
        (0..3).map(|i| upto.is_some_and(|u| i <= u)).collect()
    };
    assert_eq!(read(&[]), [false; 3]);
    assert_eq!(read(&["$b"]), [true, true, false]);
    let e = &rows.events()[0];
    assert!(e.stamp_read(true).ends_with("✓✓") && e.stamp_read(false).ends_with(" ✓"));
}
