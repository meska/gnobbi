use matrix_desktop::LiveOperations;
use matrix_domain::TransactionId;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};
use matrix_engine::operations::{
    MessageDetails, MessagePermissions, OperationCommand, OperationResult,
};
use matrix_engine::operations::{Operation, OperationError};

#[test]
fn accepted_write_survives_selection_clear_and_retries_same_transaction() {
    let mut actions = LiveOperations::default();
    let selected = id("!room:test", "$first");
    actions.select(selected.clone());
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
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
        TransactionId::new("tx")
    ));
    actions.clear_selection();
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("accepted write must not be cancelled");
    };
    assert!(request.scope == selected.scope);
    assert!(actions.apply(OperationResult::Applied {
        request: request.clone(),
        result: Err(OperationError::Transport)
    }));
    assert!(actions.retry());
    let Some(OperationCommand::Apply(retry)) = actions.take_command() else {
        panic!("retry");
    };
    assert!(retry.transaction_id == request.transaction_id && retry.operation == request.operation);
    assert!(actions.apply(OperationResult::Applied {
        request: retry,
        result: Ok(EventId::new("$edited"))
    }));
    assert_eq!(actions.confirmed().len(), 1);
    assert!(!actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$edited"))
    }));
    assert_eq!(actions.confirmed().len(), 1);
}

#[test]
fn forbidden_draft_action_preserves_editor_and_success_consumes_it() {
    let mut actions = LiveOperations::default();
    let mut draft = matrix_desktop::Composer::default();
    draft.replace(None, "edited Unicode è\nlinea");
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            },
        }),
    });
    assert!(!actions.submit_draft(true, TransactionId::new("denied"), &mut draft));
    assert_eq!(draft.text(), "edited Unicode è\nlinea");
    assert!(actions.submit_draft(false, TransactionId::new("reply"), &mut draft));
    assert!(draft.text().is_empty());
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("reply");
    };
    assert!(
        request.operation
            == Operation::Reply {
                body: "edited Unicode è\nlinea".into()
            }
    );
}

#[test]
fn forbidden_result_releases_write_slot_for_another_action() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        reply: true,
        ..Default::default()
    });
    actions.submit(
        Operation::Edit {
            body: "edit".into(),
        },
        TransactionId::new("tx"),
    );
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("write");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Forbidden),
    });
    assert!(!actions.retry());
    assert!(actions.submit(
        Operation::Reply {
            body: "reply".into()
        },
        TransactionId::new("reply")
    ));
}

#[test]
fn failed_write_remains_retryable_after_inspecting_another_room() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        ..Default::default()
    });
    actions.submit(
        Operation::Edit {
            body: "edit".into(),
        },
        TransactionId::new("tx"),
    );
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("write");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Transport),
    });
    actions.select(id("!other:test", "$second"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Err(OperationError::Unavailable),
    });
    assert!(actions.retry());
}

#[test]
fn toggling_own_reaction_redacts_its_server_id() {
    let mut actions = inspected(MessagePermissions {
        react: true,
        ..Default::default()
    });
    assert!(actions.toggle_reaction("+1", TransactionId::new("react")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("react");
    };
    assert!(request.operation == Operation::React { key: "+1".into() });
    actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$reaction")),
    });
    assert!(actions.toggle_reaction("+1", TransactionId::new("remove")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("remove");
    };
    assert!(request.operation == Operation::Redact);
    assert_eq!(request.target, EventId::new("$reaction"));
}

#[test]
fn empty_draft_is_not_consumed_or_queued() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        ..Default::default()
    });
    let mut draft = matrix_desktop::Composer::default();
    draft.replace(None, "  \n");
    assert!(!actions.submit_draft(true, TransactionId::new("empty"), &mut draft));
    assert_eq!(draft.text(), "  \n");
    assert!(actions.take_command().is_none());
}

fn inspected(permissions: MessagePermissions) -> LiveOperations {
    let mut actions = LiveOperations::default();
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions,
        }),
    });
    actions
}

fn id(room: &str, event: &str) -> ScopedTimelineId {
    ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(21),
            room_id: RoomId::new(room),
        },
        id: TimelineId::Server(EventId::new(event)),
    }
}
#[test]
fn inspection_rejects_stale_selection_even_with_same_scope() {
    let mut actions = LiveOperations::default();
    assert!(actions.select(id("!room:test", "$first")));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    assert!(actions.select(id("!room:test", "$second")));
    assert!(!actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "stale".into(),
            permissions: MessagePermissions::default()
        })
    }));
    assert!(actions.details().is_none());
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    assert!(actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "current".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            }
        })
    }));
    assert!(actions.details().unwrap().body == "current");
    actions.clear_selection();
    assert!(actions.selection().is_none());
}

#[test]
fn selecting_another_message_preserves_queued_write() {
    let mut actions = LiveOperations::default();
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                react: true,
                ..Default::default()
            },
        }),
    });
    assert!(actions.submit(
        Operation::React { key: "+1".into() },
        TransactionId::new("tx")
    ));
    actions.select(id("!other:test", "$second"));
    assert!(matches!(
        actions.take_command(),
        Some(OperationCommand::Apply(_))
    ));
}
