#![cfg(feature = "linux-a11y")]
//! regression probes for existing behavior, not a new implementation.
use accesskit::{Action, ActionRequest, TreeId};
use matrix_desktop::{
    LiveOperations,
    accessibility::operation_panel::{PanelIntent, PanelSnapshot},
};
use matrix_domain::{
    AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId, TransactionId,
};
use matrix_engine::operations::{
    MessageDetails, MessagePermissions, Operation, OperationCommand, OperationError,
    OperationResult,
};

#[test]
fn failed_redaction_retry_keeps_the_accepted_request_after_selection_changes() {
    let target = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(42),
            room_id: RoomId::new("!original:fixture"),
        },
        id: TimelineId::Server(EventId::new("$original")),
    };
    let mut model = LiveOperations::default();
    assert!(model.select(target.clone()));
    let Some(OperationCommand::Inspect { tag, target: event }) = model.take_command() else {
        panic!("expected inspection")
    };
    assert!(model.apply(OperationResult::Inspected {
        tag,
        target: event,
        details: Ok(MessageDetails {
            body: "fixture è 🙂".into(),
            permissions: MessagePermissions {
                redact: true,
                ..Default::default()
            },
        }),
    }));
    assert!(model.submit(Operation::Redact, TransactionId::new("accepted-redaction")));
    let Some(OperationCommand::Apply(accepted)) = model.take_command() else {
        panic!("expected accepted redaction")
    };
    assert!(model.retry_request().is_none());
    assert!(model.apply(OperationResult::Applied {
        request: accepted.clone(),
        result: Err(OperationError::Transport),
    }));
    let failed = PanelSnapshot::project(&model, false, None).unwrap();
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: failed.retry_id(),
        data: None,
    };
    let stale = failed.action(&request).unwrap();
    assert!(matches!(stale.intent, PanelIntent::Retry));
    assert!(failed.allows(&stale));
    assert_eq!(
        stale.retry_request.as_ref().unwrap().transaction_id,
        accepted.transaction_id
    );
    model.clear_selection();
    assert!(PanelSnapshot::project(&model, false, None).is_none());
    let mut other = target.clone();
    other.scope.room_id = RoomId::new("!other:fixture");
    other.id = TimelineId::Server(EventId::new("$other"));
    assert!(model.select(other));
    let mut current = PanelSnapshot::project(&model, false, None).unwrap();
    assert!(!current.allows(&stale));
    current.retry_focused = true;
    for key in ["space", "enter"] {
        let action = current.key(key, false, false).unwrap();
        assert!(matches!(action.intent, PanelIntent::Retry));
        assert!(current.allows(&action));
        let saved = action.retry_request.as_ref().unwrap();
        assert_eq!(saved.scope, accepted.scope);
        assert_eq!(saved.target, accepted.target);
        assert_eq!(saved.transaction_id, accepted.transaction_id);
        assert!(saved.operation == Operation::Redact);
    }
    assert!(model.retry());
    assert!(!model.retry());
    assert!(
        !PanelSnapshot::project(&model, false, None)
            .unwrap()
            .allows(&stale)
    );
    let Some(OperationCommand::Apply(retried)) = model.take_command() else {
        panic!("expected retained request before inspection")
    };
    assert_eq!(retried.scope, accepted.scope);
    assert_eq!(retried.target, accepted.target);
    assert_eq!(retried.transaction_id, accepted.transaction_id);
    assert!(retried.operation == accepted.operation);
    let mut wrong = accepted.clone();
    wrong.target = EventId::new("$other");
    assert!(!model.apply(OperationResult::Applied {
        request: wrong,
        result: Ok(EventId::new("$wrong-redaction")),
    }));
    assert!(model.apply(OperationResult::Applied {
        request: accepted.clone(),
        result: Ok(EventId::new("$redaction")),
    }));
    assert!(model.retry_request().is_none());
    assert!(!model.retry());
    assert_eq!(model.confirmed().len(), 1);
    assert_eq!(model.confirmed()[0].request.target, accepted.target);
    assert_eq!(
        model.confirmed()[0].request.transaction_id,
        accepted.transaction_id
    );
    assert_eq!(model.confirmed()[0].event_id, EventId::new("$redaction"));
}
