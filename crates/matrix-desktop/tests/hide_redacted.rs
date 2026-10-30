use matrix_desktop::{DemoHistory, LiveOperations};
use matrix_domain::{
    AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, SendStatus, TimelineId, TimelineItem, TransactionId,
};
use matrix_engine::history::{HistoryEvent, HistorySender};
use matrix_engine::operations::{MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!g:test"),
    }
}

fn ev(id: &str, sender: &str, ts: u64, redacted: bool) -> HistoryEvent {
    HistoryEvent {
        redacted,
        event_id: EventId::new(id),
        body: if redacted {
            "[Messaggio eliminato]".into()
        } else {
            id.into()
        },
        outgoing: false,
        timestamp_ms: ts,
        sender: HistorySender::new(sender, Some(sender)),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: None,
    }
}

fn texts(h: &DemoHistory) -> Vec<String> {
    h.events().iter().map(|e| e.text.clone()).collect()
}

/// anna, anna(deleted), anna, bruno: the deleted row sits in the middle of one sender's series.
fn history() -> DemoHistory {
    let mut h = DemoHistory::new(scope(), 0);
    h.prepend_server(vec![
        ev("$1", "@anna:test", 500, false),
        ev("$2", "@anna:test", 1_000, true),
        ev("$3", "@anna:test", 2_000, false),
        ev("$4", "@bruno:test", 3_000, true),
        ev("$5", "@bruno:test", 4_000, false),
    ]);
    h
}

#[test]
fn shown_by_default() {
    assert_eq!(texts(&history()).len(), 5);
}

#[test]
fn hiding_removes_deleted_rows_and_regroups_neighbours() {
    let mut h = history();
    assert!(h.set_hide_redacted(true));
    assert_eq!(texts(&h), ["$1", "$3", "$5"]);
    // $3 follows $1 of the same sender: no repeated name/avatar; $5 starts bruno's series.
    assert_eq!(h.sender_label(0, 3).as_deref(), Some("@anna:test"));
    assert_eq!(h.sender_label(1, 3), None);
    assert_eq!(h.sender_avatar(1, 3), None);
    assert_eq!(h.sender_label(2, 3).as_deref(), Some("@bruno:test"));
}

#[test]
fn hiding_is_idempotent_and_showing_restores_order() {
    let mut h = history();
    assert!(!h.set_hide_redacted(false));
    h.set_hide_redacted(true);
    assert!(!h.set_hide_redacted(true));
    assert!(h.set_hide_redacted(false));
    assert_eq!(
        texts(&h),
        [
            "$1",
            "[Messaggio eliminato]",
            "$3",
            "[Messaggio eliminato]",
            "$5"
        ]
    );
    assert_eq!(
        h.sender_label(2, 3),
        None,
        "same series again, label rule unchanged"
    );
}

#[test]
fn adjacent_deleted_rows_keep_order_when_shown_again() {
    let mut h = DemoHistory::new(scope(), 0);
    h.prepend_server(vec![
        ev("$1", "@a:test", 1, false),
        ev("$2", "@a:test", 2, true),
        ev("$3", "@a:test", 3, true),
        ev("$4", "@a:test", 4, false),
    ]);
    h.set_hide_redacted(true);
    h.set_hide_redacted(false);
    let ids: Vec<_> = h
        .events()
        .iter()
        .map(|e| format!("{:?}", e.id.id))
        .collect();
    assert!(ids[1].contains("$2") && ids[2].contains("$3"), "{ids:?}");
}

#[test]
fn live_redaction_and_new_deleted_rows_are_hidden_without_a_reload() {
    let mut h = history();
    h.set_hide_redacted(true);
    // $3 is redacted live; $6 arrives already redacted: neither shows, $6 adds no visible row.
    h.refresh_server(vec![
        ev("$3", "@anna:test", 2_000, true),
        ev("$6", "@anna:test", 5_000, true),
    ]);
    assert_eq!(texts(&h), ["$1", "$5"]);
    h.set_hide_redacted(false);
    let ids: Vec<_> = h.events().iter().map(|e| format!("{:?}", e.id.id)).collect();
    for (i, want) in ["$1", "$2", "$3", "$4", "$5", "$6"].iter().enumerate() {
        assert!(ids[i].contains(want), "{ids:?}");
    }
}

#[test]
fn prepend_counts_only_visible_rows() {
    let mut h = DemoHistory::new(scope(), 0);
    h.set_hide_redacted(true);
    let added = h.prepend_server(vec![
        ev("$1", "@a:test", 500, true),
        ev("$2", "@a:test", 1, false),
    ]);
    assert_eq!((added, h.events().len()), (1, 1));
}

#[test]
fn own_deleted_row_returns_to_its_server_position() {
    let mut h = DemoHistory::new(scope(), 0);
    h.prepend_server(vec![ev("$1", "@anna:test", 1_000, false), ev("$2", "@anna:test", 2_000, false)]);
    h.project_delivery(&[TimelineItem {
        identity: ScopedTimelineId { scope: scope(), id: TimelineId::Server(EventId::new("$own")) },
        transaction_id: Some(TransactionId::new("demo-local-1")),
        body: "ciao".into(),
        status: SendStatus::Sent,
    }]);
    h.set_hide_redacted(true);
    h.refresh_server(vec![ev("$own", "@anna:test", 3_000, true), ev("$3", "@anna:test", 4_000, false)]);
    h.set_hide_redacted(false);
    let ids: Vec<_> = h.events().iter().map(|e| format!("{:?}", e.id.id)).collect();
    for (i, want) in ["$1", "$2", "$own", "$3"].iter().enumerate() {
        assert!(ids[i].contains(want), "{ids:?}");
    }
    assert_eq!(ids.len(), 4, "{ids:?}");
}

#[test]
fn pending_row_stays_last_when_a_deleted_row_returns() {
    let mut h = DemoHistory::new(scope(), 0);
    h.set_hide_redacted(true);
    h.prepend_server(vec![ev("$1", "@anna:test", 1_000, false), ev("$2", "@anna:test", 2_000, true)]);
    h.submit_text("ciao".into());
    h.set_hide_redacted(false);
    let t = texts(&h);
    assert_eq!(t.len(), 3, "{t:?}");
    assert_eq!((t[0].as_str(), t[1].as_str(), t[2].as_str()), ("$1", "[Messaggio eliminato]", "ciao"));
}

#[test]
fn locally_deleted_own_row_still_unsynced_returns_to_the_tail() {
    let mut h = DemoHistory::new(scope(), 0);
    h.prepend_server(vec![ev("$1", "@anna:test", 1_000, false), ev("$2", "@anna:test", 2_000, false)]);
    h.project_delivery(&[TimelineItem { identity: ScopedTimelineId { scope: scope(), id: TimelineId::Server(EventId::new("$own")) }, transaction_id: Some(TransactionId::new("tx1")), body: "ciao".into(), status: SendStatus::Sent }]);
    let mut a = LiveOperations::default();
    a.select(ScopedTimelineId { scope: scope(), id: TimelineId::Server(EventId::new("$own")) });
    let Some(OperationCommand::Inspect { tag, target }) = a.take_command() else { panic!() };
    a.apply(OperationResult::Inspected { tag, target, details: Ok(MessageDetails { body: "ciao".into(), permissions: MessagePermissions { edit: true, reply: true, react: true, redact: true } }) });
    assert!(a.submit(Operation::Redact, TransactionId::new("tx2")));
    let Some(OperationCommand::Apply(request)) = a.take_command() else { panic!() };
    a.apply(OperationResult::Applied { request, result: Ok(EventId::new("$red")) });
    h.set_hide_redacted(true);
    h.project_operations(a.confirmed());
    assert_eq!(h.events().len(), 2);
    h.set_hide_redacted(false);
    let ids: Vec<_> = h.events().iter().map(|e| format!("{:?}", e.id.id)).collect();
    assert!(ids[2].contains("$own"), "{ids:?}");
}
