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

#[test]
fn saved_room_left_or_gone_never_opens_another_row() {
    let mut s = LastRoomRestore::default();
    // The list only has other rooms: nothing is opened, the caller keeps the selection empty while waiting.
    for _ in 0..10 {
        assert_eq!(s.poll(acc(1), || Some(r("!left:s")), |room| room != &r("!left:s")), None);
        assert!(s.waiting());
    }
}

#[test]
fn restored_room_removed_by_first_sync_is_reported_once_until_the_user_chooses() {
    let mut s = LastRoomRestore::default();
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), |_| true), Some(r("!a:s")));
    assert!(!s.lost(&r("!other:s")));
    assert!(s.lost(&r("!a:s")), "the opened room was removed: no row may take over");
    assert!(s.waiting(), "awaited again: a later sync may bring it back");
    assert!(!s.lost(&r("!a:s")), "only once");
    assert_eq!(s.poll(acc(1), || None, |_| true), Some(r("!a:s")));
    let mut s = LastRoomRestore::default();
    s.poll(acc(1), || Some(r("!a:s")), |_| true);
    s.cancel();
    assert!(!s.lost(&r("!a:s")), "after a manual choice the usual fallback applies");
}
