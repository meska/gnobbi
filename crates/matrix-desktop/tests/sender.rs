use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!group:test"),
    }
}

#[test]
fn live_mapping_carries_sender_id_and_name() {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        event_id: EventId::new("$a"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", Some("Anna")),
        video: None,
        styles: Vec::new(),
        thread: None,
    }]);
    let sender = rows.events()[0].sender.as_ref().expect("sender");
    assert_eq!(sender.id, "@anna:test");
    assert_eq!(sender.name.as_deref(), Some("Anna"));
}

#[test]
fn blank_display_name_is_unknown() {
    assert_eq!(HistorySender::new("@a:test", Some("  ")).name, None);
    assert_eq!(HistorySender::new("@a:test", None).name, None);
}

#[test]
fn demo_incoming_events_have_sender_and_outgoing_do_not() {
    let rows = DemoHistory::new(scope(), 6);
    for event in rows.events() {
        assert_eq!(event.sender.is_some(), !event.outgoing);
    }
    assert!(
        rows.events()
            .iter()
            .any(|e| e.sender.as_ref().is_some_and(|s| s.name.is_some()))
    );
}

fn incoming(id: &str, sender: &str, name: Option<&str>, timestamp_ms: u64) -> HistoryEvent {
    HistoryEvent {
        event_id: EventId::new(id),
        body: "x".into(),
        outgoing: false,
        timestamp_ms,
        sender: HistorySender::new(sender, name),
        video: None,
        styles: Vec::new(),
        thread: None,
    }
}

#[test]
fn sender_label_follows_the_rule_in_a_group() {
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![
        incoming("$1", "@anna:test", Some("Anna"), 0),
        incoming("$2", "@anna:test", Some("Anna"), 60_000),
        incoming("$3", "@bruno:test", None, 120_000),
        incoming("$4", "@bruno:test", None, 120_000 + 6 * 60_000),
    ]);
    let labels: Vec<_> = (0..4).map(|i| rows.sender_label(i, 3)).collect();
    assert_eq!(
        labels,
        [
            Some("Anna".to_owned()),
            None,
            Some("@bruno:test".to_owned()),
            Some("@bruno:test".to_owned())
        ]
    );
    assert!((0..4).all(|i| rows.sender_label(i, 2).is_none()));
}
