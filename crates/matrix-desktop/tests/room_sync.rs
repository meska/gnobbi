use matrix_desktop::Sidebar;
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomListDiff, RoomSummary};

fn scope(id: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(18),
        room_id: RoomId::new(id),
    }
}
fn upsert(id: &str, name: &str, unread: u64) -> RoomListDiff {
    RoomListDiff::Upsert(RoomSummary {
        scope: scope(id),
        name: name.to_owned(),
        unread,
        info: Default::default(),
    })
}

#[test]
fn space_filter_uses_direct_joined_children_without_changing_selection() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    sidebar.apply_diff(upsert("!one:test", "Alpha", 2));
    sidebar.apply_diff(upsert("!other:test", "Outside", 0));
    let mut space = match upsert("!space:test", "Workspace", 0) {
        RoomListDiff::Upsert(r) => r,
        _ => unreachable!(),
    };
    space.info.is_space = true;
    space.info.children = vec![RoomId::new("!one:test"), RoomId::new("!not-joined:test")];
    sidebar.apply_diff(RoomListDiff::Upsert(space));
    assert_eq!(sidebar.spaces().count(), 1);
    assert!(sidebar.select(&scope("!other:test")));
    assert!(sidebar.set_space(Some(scope("!space:test"))));
    assert_eq!(sidebar.visible_indices(), &[0, 2]);
    assert_eq!(sidebar.selected().unwrap().scope, scope("!other:test"));
    sidebar.set_query("alpha");
    assert_eq!(sidebar.visible_indices(), &[0]);
    sidebar.set_unread_only(true);
    assert_eq!(sidebar.visible_indices(), &[0]);
    assert!(!sidebar.set_space(Some(scope("!other:test"))));
    assert_eq!(sidebar.active_space(), Some(&scope("!space:test")));
    sidebar.set_query("");
    sidebar.set_unread_only(false);
    assert!(sidebar.set_space(None));
    assert_eq!(sidebar.visible_indices(), &[0, 1, 2]);
    assert_eq!(sidebar.rooms()[2].info.children.len(), 2);
}

#[test]
fn space_removal_clears_filter_without_accepting_foreign_account_diffs() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    let mut space = match upsert("!space:test", "Workspace", 0) {
        RoomListDiff::Upsert(r) => r,
        _ => unreachable!(),
    };
    space.info.is_space = true;
    sidebar.apply_diff(RoomListDiff::Upsert(space.clone()));
    sidebar.apply_diff(upsert("!other:test", "Outside", 0));
    assert!(sidebar.set_space(Some(scope("!space:test"))));
    space.scope.account_id = AccountId::new(99);
    assert!(!sidebar.apply_diff(RoomListDiff::Upsert(space.clone())));
    assert!(!sidebar.set_space(Some(space.scope)));
    sidebar.apply_diff(RoomListDiff::Remove(scope("!space:test")));
    assert!(sidebar.active_space().is_none());
    assert_eq!(sidebar.visible_indices(), &[0]);
}

#[test]
fn removing_an_earlier_row_preserves_selection_by_room_identity() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    sidebar.apply_diff(upsert("!first:fixture.invalid", "Primo", 0));
    sidebar.apply_diff(upsert("!second:fixture.invalid", "Secondo", 0));
    assert!(sidebar.select(&scope("!second:fixture.invalid")));
    assert!(sidebar.apply_diff(RoomListDiff::Remove(scope("!first:fixture.invalid"))));
    assert_eq!(
        sidebar.selected().unwrap().scope,
        scope("!second:fixture.invalid")
    );
    assert_eq!(sidebar.rooms().len(), 1);
    assert!(sidebar.apply_diff(RoomListDiff::Remove(scope("!second:fixture.invalid"))));
    assert!(sidebar.selected().is_none());
    assert!(sidebar.visible_indices().is_empty());
}

#[test]
fn unread_filter_reacts_to_live_diffs_without_changing_hidden_selection() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    sidebar.apply_diff(upsert("!one:fixture.invalid", "Alpha", 0));
    sidebar.apply_diff(upsert("!two:fixture.invalid", "Beta", 3));
    sidebar.set_unread_only(true);
    assert_eq!(sidebar.visible_indices(), &[1]);
    assert_eq!(
        sidebar.selected().unwrap().scope,
        scope("!one:fixture.invalid")
    );
    sidebar.set_query("alpha");
    assert!(sidebar.visible_indices().is_empty());
    sidebar.apply_diff(upsert("!one:fixture.invalid", "Alpha", 2));
    assert_eq!(sidebar.visible_indices(), &[0]);
    sidebar.apply_diff(upsert("!one:fixture.invalid", "Renamed", 2));
    assert!(sidebar.visible_indices().is_empty());
    assert_eq!(sidebar.selected().unwrap().name, "Renamed");
    assert_eq!(sidebar.rooms().len(), 2);
    sidebar.set_query("");
    sidebar.set_unread_only(false);
    // Plain list: idle rooms by name, so Beta (index 1) precedes Renamed (index 0).
    assert_eq!(sidebar.visible_indices(), &[1, 0]);
}

#[test]
fn public_peer_marker_survives_scoped_client_route_without_becoming_unread_count() {
    use matrix_domain::EventId;
    use matrix_engine::receipts::PublicReadReceipt;
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    let RoomListDiff::Upsert(mut summary) = upsert("!one:test", "Room", 3) else {
        unreachable!()
    };
    summary.info.public_read_receipts = vec![PublicReadReceipt {
        event_id: EventId::new("$exact"),
        readers: 1,
    }];
    assert!(sidebar.apply_diff(RoomListDiff::Upsert(summary.clone())));
    let room = sidebar.selected().unwrap();
    assert_eq!(room.unread, 3);
    assert_eq!(
        room.info.public_read_receipts[0].event_id.as_str(),
        "$exact"
    );
    assert_eq!(room.info.public_read_receipts[0].readers, 1);
    summary.scope.account_id = AccountId::new(99);
    summary.info.public_read_receipts.clear();
    assert!(!sidebar.apply_diff(RoomListDiff::Upsert(summary)));
    assert_eq!(
        sidebar.selected().unwrap().info.public_read_receipts.len(),
        1
    );
    sidebar.apply_diff(RoomListDiff::Remove(scope("!one:test")));
    sidebar.apply_diff(upsert("!one:test", "Rejoined", 0));
    assert!(
        sidebar
            .selected()
            .unwrap()
            .info
            .public_read_receipts
            .is_empty()
    );
}

#[test]
fn upsert_retires_navigation_when_unread_eligibility_leaves_and_returns_unobserved() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    sidebar.apply_diff(upsert("!one:test", "Alpha", 2));
    sidebar.set_unread_only(true);
    let before = sidebar.navigation_revision();
    sidebar.apply_diff(upsert("!one:test", "Alpha", 0));
    sidebar.apply_diff(upsert("!one:test", "Alpha", 2));
    assert_ne!(sidebar.navigation_revision(), before);
    assert_eq!(sidebar.visible_indices(), &[0]);
    assert_eq!(sidebar.selected().unwrap().scope, scope("!one:test"));
    let current = sidebar.navigation_revision();
    sidebar.apply_diff(upsert("!one:test", "Alpha renamed", 3));
    assert_eq!(
        sidebar.navigation_revision(),
        current,
        "ordinary label/count updates must retain unchanged live actions"
    );
}

#[test]
fn space_reclassification_retires_navigation_without_an_intermediate_projection() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    let RoomListDiff::Upsert(mut summary) = upsert("!space:test", "Workspace", 0) else {
        unreachable!()
    };
    summary.info.is_space = true;
    sidebar.apply_diff(RoomListDiff::Upsert(summary.clone()));
    let before = sidebar.navigation_revision();
    summary.info.is_space = false;
    sidebar.apply_diff(RoomListDiff::Upsert(summary.clone()));
    summary.info.is_space = true;
    sidebar.apply_diff(RoomListDiff::Upsert(summary));
    assert_ne!(sidebar.navigation_revision(), before);
    assert_eq!(sidebar.spaces().count(), 1);
}

#[test]
fn connected_sidebar_projects_server_room_without_demo_rows() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    assert!(sidebar.rooms().is_empty());
    assert!(sidebar.selected().is_none());
    assert!(sidebar.apply_diff(upsert("!one:fixture.invalid", "Laboratorio", 3)));
    assert_eq!(sidebar.rooms().len(), 1);
    let room = sidebar.selected().unwrap();
    assert_eq!(room.scope, scope("!one:fixture.invalid"));
    assert_eq!(room.name, "Laboratorio");
    assert_eq!(room.unread, 3);
    assert_eq!(room.preview, "");
    assert_eq!(sidebar.visible_indices(), &[0]);
}

fn upsert_info(id: &str, unread: u64, members: u64, preview: Option<&str>) -> RoomListDiff {
    RoomListDiff::Upsert(RoomSummary {
        scope: scope(id),
        name: id.to_owned(),
        unread,
        info: matrix_engine::RoomInfo {
            joined_members: members,
            preview: preview.map(str::to_owned),
            ..Default::default()
        },
    })
}

#[test]
fn rooms_with_only_me_and_no_messages_are_hidden_until_shown() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    sidebar.apply_diff(upsert_info("!empty:test", 0, 1, None));
    sidebar.apply_diff(upsert_info("!talk:test", 0, 1, Some("ciao")));
    sidebar.apply_diff(upsert_info("!unread:test", 3, 1, None));
    sidebar.apply_diff(upsert_info("!peer:test", 0, 2, None));
    sidebar.apply_diff(upsert_info("!unknown:test", 0, 0, None));
    let names = |s: &Sidebar| {
        s.visible_indices()
            .iter()
            .map(|&i| s.rooms()[i].name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&sidebar),
        ["!peer:test", "!talk:test", "!unknown:test", "!unread:test"]
    );
    assert_eq!(sidebar.hidden_empty_count(), 1);
    sidebar.set_show_empty(true);
    assert_eq!(sidebar.visible_indices().len(), 5);
    assert_eq!(sidebar.hidden_empty_count(), 1);
    // a deleted last message leaves the room empty again
    sidebar.set_show_empty(false);
    sidebar.apply_diff(upsert_info("!talk:test", 0, 1, None));
    assert_eq!(sidebar.hidden_empty_count(), 2);
    assert_eq!(sidebar.visible_indices().len(), 3);
}
