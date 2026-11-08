use matrix_desktop::last_room_restore::{LastRoomRestore, RESTORE_TIMEOUT};
use matrix_domain::{AccountId, RoomId};
use std::time::{Duration, Instant};

fn r(s: &str) -> RoomId { RoomId::new(s) }
fn acc(n: u64) -> AccountId { AccountId::new(n) }

#[test]
fn opens_when_listed_and_only_once() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), t, |_| true), Some(r("!a:s")));
    assert!(!s.waiting());
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), t, |_| true), None, "no re-arm for same account");
}

#[test]
fn waits_for_sync_then_opens() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    assert_eq!(s.poll(acc(1), || Some(r("!a:s")), t, |_| false), None);
    assert!(s.waiting());
    let late = t + RESTORE_TIMEOUT - Duration::from_millis(1);
    assert_eq!(s.poll(acc(1), || None, late, |_| true), Some(r("!a:s")));
}

#[test]
fn timeout_gives_up_without_opening() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    s.poll(acc(1), || Some(r("!a:s")), t, |_| false);
    assert_eq!(s.poll(acc(1), || None, t + RESTORE_TIMEOUT, |_| false), None);
    assert!(!s.waiting());
    assert_eq!(s.poll(acc(1), || None, t + RESTORE_TIMEOUT, |_| true), None, "never opens after giving up");
}

#[test]
fn no_saved_room_does_nothing() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    assert_eq!(s.poll(acc(1), || None, t, |_| panic!("must not open")), None);
    assert!(!s.waiting());
}

#[test]
fn account_change_rearms_with_that_accounts_room_and_drops_old_wait() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    s.poll(acc(1), || Some(r("!a:s")), t, |_| false);
    assert_eq!(s.poll(acc(2), || Some(r("!b:s")), t, |room| room == &r("!b:s")), Some(r("!b:s")));
    s.poll(acc(1), || None, t, |_| false);
    assert!(!s.waiting(), "account 1 re-armed with its own (none) saved room");
}

#[test]
fn manual_choice_cancels() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    s.poll(acc(1), || Some(r("!a:s")), t, |_| false);
    s.cancel();
    assert_eq!(s.poll(acc(1), || None, t, |_| true), None);
}

#[test]
fn timeout_counts_from_first_list_not_from_arm() {
    let (mut s, t) = (LastRoomRestore::default(), Instant::now());
    s.arm(acc(1), || Some(r("!a:s")), t);
    let listed = t + RESTORE_TIMEOUT - Duration::from_millis(1);
    s.list_arrived(listed);
    let later = listed + RESTORE_TIMEOUT - Duration::from_millis(1);
    assert_eq!(s.poll(acc(1), || None, later, |_| false), None);
    assert!(s.waiting(), "still within 5 s of the list");
    assert_eq!(s.poll(acc(1), || None, later, |_| true), Some(r("!a:s")));
}
