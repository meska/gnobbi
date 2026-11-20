use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, RoomId, RoomScope, TimelineId, TransactionId};

#[test]
fn composer_submission_appends_exact_text_with_scoped_local_identity() {
    let scope = RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!history:fixture.invalid"),
    };
    let mut history = DemoHistory::new(scope.clone(), 10);
    let original = history.events()[0].id.clone();
    history.submit_text(" caffè 🦀\nseconda riga ".into());
    history.prepend_older(2);
    history.append_live(1);
    history.submit_text("altro".into());
    assert_eq!(history.events().len(), 15);
    assert_eq!(history.events()[2].id, original);
    let sent = &history.events()[12];
    assert_eq!(sent.text, " caffè 🦀\nseconda riga ");
    assert!(sent.outgoing);
    assert_eq!(sent.id.scope, scope);
    assert_eq!(
        sent.id.id,
        TimelineId::Pending(TransactionId::new("demo-local-1"))
    );
    assert_eq!(
        history.events()[14].id.id,
        TimelineId::Pending(TransactionId::new("demo-local-2"))
    );
    assert_eq!(sent.time, "local");
}
