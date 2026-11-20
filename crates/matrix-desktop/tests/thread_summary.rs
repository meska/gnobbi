use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender, ThreadInfo};

fn root(replies: Option<u32>, latest: &str) -> HistoryEvent {
    HistoryEvent {
        poll: None,
        redacted: false,
        event_id: EventId::new("$root"),
        body: "radice".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: HistorySender::new("@anna:test", Some("Anna")),
        video: None,
        audio: None,
        styles: Vec::new(), mentions: Vec::new(), highlighted: false,
        thread: replies.map(|num_replies| ThreadInfo {
            num_replies,
            latest_sender: Some(HistorySender::new(latest, None)),
            unread: 0,
        }),
        thread_root: None, reply_to: None,
        status: false, status_row: None,
        utd: false,
        attachment_name: None,
        attachment_size: None, media: None,
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

#[test]
fn unread_replies_show_next_to_the_count_and_clear_when_the_thread_is_read() {
    let scope = RoomScope { account_id: AccountId::new(7), room_id: RoomId::new("!r:test") };
    let mut rows = DemoHistory::new(scope, 0);
    let with = |unread| {
        let mut event = root(Some(3), "@bob:test");
        event.thread.as_mut().unwrap().unread = unread;
        event
    };
    rows.prepend_server(vec![with(2)]);
    assert_eq!(rows.events()[0].thread_label().unwrap().0, "3 risposte · 2 non lette");
    assert!(rows.refresh_server(vec![with(1)]), "a changed unread count repaints");
    assert_eq!(rows.events()[0].thread_label().unwrap().0, "3 risposte · 1 non letta");
    assert!(rows.clear_thread_unread(&EventId::new("$root")));
    assert!(!rows.clear_thread_unread(&EventId::new("$root")), "nothing left to clear");
    assert_eq!(rows.events()[0].thread_label().unwrap().0, "3 risposte");
}
