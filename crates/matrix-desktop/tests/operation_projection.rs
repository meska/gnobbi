use matrix_desktop::{DemoHistory, LiveOperations};
use matrix_domain::{
    AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId, TransactionId,
};
use matrix_engine::{
    history::HistoryEvent,
    operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
    },
};

#[test]
fn edited_body_copy_does_not_include_status_annotation() {
    let (mut rows, mut actions) = fixture();
    ack(
        &mut actions,
        Operation::Edit {
            body: "edited".into(),
        },
        "$edit",
    );
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events()[0].text, "edited");
    assert!(rows.events()[0].time.ends_with(" · modificato"));
}

#[test]
fn removed_reaction_disappears_without_redacting_message() {
    let (mut rows, mut actions) = fixture();
    ack(
        &mut actions,
        Operation::React { key: "+1".into() },
        "$reaction",
    );
    assert!(actions.toggle_reaction("+1", TransactionId::new("remove")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("remove");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$removed")),
    });
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events().len(), 1);
    assert_eq!(rows.events()[0].text, "old");
    assert_eq!(rows.events()[0].annotation, "");
}

#[test]
fn acknowledged_redaction_keeps_placeholder_at_same_identity() {
    let (mut rows, mut actions) = fixture();
    ack(&mut actions, Operation::Redact, "$redaction");
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events().len(), 1);
    assert_eq!(rows.events()[0].text, "[Messaggio eliminato]");
    assert_eq!(
        rows.events()[0].id.id,
        TimelineId::Server(EventId::new("$first"))
    );
}

#[test]
fn reaction_is_a_separate_idempotent_annotation() {
    let (mut rows, mut actions) = fixture();
    ack(
        &mut actions,
        Operation::React { key: "+1".into() },
        "$reaction",
    );
    rows.project_operations(actions.confirmed());
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events().len(), 1);
    assert_eq!(rows.events()[0].text, "old");
    assert_eq!(rows.events()[0].annotation, "+1 · tu");
}

#[test]
fn acknowledged_reply_adds_one_identified_bubble() {
    let (mut rows, mut actions) = fixture();
    ack(
        &mut actions,
        Operation::Reply {
            body: "reply".into(),
        },
        "$reply",
    );
    rows.project_operations(actions.confirmed());
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events().len(), 2);
    assert_eq!(rows.events()[1].text, "reply");
    assert!(rows.events()[1].time.contains("Risposta"));
    assert_eq!(
        rows.events()[1].id.id,
        TimelineId::Server(EventId::new("$reply"))
    );
}

fn fixture() -> (DemoHistory, LiveOperations) {
    let scope = RoomScope {
        account_id: AccountId::new(21),
        room_id: RoomId::new("!room:test"),
    };
    let mut rows = DemoHistory::new(scope.clone(), 0);
    rows.prepend_server(vec![HistoryEvent {
        redacted: false,
        event_id: EventId::new("$first"),
        body: "old".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    let mut actions = LiveOperations::default();
    actions.select(ScopedTimelineId {
        scope,
        id: TimelineId::Server(EventId::new("$first")),
    });
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                edit: true,
                reply: true,
                react: true,
                redact: true,
            },
        }),
    });
    (rows, actions)
}
fn ack(actions: &mut LiveOperations, operation: Operation, event: &str) {
    assert!(actions.submit(operation, TransactionId::new(event)));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("apply");
    };
    assert!(actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new(event))
    }));
}

#[test]
fn acknowledged_edit_replaces_bubble_without_duplication_or_cross_room_change() {
    let scope = RoomScope {
        account_id: AccountId::new(21),
        room_id: RoomId::new("!room:test"),
    };
    let mut rows = DemoHistory::new(scope.clone(), 0);
    rows.prepend_server(vec![HistoryEvent {
        redacted: false,
        event_id: EventId::new("$first"),
        body: "old".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    let mut actions = LiveOperations::default();
    actions.select(ScopedTimelineId {
        scope: scope.clone(),
        id: TimelineId::Server(EventId::new("$first")),
    });
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                edit: true,
                ..Default::default()
            },
        }),
    });
    assert!(actions.submit(
        Operation::Edit {
            body: "edited".into()
        },
        TransactionId::new("edit")
    ));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("apply");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$edit")),
    });
    rows.project_operations(actions.confirmed());
    rows.project_operations(actions.confirmed());
    assert_eq!(rows.events().len(), 1);
    assert_eq!(rows.events()[0].text, "edited");
    assert_eq!(
        rows.events()[0].id.id,
        TimelineId::Server(EventId::new("$first"))
    );
    let other_scope = RoomScope {
        account_id: AccountId::new(22),
        ..scope
    };
    let mut other = DemoHistory::new(other_scope, 0);
    other.prepend_server(vec![HistoryEvent {
        redacted: false,
        event_id: EventId::new("$first"),
        body: "other".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    other.project_operations(actions.confirmed());
    assert_eq!(other.events()[0].text, "other");
}

#[test]
fn refresh_server_keeps_edited_suffix_on_local_clock_time() {
    let (mut rows, mut actions) = fixture();
    ack(&mut actions, Operation::Edit { body: "edited".into() }, "$edit");
    rows.project_operations(actions.confirmed());
    let ts = 1_784_158_200_000;
    rows.refresh_server(vec![HistoryEvent {
        redacted: false,
        event_id: EventId::new("$first"),
        body: "edited".into(),
        outgoing: true,
        timestamp_ms: ts,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    let expected = format!("{} · modificato", matrix_desktop::time_fmt::clock_time(ts));
    assert_eq!(rows.events()[0].time, expected);
}
