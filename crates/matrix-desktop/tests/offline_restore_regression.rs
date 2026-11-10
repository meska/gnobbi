use matrix_desktop::last_room_restore::LastRoomRestore;
use matrix_domain::{AccountId, RoomId};

#[test]
fn saved_room_remains_pending_without_live_list() {
    let mut restore = LastRoomRestore::default();
    let account = AccountId::new(1);
    let room = RoomId::new("!saved:example.org");
    restore.arm(account, || Some(room.clone()));
    restore.poll(account, || None, |_| false);
    assert!(restore.waiting(), "offline time must not expire the saved room");
    assert_eq!(restore.poll(account, || None, |_| true), Some(room));
}
