#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
//! A status row reads «Stato della stanza» in the semantic node, never «In entrata»/«In uscita», and offers no menu.
use matrix_desktop::{DemoHistory, accessibility::HistoryNodes};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn row(id: &str, body: &str, status: bool) -> HistoryEvent {
    HistoryEvent {
        event_id: EventId::new(id),
        body: body.into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", None),
        video: None,
        attachment_name: None,
        attachment_size: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status,
    }
}

#[test]
fn status_row_is_labelled_and_has_no_menu() {
    let mut rows = DemoHistory::new(RoomScope { account_id: AccountId::new(45), room_id: RoomId::new("!s:fixture.invalid") }, 0);
    rows.prepend_server(vec![row("$s", "@anna:test è entrato/a nella stanza", true), row("$m", "ciao", false)]);
    let messages = HistoryNodes::default().project(&rows, 0, true).messages;
    assert!(messages[0].description.contains("Stato della stanza"), "{}", messages[0].description);
    assert!(messages[0].description.starts_with("00:00 UTC"), "{}", messages[0].description);
    assert!(!messages[0].description.contains("In entrata"), "{}", messages[0].description);
    assert_eq!(messages[0].text, "@anna:test è entrato/a nella stanza");
    assert!(!rows.events()[0].menu_available(true));
    assert!(messages[0].actions.is_none());
    assert!(messages[1].description.contains("In entrata"), "{}", messages[1].description);
}
