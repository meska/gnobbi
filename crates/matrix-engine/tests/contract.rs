use matrix_domain::{AccountId, RoomId, RoomScope, Timeline, TimelineUpdate, TransactionId};
use matrix_engine::{EngineCommand, EngineEvent, ViewRequests, bounded};
use std::num::NonZeroUsize;

#[test]
fn queued_history_is_rejected_after_switch_but_durable_send_is_retained() {
    let mut views = ViewRequests::new();
    let tag = views.start(scope()).unwrap();
    let (mut ui_commands, mut backend_commands) = bounded(NonZeroUsize::new(1).unwrap());
    let (mut backend_events, mut ui_events) = bounded(NonZeroUsize::new(1).unwrap());
    assert!(
        ui_commands
            .try_send(EngineCommand::LoadHistory {
                tag: tag.clone(),
                before: None
            })
            .is_ok()
    );
    let EngineCommand::LoadHistory {
        tag: worker,
        before,
    } = backend_commands.try_recv().ok().unwrap()
    else {
        panic!("wrong request")
    };
    assert!(before.is_none());
    assert!(!worker.is_cancelled());
    assert!(
        backend_events
            .try_send(EngineEvent::HistoryItem {
                tag: worker.clone(),
                event_id: matrix_domain::EventId::new("$old:fixture"),
                body: "old history".into()
            })
            .is_ok()
    );
    let mut next_scope = scope();
    next_scope.room_id = RoomId::new("!other:fixture");
    let current = views.start(next_scope).unwrap();
    assert!(worker.is_cancelled());
    let EngineEvent::HistoryItem { tag: received, .. } = ui_events.try_recv().ok().unwrap() else {
        panic!("wrong event")
    };
    assert!(!views.accepts(&received));
    assert!(views.accepts(&current));
    // Durable send state is routed to its room even though the view changed.
    assert!(
        backend_events
            .try_send(EngineEvent::Timeline {
                scope: scope(),
                update: TimelineUpdate::LocalSend {
                    transaction_id: TransactionId::new("offline-tx"),
                    body: "fixture".into()
                }
            })
            .is_ok()
    );
    let EngineEvent::Timeline {
        scope: route,
        update,
    } = ui_events.try_recv().ok().unwrap()
    else {
        panic!("wrong event")
    };
    let mut old_room = Timeline::new(scope());
    old_room.apply(&route, update).unwrap();
    assert_eq!(old_room.items().len(), 1);
}

#[test]
fn overflow_returns_original_send_command_for_explicit_retry() {
    let (mut commands, mut backend) = bounded(NonZeroUsize::new(1).unwrap());
    let mut views = ViewRequests::new();
    assert!(
        commands
            .try_send(EngineCommand::LoadHistory {
                tag: views.start(scope()).unwrap(),
                before: None
            })
            .is_ok()
    );
    let original = EngineCommand::SendText {
        scope: scope(),
        transaction_id: TransactionId::new("retry-tx"),
        body: "retained body".into(),
    };
    let Err(matrix_engine::TrySendError::Full(returned)) = commands.try_send(original) else {
        panic!("overflow must be explicit")
    };
    assert!(backend.try_recv().is_ok());
    assert!(commands.try_send(returned).is_ok());
    let EngineCommand::SendText {
        scope: actual_scope,
        transaction_id,
        body,
    } = backend.try_recv().ok().unwrap()
    else {
        panic!("send was lost")
    };
    assert_eq!(actual_scope, scope());
    assert_eq!(transaction_id, TransactionId::new("retry-tx"));
    assert_eq!(body, "retained body");
}

#[test]
fn active_history_item_can_be_applied_to_its_scoped_projection() {
    let mut views = ViewRequests::new();
    let tag = views.start(scope()).unwrap();
    let (mut backend, mut ui) = bounded(NonZeroUsize::new(1).unwrap());
    assert!(
        backend
            .try_send(EngineEvent::HistoryItem {
                tag,
                event_id: matrix_domain::EventId::new("$history:fixture"),
                body: "history".into()
            })
            .is_ok()
    );
    let EngineEvent::HistoryItem {
        tag,
        event_id,
        body,
    } = ui.try_recv().ok().unwrap()
    else {
        panic!("wrong event")
    };
    assert!(views.accepts(&tag));
    let mut timeline = Timeline::new(scope());
    timeline
        .apply(
            tag.scope(),
            TimelineUpdate::Echo {
                transaction_id: None,
                event_id,
                body,
            },
        )
        .unwrap();
    assert_eq!(timeline.items()[0].body, "history");
}

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!offline:fixture"),
    }
}

#[test]
fn send_command_and_durable_event_preserve_scope_and_identity() {
    fn thread_safe<T: Send + 'static>() {}
    thread_safe::<EngineCommand>();
    thread_safe::<EngineEvent>();
    let command = EngineCommand::SendText {
        scope: scope(),
        transaction_id: TransactionId::new("offline-tx"),
        body: "fixture".into(),
    };
    let EngineCommand::SendText {
        scope: route,
        transaction_id,
        body,
    } = command
    else {
        panic!("wrong command")
    };
    let event = EngineEvent::Timeline {
        scope: route,
        update: TimelineUpdate::LocalSend {
            transaction_id,
            body,
        },
    };
    let EngineEvent::Timeline {
        scope: route,
        update,
    } = event
    else {
        panic!("wrong event")
    };
    let mut timeline = Timeline::new(scope());
    timeline.apply(&route, update).unwrap();
    assert_eq!(timeline.items()[0].body, "fixture");
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("offline-tx"))
    );
}
