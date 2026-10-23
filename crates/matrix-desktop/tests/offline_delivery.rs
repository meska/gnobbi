use matrix_desktop::{DemoHistory, DemoSendState, OfflineDelivery};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn history() -> DemoHistory {
    let mut h = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(81),
            room_id: RoomId::new("!offline:test"),
        },
        0,
    );
    h.submit_text("locale caffè 🙂\nnon inviato".into());
    h
}

#[test]
fn retirement_and_replacement_cannot_rebind_the_same_transaction() {
    let mut h = history();
    let mut controls = OfflineDelivery::default();
    let old = controls.target(&h.events()[0]).unwrap();
    controls.clear();
    let current = controls.target(&h.events()[0]).unwrap();
    assert!(!controls.is_current(&old, &h));
    assert!(controls.is_current(&current, &h));
    assert_eq!(controls.apply(&old, &mut h), None);
    // Another scope can reuse the local transaction string without inheriting a source.
    let mut other = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(82),
            room_id: RoomId::new("!other:test"),
        },
        0,
    );
    other.submit_text(h.events()[0].text.clone());
    controls.retain_current(&other);
    assert!(!controls.is_current(&current, &h));
    controls.target(&h.events()[0]).unwrap();
    assert!(!controls.is_current(&current, &h));
    let mut replacement = history();
    replacement = {
        let mut r = DemoHistory::new(replacement.events()[0].id.scope.clone(), 0);
        r.submit_text("replacement".into());
        r
    };
    controls.retain_current(&replacement);
    assert_eq!(controls.apply(&current, &mut replacement), None);
    let server = DemoHistory::new(h.events()[0].id.scope.clone(), 1);
    assert!(controls.target(&server.events()[0]).is_none());
}

#[test]
fn retry_reuses_identity_without_reviving_the_old_pending_control() {
    let mut h = history();
    let original = h.events()[0].id.clone();
    let mut controls = OfflineDelivery::default();
    let pending = controls.target(&h.events()[0]).unwrap();
    controls.apply(&pending, &mut h).unwrap();
    let failed = controls.target(&h.events()[0]).unwrap();
    assert_eq!(controls.apply(&failed, &mut h), Some(0));
    assert_eq!(h.events()[0].send_state, Some(DemoSendState::Pending));
    assert_eq!(h.events()[0].id, original);
    assert_eq!(h.events().len(), 1);
    assert_eq!(controls.apply(&pending, &mut h), None);
    assert_eq!(controls.apply(&failed, &mut h), None);
    assert!(controls.target(&h.events()[0]).unwrap().instance() != pending.instance());
}

#[test]
fn captured_pending_control_fails_only_its_one_local_transaction() {
    let mut h = history();
    let id = h.events()[0].id.clone();
    let mut controls = OfflineDelivery::default();
    let target = controls.target(&h.events()[0]).unwrap();
    assert!(controls.is_current(&target, &h));
    assert_eq!(controls.apply(&target, &mut h), Some(0));
    assert_eq!(h.events().len(), 1);
    assert_eq!(h.events()[0].id, id);
    assert_eq!(h.events()[0].send_state, Some(DemoSendState::Failed));
    assert!(!h.events()[0].server_confirmed);
    assert!(!controls.is_current(&target, &h));
    assert_eq!(controls.apply(&target, &mut h), None);
}
