use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender, ThreadInfo};

fn root(replies: Option<u32>, latest: &str) -> HistoryEvent {
    HistoryEvent {
        event_id: EventId::new("$root"),
        body: "radice".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", Some("Anna")),
        video: None,
        styles: Vec::new(),
        thread: replies.map(|num_replies| ThreadInfo {
            num_replies,
            latest_sender: Some(HistorySender::new(latest, None)),
        }),
        thread_root: None,
    }
}

#[test]
fn reply_count_follows_the_timeline_live() {
    let scope = RoomScope { account_id: AccountId::new(7), room_id: RoomId::new("!r:test") };
    let mut rows = DemoHistory::new(scope, 0);
    rows.prepend_server(vec![root(None, "")]);
    assert_eq!(rows.events()[0].thread_label(), None, "no thread, no summary");
    assert!(rows.refresh_server(vec![root(Some(1), "@bob:test")]), "first reply must repaint");
    assert_eq!(rows.events()[0].thread_label(), Some(("1 risposta".into(), "BO".into(), None)));
    assert!(rows.refresh_server(vec![root(Some(2), "@carla:test")]));
    assert_eq!(rows.events()[0].thread_label(), Some(("2 risposte".into(), "CA".into(), None)));
    assert!(!rows.refresh_server(vec![root(Some(2), "@carla:test")]), "same summary: no repaint");
}
