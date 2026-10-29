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
        styles: Vec::new(),
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
        styles: Vec::new(),
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
            styles: Vec::new(),
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
            styles: Vec::new(),
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
fn too_large_failure_says_so_and_cannot_be_retried() {
    use matrix_engine::sending::{SendError, SendResult};
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "big");
    let tx = TransactionId::new("big-1");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult { scope: scope(), transaction_id: tx.clone(), result: Err(SendError::TooLarge) }));
    assert_eq!(delivery.failure_label(&scope(), &tx), Some("File troppo grande per il server"));
    assert!(delivery.is_final_failure(&scope(), &tx));
    assert!(!delivery.retry(&scope(), &tx));
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

fn note_file(name: &str) -> matrix_engine::sending::Attachment {
    matrix_engine::sending::Attachment {
        filename: name.into(),
        data: vec![7; 4],
        voice: Some(matrix_engine::sending::Voice {
            duration_ms: 1500,
            waveform: vec![0.5],
        }),
    }
}

#[test]
fn attachment_stays_with_the_composer_when_the_queue_is_full_or_the_id_repeats() {
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    for n in 0..8 {
        let mut pending = Some(note_file("n.wav"));
        assert!(delivery.submit_with(
            scope(),
            TransactionId::new(format!("q{n}")),
            &mut draft,
            &mut pending
        ));
    }
    // Ninth send while eight are staged: refused, the recording is not lost.
    let mut pending = Some(note_file("nona.wav"));
    assert!(!delivery.submit_with(scope(), TransactionId::new("q9"), &mut draft, &mut pending));
    assert_eq!(pending.as_ref().unwrap().filename, "nona.wav");
    // Same for a transaction id already on the timeline.
    while delivery.take_request().is_some() {}
    let mut pending = Some(note_file("bis.wav"));
    assert!(!delivery.submit_with(scope(), TransactionId::new("q0"), &mut draft, &mut pending));
    assert!(pending.is_some());
    assert!(delivery.take_request().is_none());
}

#[test]
fn whitespace_caption_shows_the_file_name_and_voice_markers_survive_retry() {
    use matrix_engine::sending::{SendError, SendResult};
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "  \n ");
    let tx = TransactionId::new("voce");
    let mut pending = Some(note_file("nota-vocale.wav"));
    assert!(delivery.submit_with(scope(), tx.clone(), &mut draft, &mut pending));
    assert_eq!(
        delivery.items(&scope()).last().unwrap().body,
        "nota-vocale.wav"
    );
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Err(SendError::Transport)
    }));
    assert!(delivery.retry(&scope(), &tx));
    let again = delivery.take_request().unwrap();
    let file = again
        .attachment
        .expect("retry resends the file, not only the caption");
    assert_eq!(
        (file.data, file.voice.unwrap().duration_ms),
        (vec![7; 4], 1500)
    );
}

#[test]
fn a_confirmed_attachment_cannot_be_retried_as_caption_only() {
    use matrix_domain::EventId;
    use matrix_engine::sending::SendResult;
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "didascalia");
    let tx = TransactionId::new("ok");
    let mut pending = Some(note_file("a.wav"));
    assert!(delivery.submit_with(scope(), tx.clone(), &mut draft, &mut pending));
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult {
        scope: scope(),
        transaction_id: tx.clone(),
        result: Ok(EventId::new("$e"))
    }));
    assert!(!delivery.retry(&scope(), &tx));
    assert!(
        delivery.take_request().is_none(),
        "no text-only resend of a delivered file"
    );
}

/// A send queue that holds the texts it was told about and records what the window asked of it.
#[derive(Default)]
struct FakeQueue {
    held: std::sync::Mutex<Vec<TransactionId>>,
    calls: std::sync::Mutex<Vec<String>>,
}
impl matrix_engine::sending::SendControl for FakeQueue {
    fn retry(&self, id: &TransactionId) -> bool {
        self.calls.lock().unwrap().push(format!("retry {}", id.as_str()));
        self.held.lock().unwrap().contains(id)
    }
    fn cancel(&self, id: &TransactionId) -> bool {
        self.calls.lock().unwrap().push(format!("cancel {}", id.as_str()));
        self.held.lock().unwrap().contains(id)
    }
    fn edit(&self, id: &TransactionId, body: &str, _: &[matrix_engine::sending::Mention]) -> bool {
        self.calls.lock().unwrap().push(format!("edit {} {body}", id.as_str()));
        self.held.lock().unwrap().contains(id)
    }
}

fn queued_delivery(held: &[&str]) -> (LiveDelivery, std::sync::Arc<FakeQueue>) {
    let queue = std::sync::Arc::new(FakeQueue::default());
    *queue.held.lock().unwrap() = held.iter().map(|id| TransactionId::new(*id)).collect();
    let slot = matrix_engine::sending::ControlSlot::default();
    *slot.lock().unwrap() = Some(queue.clone());
    (LiveDelivery::with_control(slot), queue)
}

#[test]
fn a_failed_text_in_the_send_queue_is_retried_there_not_sent_again() {
    use matrix_engine::sending::{SendError, SendResult};
    let (mut delivery, queue) = queued_delivery(&["q1"]);
    let mut draft = Composer::default();
    draft.replace(None, "queued text");
    let tx = TransactionId::new("q1");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    assert!(delivery.apply(SendResult { scope: scope(), transaction_id: tx.clone(), result: Err(SendError::Transport) }));
    assert!(delivery.retry(&scope(), &tx));
    assert_eq!(delivery.items(&scope())[0].status, SendStatus::Pending);
    assert!(delivery.take_request().is_none(), "the queue resends, the window stages nothing");
    assert_eq!(*queue.calls.lock().unwrap(), ["retry q1"]);
}

#[test]
fn cancel_waits_for_the_queue_and_then_removes_the_message() {
    use matrix_engine::sending::SendNotice;
    let (mut delivery, queue) = queued_delivery(&["q2"]);
    let mut draft = Composer::default();
    draft.replace(None, "withdraw me");
    let tx = TransactionId::new("q2");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    assert!(delivery.cancel(&scope(), &tx));
    assert_eq!(delivery.items(&scope()).len(), 1, "still shown until the queue confirms");
    assert!(delivery.notice(scope(), tx.clone(), SendNotice::Cancelled));
    assert!(delivery.items(&scope()).is_empty());
    assert_eq!(*queue.calls.lock().unwrap(), ["cancel q2"]);
}

#[test]
fn cancel_and_edit_of_a_message_not_yet_handed_over_act_on_the_staged_request() {
    let mut delivery = LiveDelivery::new();
    let mut draft = Composer::default();
    draft.replace(None, "first");
    let (keep, drop_) = (TransactionId::new("keep"), TransactionId::new("drop"));
    assert!(delivery.submit(scope(), keep.clone(), &mut draft));
    draft.replace(None, "second");
    assert!(delivery.submit(scope(), drop_.clone(), &mut draft));
    assert!(delivery.edit(&scope(), &keep, "first, corrected"));
    assert!(!delivery.edit(&scope(), &keep, "  "), "an empty text is no edit");
    assert!(delivery.cancel(&scope(), &drop_));
    assert!(!delivery.cancel(&scope(), &drop_), "already gone");
    let request = delivery.take_request().unwrap();
    assert_eq!((request.transaction_id, request.body.as_str()), (keep, "first, corrected"));
    assert!(delivery.take_request().is_none());
    assert_eq!(delivery.items(&scope()).len(), 1);
    assert_eq!(delivery.items(&scope())[0].body, "first, corrected");
}

#[test]
fn queue_edit_updates_the_shown_text_and_a_sent_message_cannot_be_withdrawn() {
    use matrix_engine::sending::{SendNotice, SendResult};
    let (mut delivery, queue) = queued_delivery(&["q3"]);
    let mut draft = Composer::default();
    draft.replace(None, "draft");
    let tx = TransactionId::new("q3");
    assert!(delivery.submit(scope(), tx.clone(), &mut draft));
    delivery.take_request().unwrap();
    assert!(delivery.edit(&scope(), &tx, "draft 2"));
    assert!(delivery.notice(scope(), tx.clone(), SendNotice::Edited { body: "draft 2".into() }));
    assert_eq!(delivery.items(&scope())[0].body, "draft 2");
    assert!(delivery.apply(SendResult { scope: scope(), transaction_id: tx.clone(), result: Ok(matrix_domain::EventId::new("$sent")) }));
    assert!(!delivery.notice(scope(), tx.clone(), SendNotice::Cancelled));
    assert_eq!(delivery.items(&scope())[0].status, SendStatus::Sent);
    assert_eq!(*queue.calls.lock().unwrap(), ["edit q3 draft 2"]);
}

#[test]
fn messages_left_in_the_queue_by_an_earlier_run_reappear_pending_or_failed() {
    use matrix_engine::sending::SendNotice;
    let mut delivery = LiveDelivery::new();
    let (a, b) = (TransactionId::new("old-a"), TransactionId::new("old-b"));
    assert!(delivery.notice(scope(), a.clone(), SendNotice::Restored { body: "still going".into(), failed: false }));
    assert!(delivery.notice(scope(), b.clone(), SendNotice::Restored { body: "wedged".into(), failed: true }));
    let items = delivery.items(&scope());
    assert_eq!((items[0].status.clone(), items[0].body.as_str()), (SendStatus::Pending, "still going"));
    assert_eq!((items[1].status.clone(), items[1].body.as_str()), (SendStatus::Failed, "wedged"));
    assert_eq!(delivery.failure_label(&scope(), &b), Some("Invio non confermato · Matrix"));
    // The queue tries the failed one again: it is pending, and the label is gone.
    assert!(delivery.notice(scope(), b.clone(), SendNotice::Retrying));
    assert_eq!(delivery.items(&scope())[1].status, SendStatus::Pending);
    assert_eq!(delivery.failure_label(&scope(), &b), None);
}
