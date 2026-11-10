use matrix_desktop::{last_room_restore::LastRoomRestore, Sidebar};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

#[test]
fn cached_fallback_is_not_selected_while_saved_room_is_pending() {
    let account = AccountId::new(1);
    let scope = |id| RoomScope { account_id: account, room_id: RoomId::new(id) };
    let mut sidebar = Sidebar::connected(account);
    let mut restore = LastRoomRestore::default();
    let saved = scope("!saved:example.org");
    restore.arm(account, || Some(saved.room_id.clone()));
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: scope("!other:example.org"), name: "Other".into(), unread: 1,
        info: RoomInfo { preview: Some("message".into()), ..Default::default() },
    })));
    let opened = restore.poll(account, || None, |room| {
        let wanted = RoomScope { account_id: account, room_id: room.clone() };
        sidebar.is_joined(&wanted) && sidebar.select(&wanted)
    });
    if opened.is_none() && restore.waiting() { sidebar.clear_selection(); }
    assert!(restore.waiting());
    assert!(sidebar.selected().is_none());
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: saved.clone(), name: "Saved".into(), unread: 0,
        info: RoomInfo { preview: Some("message".into()), ..Default::default() },
    })));
    assert_eq!(restore.poll(account, || None, |room| {
        let wanted = RoomScope { account_id: account, room_id: room.clone() };
        sidebar.is_joined(&wanted) && sidebar.select(&wanted)
    }), Some(saved.room_id));
    assert_eq!(sidebar.selected().unwrap().name, "Saved");
}
