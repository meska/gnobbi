use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn ev(body: &str, utd: bool) -> HistoryEvent {
    HistoryEvent {
        forwarded: false,
        redacted: false,
        event_id: EventId::new("$old"),
        body: body.into(),
        outgoing: false,
        timestamp_ms: 1,
        sender: HistorySender::new("@a:test", Some("a")),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false, poll: None, media: None,
        thread: None,
        thread_root: None, reply_to: None,
        status: false, status_row: None,
        utd, reactions: Vec::new(),
        attachment_name: None,
        attachment_size: None,
    }
}

/// After «Recupera chiavi» the same open timeline gets the readable row: no UTD flag, no «Recupera chiavi».
#[test]
fn undecryptable_row_becomes_readable_in_place() {
    let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") };
    let mut h = DemoHistory::new(scope, 0);
    h.append_server(vec![ev("Cifrato · chiavi non disponibili", true)]);
    assert!(h.events()[0].utd);
    assert!(!h.refresh_server(vec![ev("Cifrato · chiavi non disponibili", true)]), "unchanged");
    assert!(h.refresh_server(vec![ev("recover old", false)]), "the body changed");
    assert_eq!(h.events().len(), 1);
    assert_eq!(h.events()[0].text, "recover old");
    assert!(!h.events()[0].utd);
}
