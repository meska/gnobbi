use matrix_desktop::last_room_restore::LastRoomRestore;
use matrix_domain::{AccountId, RoomId};

fn r(s: &str) -> RoomId { RoomId::new(s) }
fn acc(n: u64) -> AccountId { AccountId::new(n) }

#[test]
fn opens_when_listed_and_only_once() {
    let mut s = LastRoomRestore::default();
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), |_| true), Some(r("!a:s")));
    assert!(!s.waiting());
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), |_| true), None, "no re-arm for same account");
}

#[test]
fn waits_for_sync_then_opens() {
    let mut s = LastRoomRestore::default();
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), |_| false), None);
    assert!(s.waiting());
    assert_eq!(s.poll(acc(1), || None, |_| true), Some(r("!a:s")));
}

#[test]
fn absence_never_discards_the_saved_room() {
    let mut s = LastRoomRestore::default();
    s.poll(acc(1), || Some(r("!a:s")), |_| false);
    for _ in 0..100 {
        assert_eq!(s.poll(acc(1), || None, |_| false), None);
        assert!(s.waiting());
    }
    assert_eq!(s.poll(acc(1), || None, |_| true), Some(r("!a:s")));
}

#[test]
fn no_saved_room_does_nothing() {
    let mut s = LastRoomRestore::default();
    assert_eq!(s.poll(acc(1), || None, |_| panic!("must not open")), None);
    assert!(!s.waiting());
}

#[test]
fn account_change_rearms_with_that_accounts_room_and_drops_old_wait() {
    let mut s = LastRoomRestore::default();
    s.poll(acc(1), || Some(r("!a:s")), |_| false);
    assert_eq!(s.poll(acc(2), || Some(r("!b:s")), |room| room == &r("!b:s")), Some(r("!b:s")));
    s.poll(acc(1), || None, |_| false);
    assert!(!s.waiting(), "account 1 re-armed with its own (none) saved room");
}

#[test]
fn manual_choice_cancels() {
    let mut s = LastRoomRestore::default();
    s.poll(acc(1), || Some(r("!a:s")), |_| false);
    s.cancel();
    assert_eq!(s.poll(acc(1), || None, |_| true), None);
}
