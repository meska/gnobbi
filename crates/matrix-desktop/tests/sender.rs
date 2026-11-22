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
        poll: None,
        redacted: false,
        event_id: EventId::new("$a"),
        body: "ciao".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", Some("Anna")),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None, thread_root: None, reply_to: None,
        status: false, status_row: None,
        reactions: Vec::new(), utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
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
        poll: None,
        redacted: false,
        event_id: EventId::new(id),
        body: "x".into(),
        outgoing: false,
        timestamp_ms,
        sender: HistorySender::new(sender, name),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None, thread_root: None, reply_to: None,
        status: false, status_row: None,
        reactions: Vec::new(), utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
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

#[test]
fn status_row_offers_no_menu_and_does_not_hide_the_next_sender_name() {
    let row = |id: &str, status: bool| HistoryEvent {
        poll: None,
        event_id: EventId::new(id),
        body: "meska ha invitato Marco".into(),
        outgoing: false,
        redacted: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", Some("Anna")),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None,
        thread_root: None, reply_to: None,
        status, status_row: None,
        reactions: Vec::new(), utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
    };
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![row("$s", true), row("$m", false)]);
    assert!(!rows.events()[0].menu_available(true));
    assert!(rows.events()[1].menu_available(true));
    assert_eq!(rows.sender_label(1, 3).as_deref(), Some("Anna"));
}

#[test]
fn series_continues_only_for_same_sender_within_threshold_without_status_rows() {
    let mut status = incoming("$s", "@anna:test", Some("Anna"), 90_000);
    status.status = true;
    let mine = |id: &str, ts| {
        let mut e = incoming(id, "@me:test", None, ts);
        e.outgoing = true;
        e
    };
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![
        incoming("$1", "@anna:test", Some("Anna"), 0),
        incoming("$2", "@anna:test", Some("Anna"), 60_000), // same sender
        status,                                              // event in between
        incoming("$3", "@anna:test", Some("Anna"), 100_000), // after status
        incoming("$4", "@bruno:test", None, 110_000),        // sender changes
        incoming("$5", "@bruno:test", None, 110_000 + 6 * 60_000), // over threshold
        mine("$m1", 400_000),
        mine("$m2", 410_000), // own messages (outgoing)
    ]);
    let got: Vec<_> = (0..9).map(|i| rows.continues_series(i)).collect();
    assert_eq!(got, [false, true, false, false, false, false, false, true, false]);
}
