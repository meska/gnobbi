//! «Esci» on a room row, through the same `Sidebar` entry points the shell calls: `room_left` for the
//! engine's answer (`OperationResult::RoomLeft`) and `apply_diff` for the room-list events. The engine
//! side (server membership, late sync, restart) is tests/leave_membership.rs in matrix-engine.
use matrix_desktop::Sidebar;
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary, operations::OperationError};

const ACCOUNT: AccountId = AccountId::new(61);

fn scope(id: &str) -> RoomScope {
    RoomScope { account_id: ACCOUNT, room_id: RoomId::new(id) }
}

fn upsert(sidebar: &mut Sidebar, id: &str, name: &str) {
    let info = RoomInfo { joined_members: 2, preview: Some("ciao".into()), ..Default::default() };
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary { scope: scope(id), name: name.into(), unread: 0, info })));
}

fn three_rooms() -> Sidebar {
    let mut sidebar = Sidebar::connected(ACCOUNT);
    for (id, name) in [("!a:t.invalid", "a"), ("!b:t.invalid", "b"), ("!c:t.invalid", "c")] {
        upsert(&mut sidebar, id, name);
    }
    sidebar
}

fn listed(sidebar: &Sidebar) -> Vec<&str> {
    sidebar.visible_indices().iter().map(|&i| sidebar.rooms()[i].name.as_str()).collect()
}

#[test]
fn confirmed_leave_of_the_selected_room_drops_the_row_empties_the_panel_and_retires_the_panels() {
    let mut sidebar = three_rooms();
    let b = scope("!b:t.invalid");
    assert!(sidebar.select(&b));
    let revision = sidebar.navigation_revision();
    sidebar.room_left(&b, Ok(()));
    assert!(!listed(&sidebar).contains(&"b"), "ordinary list still shows the left room");
    // Two columns: an empty panel, not the first row of the list (review I5).
    assert!(sidebar.selected().is_none());
    // Everything bound to the old revision (open panels, pending navigation) is dead.
    assert!(sidebar.navigation_revision() > revision);
    assert_eq!(sidebar.leave_error(&b), None);
}

#[test]
fn confirmed_leave_of_the_only_room_leaves_nothing_selected() {
    let mut sidebar = Sidebar::connected(ACCOUNT);
    upsert(&mut sidebar, "!a:t.invalid", "a");
    sidebar.room_left(&scope("!a:t.invalid"), Ok(()));
    assert!(sidebar.rooms().is_empty() && sidebar.selected().is_none());
}

#[test]
fn refused_leave_keeps_row_selection_and_panels_and_names_the_error() {
    for (error, text) in [
        (OperationError::Forbidden, "Not allowed by the server"),
        (OperationError::Transport, "Connection failed"),
    ] {
        let mut sidebar = three_rooms();
        let b = scope("!b:t.invalid");
        assert!(sidebar.select(&b));
        let revision = sidebar.navigation_revision();
        sidebar.room_left(&b, Err(error));
        assert_eq!(listed(&sidebar), ["a", "b", "c"], "{error:?}");
        assert_eq!(sidebar.selected().map(|r| &r.scope), Some(&b));
        assert_eq!(sidebar.navigation_revision(), revision);
        assert_eq!(sidebar.leave_error(&b), Some(text));
        // The room is not «not found»: it is still there and still joined.
        assert!(sidebar.rooms().iter().any(|r| r.scope == b && !r.info.invited));
    }
}

#[test]
fn a_room_the_sdk_no_longer_has_joined_is_not_reported_as_not_found() {
    let mut sidebar = three_rooms();
    let a = scope("!a:t.invalid");
    sidebar.room_left(&a, Err(OperationError::Unavailable));
    assert_eq!(listed(&sidebar), ["b", "c"]);
    assert_eq!(sidebar.leave_error(&a), None);
}

#[test]
fn the_sync_remove_that_follows_a_confirmed_leave_changes_nothing() {
    let mut sidebar = three_rooms();
    let b = scope("!b:t.invalid");
    assert!(sidebar.select(&b));
    sidebar.room_left(&b, Ok(()));
    let (revision, selected) = (sidebar.navigation_revision(), sidebar.selected().map(|r| r.scope.clone()));
    assert!(!sidebar.apply_diff(RoomListDiff::Remove(b.clone())));
    assert_eq!(sidebar.navigation_revision(), revision);
    assert_eq!(sidebar.selected().map(|r| r.scope.clone()), selected);
    // Other rooms keep updating normally afterwards.
    upsert(&mut sidebar, "!c:t.invalid", "c2");
    assert_eq!(listed(&sidebar).len(), 2);
}
