#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use matrix_desktop::{Composer, DemoHistory, LiveDelivery, accessibility::HistoryNodes};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, TransactionId};
use matrix_engine::sending::{SendError, SendResult};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(44),
        room_id: RoomId::new("!states:fixture.invalid"),
    }
}

#[test]
fn confirmed_redaction_has_semantics_not_inferred_from_message_body() {
    use matrix_desktop::LiveOperations;
    use matrix_engine::{
        history::HistoryEvent,
        operations::{
            MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
        },
    };
    let event = EventId::new("$original:fixture.invalid");
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        event_id: event.clone(),
        body: "[Messaggio eliminato]".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
    }]);
    let mut nodes = HistoryNodes::default();
    let original = nodes.project(&rows, 0, true).messages.remove(0);
    assert!(!original.description.contains("Stato: eliminato"));
    let mut operations = LiveOperations::default();
    assert!(operations.select(original.event.clone()));
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspect")
    };
    assert!(operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: original.text.clone(),
            permissions: MessagePermissions {
                redact: true,
                ..Default::default()
            }
        })
    }));
    assert!(operations.submit(Operation::Redact, TransactionId::new("redact")));
    let Some(OperationCommand::Apply(request)) = operations.take_command() else {
        panic!("write")
    };
    assert!(operations.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$redaction:fixture.invalid"))
    }));
    rows.project_operations(operations.confirmed());
    let redacted = nodes.project(&rows, 0, true).messages.remove(0);
    assert_eq!(redacted.id, original.id);
    assert_eq!(redacted.event, original.event);
    assert!(
        redacted.description.contains("Stato: eliminato"),
        "{}",
        redacted.description
    );
    assert!(
        redacted
            .description
            .contains("Ricevuta di lettura non disponibile")
    );
}

#[test]
fn own_reaction_is_named_separately_from_edit_and_delivery() {
    use matrix_desktop::LiveOperations;
    use matrix_engine::{
        history::HistoryEvent,
        operations::{
            MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
        },
    };
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![HistoryEvent {
        event_id: EventId::new("$react-target"),
        body: "original".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None, thread_root: None,
        status: false,
    }]);
    let mut nodes = HistoryNodes::default();
    let original = nodes.project(&rows, 0, true).messages.remove(0);
    let mut operations = LiveOperations::default();
    operations.select(original.event.clone());
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspect")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "original".into(),
            permissions: MessagePermissions {
                edit: true,
                react: true,
                redact: true,
                ..Default::default()
            },
        }),
    });
    for (index, operation) in [
        Operation::Edit {
            body: "nuovo 🙂".into(),
        },
        Operation::React { key: "+1".into() },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(operations.submit(operation, TransactionId::new(format!("op{index}"))));
        let Some(OperationCommand::Apply(request)) = operations.take_command() else {
            panic!("write")
        };
        operations.apply(OperationResult::Applied {
            request,
            result: Ok(EventId::new(format!("$op{index}"))),
        });
    }
    rows.project_operations(operations.confirmed());
    let modified = nodes.project(&rows, 0, true).messages.remove(0);
    assert_eq!(modified.id, original.id);
    assert_eq!(modified.event, original.event);
    assert_eq!(modified.text, "nuovo 🙂");
    assert!(modified.description.contains("modificato"));
    assert!(
        modified.description.contains("Reazione propria: +1 · tu"),
        "{}",
        modified.description
    );
    assert!(operations.toggle_reaction("+1", TransactionId::new("remove")));
    let Some(OperationCommand::Apply(request)) = operations.take_command() else {
        panic!("remove reaction")
    };
    operations.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$remove")),
    });
    rows.project_operations(operations.confirmed());
    let removed = nodes.project(&rows, 0, true).messages.remove(0);
    assert_eq!(removed.id, original.id);
    assert!(!removed.description.contains("Reazione propria"));
    assert!(removed.description.contains("modificato"));
    rows.prepend_older(25);
    assert_eq!(
        nodes.project(&rows, 25, true).messages[0].description,
        removed.description
    );
    assert_eq!(nodes.project(&rows, 25, true).messages[0].id, removed.id);
    let other = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(45),
            ..scope()
        },
        1,
    );
    assert_ne!(nodes.project(&other, 0, true).messages[0].id, removed.id);
    assert_eq!(
        nodes.project(&rows, 25, true).messages[0].description,
        removed.description
    );
}

#[test]
fn public_peer_receipt_names_only_the_exact_scoped_event() {
    use matrix_engine::{history::HistoryEvent, receipts::PublicReadReceipt};
    let mut rows = DemoHistory::new(scope(), 0);
    rows.prepend_server(vec![
        HistoryEvent {
            event_id: EventId::new("$read"),
            body: "original".into(),
            outgoing: true,
            timestamp_ms: 0,
            sender: matrix_engine::history::HistorySender::new("@peer:test", None),
            video: None,
            styles: Vec::new(),
            thread: None, thread_root: None,
            status: false,
        },
        HistoryEvent {
            event_id: EventId::new("$later"),
            body: "later".into(),
            outgoing: true,
            timestamp_ms: 1,
            sender: matrix_engine::history::HistorySender::new("@peer:test", None),
            video: None,
            styles: Vec::new(),
            thread: None, thread_root: None,
            status: false,
        },
    ]);
    let mut nodes = HistoryNodes::default();
    let mut snapshot = nodes.project(&rows, 0, true);
    let original = snapshot.messages[0].id;
    let receipts = vec![PublicReadReceipt {
        event_id: EventId::new("$read"),
        readers: 1,
    }];
    snapshot.apply_public_receipts(&scope(), &receipts);
    assert_eq!(snapshot.messages[0].id, original);
    assert!(
        snapshot.messages[0]
            .description
            .contains("Confermato dal server")
    );
    assert!(
        snapshot.messages[0]
            .description
            .contains("Ricevuta pubblica su questo evento: 1 peer")
    );
    assert!(!snapshot.messages[0].description.contains("non disponibile"));
    assert!(
        snapshot.messages[1]
            .description
            .contains("Ricevuta di lettura non disponibile")
    );
    let unchanged = snapshot.messages[0].description.clone();
    snapshot.apply_public_receipts(&scope(), &receipts);
    assert_eq!(snapshot.messages[0].description, unchanged);
    let advanced = vec![PublicReadReceipt {
        event_id: EventId::new("$later"),
        readers: 1,
    }];
    let mut snapshot = nodes.project(&rows, 0, true);
    snapshot.apply_public_receipts(&scope(), &advanced);
    assert!(
        snapshot.messages[0]
            .description
            .contains("Ricevuta di lettura non disponibile")
    );
    assert!(
        snapshot.messages[1]
            .description
            .contains("Ricevuta pubblica su questo evento: 1 peer")
    );
    for foreign in [
        RoomScope {
            account_id: AccountId::new(99),
            ..scope()
        },
        RoomScope {
            room_id: RoomId::new("!other:test"),
            ..scope()
        },
    ] {
        let mut snapshot = nodes.project(&rows, 0, true);
        snapshot.apply_public_receipts(&foreign, &receipts);
        assert!(
            snapshot
                .messages
                .iter()
                .all(|m| !m.description.contains("Ricevuta pubblica"))
        );
    }
    rows.prepend_older(25);
    let mut snapshot = nodes.project(&rows, 25, true);
    snapshot.apply_public_receipts(&scope(), &receipts);
    assert_eq!(snapshot.messages[0].id, original);
    assert_eq!(snapshot.messages[0].description, unchanged);
}

#[test]
fn offline_history_explicitly_disclaims_matrix_delivery() {
    let fixture = DemoHistory::new(scope(), 3);
    let mut nodes = HistoryNodes::default();
    let mut snapshot = nodes.project(&fixture, 0, false);
    snapshot.mark_offline();
    for message in snapshot.messages {
        assert!(
            message
                .description
                .contains("Demo offline · Nessuna consegna Matrix")
        );
        assert!(!message.description.contains("Confermato dal server"));
        assert!(message.actions.is_none());
        assert!(message.retry.is_none());
    }
}

#[test]
fn server_ack_is_explicitly_not_a_peer_read_receipt() {
    let mut rows = DemoHistory::new(scope(), 0);
    let mut delivery = LiveDelivery::new();
    let mut editor = Composer::default();
    editor.replace(None, "caffè 🙂");
    let tx = TransactionId::new("state-1");
    assert!(delivery.submit(scope(), tx.clone(), &mut editor));
    delivery.take_request().unwrap();
    rows.project_delivery(delivery.items(&scope()));
    let mut nodes = HistoryNodes::default();
    let pending = nodes.project(&rows, 0, true).messages.remove(0);
    assert!(pending.description.contains("In attesa"));
    assert!(!pending.description.contains("Confermato dal server"));
    delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Err(SendError::Transport),
    });
    rows.project_delivery(delivery.items(&scope()));
    let failed = nodes.project(&rows, 0, true).messages.remove(0);
    assert_eq!(failed.id, pending.id);
    assert!(failed.description.contains("Invio fallito"));
    assert!(delivery.retry(&scope(), &tx));
    assert!(
        !delivery.retry(&scope(), &tx),
        "duplicate retry cannot enqueue another send"
    );
    let retried = delivery.take_request().unwrap();
    assert_eq!(retried.scope, scope());
    assert_eq!(retried.transaction_id, tx);
    assert_eq!(retried.body, "caffè 🙂");
    rows.project_delivery(delivery.items(&scope()));
    assert_eq!(nodes.project(&rows, 0, true).messages[0].id, pending.id);
    let event = EventId::new("$state-ack:fixture.invalid");
    delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx,
        result: Ok(event),
    });
    rows.project_delivery(delivery.items(&scope()));
    let ack = nodes.project(&rows, 0, true).messages.remove(0);
    assert_eq!(
        ack.id, pending.id,
        "ACK must preserve the focused semantic node"
    );
    assert!(
        ack.description.contains("Confermato dal server"),
        "{}",
        ack.description
    );
    assert!(
        ack.description
            .contains("Ricevuta di lettura non disponibile")
    );
    assert!(!ack.description.contains("Invio fallito"));
    assert!(!ack.description.contains("In attesa"));
    assert_eq!(rows.events().len(), 1);
    let fixture = DemoHistory::new(scope(), 3);
    for item in nodes.project(&fixture, 0, false).messages {
        assert!(
            !item.description.contains("Confermato dal server"),
            "offline is not server delivery"
        );
    }
}
