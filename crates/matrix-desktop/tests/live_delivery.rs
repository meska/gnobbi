use matrix_desktop::{Composer, LiveDelivery};
use matrix_domain::{AccountId, RoomId, RoomScope, SendStatus, TransactionId};
fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(20),
        room_id: RoomId::new("!send:fixture.invalid"),
    }
}
#[test]
fn encryption_failure_has_a_distinct_visible_label_until_retry() {
    use matrix_engine::sending::{SendError, SendResult};
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "encrypted draft");
    let tx = TransactionId::new("crypto");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Err(SendError::Encryption),
    }));
    assert_eq!(
        delivery.failure_label(&scope(), &tx),
        Some("Cifratura non riuscita · nessun invio in chiaro")
    );
    let mut other = scope();
    other.account_id = AccountId::new(99);
    assert_eq!(delivery.failure_label(&other, &tx), None);
    assert!(delivery.retry(&scope(), &tx));
    assert_eq!(delivery.failure_label(&scope(), &tx), None);
}

#[test]
fn rendered_ack_merges_history_echo_without_duplicate_bubble() {
    use matrix_desktop::DemoHistory;
    use matrix_domain::{EventId, TimelineId};
    use matrix_engine::{history::HistoryEvent, sending::SendResult};
    let mut delivery = LiveDelivery::new();
    let mut rows = DemoHistory::new(scope(), 0);
    let mut draft = Composer::default();
    draft.replace(None, "own message");
    let tx = TransactionId::new("ack");
    delivery.submit(scope(), tx.clone(), &mut draft);
    rows.project_delivery(delivery.items(&scope()));
    assert_eq!(rows.events().len(), 1);
    let event = EventId::new("$actual:fixture.invalid");
    rows.prepend_server(vec![HistoryEvent {
        event_id: event.clone(),
        body: "own message".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
    }]);
    delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Ok(event.clone()),
    });
    rows.project_delivery(delivery.items(&scope()));
    assert_eq!(rows.events().len(), 1);
    assert_eq!(rows.events()[0].id.id, TimelineId::Server(event.clone()));
    assert_eq!(rows.events()[0].transaction_id, Some(tx));
    assert_eq!(rows.events()[0].send_state, None);
    assert_eq!(rows.events()[0].time, "ACK server");
    rows.prepend_server(vec![HistoryEvent {
        event_id: event,
        body: "own message".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
    }]);
    assert_eq!(
        rows.events().len(),
        1,
        "history after ACK also deduplicates"
    );
}

#[test]
fn repeated_ack_projection_preserves_server_content_after_edit_or_redaction() {
    use matrix_desktop::DemoHistory;
    use matrix_domain::EventId;
    use matrix_engine::{history::HistoryEvent, sending::SendResult};
    for body in ["edited città 🦀", "[Messaggio eliminato]"] {
        let mut delivery = LiveDelivery::new();
        let mut draft = Composer::default();
        draft.replace(None, "stale original");
        let tx = TransactionId::new("confirmed");
        delivery.submit(scope(), tx.clone(), &mut draft);
        delivery.apply(SendResult {
            scope: scope(),
            transaction_id: tx,
            result: Ok(EventId::new("$confirmed")),
        });
        let mut rows = DemoHistory::new(scope(), 0);
        rows.prepend_server(vec![HistoryEvent {
            event_id: EventId::new("$confirmed"),
            body: body.into(),
            outgoing: true,
            timestamp_ms: 1,
            sender: matrix_engine::history::HistorySender::new("@peer:test", None),
            video: None,
        }]);
        rows.project_delivery(delivery.items(&scope()));
        rows.project_delivery(delivery.items(&scope()));
        assert_eq!(rows.events()[0].text, body);
    }
}

#[test]
fn appended_live_echo_merges_pending_row_in_both_ack_orders() {
    use matrix_desktop::DemoHistory;
    use matrix_domain::EventId;
    use matrix_engine::{history::HistoryEvent, sending::SendResult};
    for echo_first in [false, true] {
        let mut delivery = LiveDelivery::new();
        let mut rows = DemoHistory::new(scope(), 0);
        let mut draft = Composer::default();
        draft.replace(None, "same text città 🦀");
        let tx = TransactionId::new("live-echo");
        delivery.submit(scope(), tx.clone(), &mut draft);
        rows.project_delivery(delivery.items(&scope()));
        let echo = || HistoryEvent {
            event_id: EventId::new("$echo"),
            body: "same text città 🦀".into(),
            outgoing: true,
            timestamp_ms: 1,
            sender: matrix_engine::history::HistorySender::new("@peer:test", None),
            video: None,
        };
        if echo_first {
            rows.append_server(vec![echo()]);
        }
        delivery.apply(SendResult {
            scope: scope(),
            transaction_id: tx,
            result: Ok(EventId::new("$echo")),
        });
        rows.project_delivery(delivery.items(&scope()));
        rows.append_server(vec![echo(), echo()]);
        rows.project_delivery(delivery.items(&scope()));
        assert_eq!(rows.events().len(), 1);
        assert_eq!(rows.events()[0].text, "same text città 🦀");
        assert!(rows.events()[0].server_confirmed);
    }
}

#[test]
fn full_transport_retains_commands_until_later_tick() {
    let (mut sender, mut receiver) =
        matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    let mut delivery = LiveDelivery::new();
    for tx in ["a", "b"] {
        let mut draft = Composer::default();
        draft.replace(None, tx);
        assert!(delivery.submit(scope(), TransactionId::new(tx), &mut draft));
    }
    delivery.flush(&mut sender);
    assert_eq!(
        receiver
            .try_recv()
            .unwrap_or_else(|_| panic!("first command"))
            .body,
        "a"
    );
    delivery.flush(&mut sender);
    assert_eq!(
        receiver
            .try_recv()
            .unwrap_or_else(|_| panic!("second command retained"))
            .body,
        "b"
    );
    assert!(delivery.take_request().is_none());
}

#[test]
fn failed_retry_keeps_transaction_body_and_unrelated_draft() {
    use matrix_engine::sending::{SendError, SendResult};
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "città\nretry body");
    let tx = TransactionId::new("retry-1");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    draft.replace(None, "next unsent draft");
    assert!(delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Err(SendError::Transport)
    }));
    assert_eq!(delivery.items(&scope())[0].status, SendStatus::Failed);
    assert_eq!(
        delivery.failure_label(&scope(), &tx),
        Some("Invio non confermato · Matrix")
    );
    assert!(delivery.retry(&scope(), &tx));
    assert!(
        !delivery.retry(&scope(), &tx),
        "pending attempt cannot retry twice"
    );
    let request = delivery.take_request().unwrap();
    assert_eq!(request.transaction_id, tx);
    assert_eq!(request.body, "città\nretry body");
    assert_eq!(delivery.items(&scope()).len(), 1);
    assert_eq!(draft.text(), "next unsent draft");
}

#[test]
fn saturated_staging_retains_rejected_draft() {
    let mut delivery = LiveDelivery::new();
    for index in 0..8 {
        let mut draft = Composer::default();
        draft.replace(None, "queued");
        assert!(delivery.submit(scope(), TransactionId::new(index.to_string()), &mut draft));
    }
    let mut draft = Composer::default();
    draft.replace(None, "still mine\nUnicode città");
    assert!(!delivery.submit(scope(), TransactionId::new("overflow"), &mut draft));
    assert_eq!(draft.text(), "still mine\nUnicode città");
    assert_eq!(delivery.items(&scope()).len(), 8);
}

#[test]
fn closed_transport_marks_outgoing_row_failed() {
    let (mut sender, receiver) = matrix_engine::bounded(std::num::NonZeroUsize::new(1).unwrap());
    drop(receiver);
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "retained outgoing body");
    assert!(delivery.submit(scope(), TransactionId::new("closed"), &mut draft));
    delivery.flush(&mut sender);
    assert_eq!(delivery.items(&scope())[0].status, SendStatus::Failed);
    assert_eq!(delivery.items(&scope())[0].body, "retained outgoing body");
    assert!(delivery.take_request().is_none());
}

#[test]
fn wrong_room_ack_cannot_promote_local_send() {
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "original");
    let tx = TransactionId::new("scoped");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    let mut other = scope();
    other.room_id = RoomId::new("!other:fixture.invalid");
    assert!(!delivery.apply(matrix_engine::sending::SendResult {
        scope: other,
        transaction_id: tx,
        result: Ok(matrix_domain::EventId::new("$wrong:fixture.invalid")),
    }));
    assert_eq!(delivery.items(&scope())[0].status, SendStatus::Pending);
}

#[test]
fn accepted_send_retains_exact_body_in_pending_row() {
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "città\nseconda riga");
    let tx = TransactionId::new("unique-session-1");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    assert_eq!(draft.text(), "");
    let item = delivery.items(&scope())[0].clone();
    assert_eq!(item.body, "città\nseconda riga");
    assert_eq!(item.status, SendStatus::Pending);
    let request = delivery.take_request().unwrap();
    assert_eq!(request.body, item.body);
    assert_eq!(request.transaction_id, tx);
    assert!(delivery.take_request().is_none());
}

#[test]
fn attachment_is_sent_with_the_draft_as_caption_and_survives_retry() {
    use matrix_engine::sending::{Attachment, SendError, SendResult};
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    let tx = TransactionId::new("file");
    let file = || Attachment {
        filename: "foto.png".into(),
        data: vec![1, 2, 3],
        voice: None,
    };
    // No text and no file: nothing to send; a refused send keeps the file.
    assert!(!delivery.submit_with(scope(), tx.clone(), &mut draft, &mut None));
    draft.replace(None, "didascalia");
    let mut pending = Some(file());
    assert!(delivery.submit_with(scope(), tx.clone(), &mut draft, &mut pending));
    assert!(pending.is_none(), "an accepted send takes the file");
    let request = delivery.take_request().unwrap();
    assert_eq!(request.body, "didascalia");
    assert_eq!(request.attachment.unwrap().data, vec![1, 2, 3]);
    // File without caption: the local row shows the file name.
    let tx2 = TransactionId::new("file-only");
    let mut pending = Some(file());
    assert!(delivery.submit_with(scope(), tx2.clone(), &mut draft, &mut pending));
    assert_eq!(delivery.items(&scope()).last().unwrap().body, "foto.png");
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx2.clone(),
        result: Err(SendError::Transport),
    }));
    assert!(delivery.retry(&scope(), &tx2));
    let again = delivery.take_request().unwrap();
    assert_eq!(again.body, "");
    assert_eq!(again.attachment.unwrap().filename, "foto.png");
}
