#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
//! Hiding deleted messages removes their semantic nodes too.
use matrix_desktop::{DemoHistory, accessibility::HistoryNodes};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn ev(id: &str, redacted: bool) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted,
        event_id: EventId::new(id),
        body: if redacted { "[Messaggio eliminato]".into() } else { id.into() },
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@a:test", Some("a")),
        video: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }
}

fn history(hide: bool) -> DemoHistory {
    let mut h = DemoHistory::new(RoomScope { account_id: AccountId::new(7), room_id: RoomId::new("!a:test") }, 0);
    h.set_hide_redacted(hide);
    h.prepend_server(vec![ev("$1", false), ev("$2", true), ev("$3", false)]);
    h
}

#[test]
fn accessibility_nodes_follow_the_projection_and_label_deleted_ones() {
    let mut h = history(false);
    let shown = HistoryNodes::default().project(&h, 0, true).messages;
    assert_eq!(shown.len(), 3);
    assert!(shown[1].description.contains("Stato: eliminato"), "{}", shown[1].description);
    h.set_hide_redacted(true);
    let hidden = HistoryNodes::default().project(&h, 0, true).messages;
    assert_eq!(hidden.len(), 2);
    assert!(hidden.iter().all(|m| !m.description.contains("eliminato") && m.text != "[Messaggio eliminato]"));
}

