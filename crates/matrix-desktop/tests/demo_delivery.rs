use matrix_desktop::{DemoHistory, DemoSendState};
use matrix_domain::{AccountId, RoomId, RoomScope, TimelineId};

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(0),
        room_id: RoomId::new("!a:fixture.invalid"),
    }
}

#[test]
fn retry_reuses_identity_and_body_without_appending_or_touching_other_rows() {
    let mut history = DemoHistory::new(scope(), 3);
    history.submit_text("retry caffè 🦀\nunchanged".into());
    let target = history.events()[3].id.clone();
    assert!(!history.retry_local(&target));
    assert!(history.fail_local(&target));
    history.prepend_older(2);
    history.append_live(1);
    let mut wrong = target.clone();
    wrong.scope.account_id = AccountId::new(99);
    assert!(!history.retry_local(&wrong));
    assert!(!history.retry_local(&history.events()[0].id.clone()));
    for _ in 0..3 {
        assert!(history.retry_local(&target));
        assert!(!history.retry_local(&target));
        let event = history
            .events()
            .iter()
            .find(|event| event.id == target)
            .unwrap();
        assert_eq!(event.text, "retry caffè 🦀\nunchanged");
        assert_eq!(event.send_state, Some(DemoSendState::Pending));
        assert_eq!(history.events().len(), 7);
        assert!(history.fail_local(&target));
    }
}

#[test]
fn simulated_failure_changes_only_the_scoped_local_message() {
    let mut history = DemoHistory::new(scope(), 3);
    history.submit_text("same text".into());
    history.submit_text("same text".into());
    let target = history.events()[3].id.clone();
    let mut other = target.clone();
    other.scope.account_id = AccountId::new(1);
    assert!(!history.fail_local(&other));
    other.scope = scope();
    other.scope.room_id = RoomId::new("!other:fixture.invalid");
    assert!(!history.fail_local(&other));
    assert!(!history.fail_local(&history.events()[0].id.clone()));
    assert!(history.fail_local(&target));
    assert!(!history.fail_local(&target));
    assert_eq!(history.events()[3].send_state, Some(DemoSendState::Failed));
    assert_eq!(history.events()[4].send_state, Some(DemoSendState::Pending));
    assert_eq!(history.events()[3].id, target);
    assert_eq!(history.events()[3].text, "same text");
    assert_eq!(history.events().len(), 5);
}

#[test]
fn local_submission_is_explicitly_pending_not_server_delivery() {
    let mut history = DemoHistory::new(scope(), 3);
    assert!(
        history
            .events()
            .iter()
            .all(|event| event.send_state.is_none())
    );
    history.submit_text("locale 🦀\noriginale".into());
    let event = &history.events()[3];
    assert_eq!(event.send_state, Some(DemoSendState::Pending));
    assert!(matches!(event.id.id, TimelineId::Pending(_)));
    assert_eq!(event.id.scope, scope());
    assert_eq!(event.text, "locale 🦀\noriginale");
}
