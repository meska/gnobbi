use matrix_desktop::Sidebar;
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

fn scope(id: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(38),
        room_id: RoomId::new(id),
    }
}
fn add(sidebar: &mut Sidebar, id: &str, unread: u64, info: RoomInfo) {
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: scope(id),
        name: id.into(),
        unread,
        info,
    })));
}

#[test]
fn rail_counts_direct_joined_rooms_once_without_search_bias() {
    let mut sidebar = Sidebar::connected(AccountId::new(38));
    add(&mut sidebar, "!a:test", 4, RoomInfo::default());
    add(&mut sidebar, "!b:test", 7, RoomInfo::default());
    add(
        &mut sidebar,
        "!space:test",
        2,
        RoomInfo {
            is_space: true,
            children: vec![
                RoomId::new("!a:test"),
                RoomId::new("!a:test"),
                RoomId::new("!absent:test"),
            ],
            ..Default::default()
        },
    );
    sidebar.set_query("no result");
    let entries = sidebar.navigation_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].scope, None);
    assert_eq!(entries[0].name, "Tutte");
    assert_eq!(entries[0].unread, 13);
    assert!(entries[0].selected);
    assert_eq!(entries[1].scope, Some(scope("!space:test")));
    assert_eq!(entries[1].unread, 6);
    assert!(!entries[1].selected);
    assert!(sidebar.set_space(entries[1].scope.clone()));
    assert!(sidebar.navigation_entries()[1].selected);
    assert_eq!(sidebar.selected().unwrap().scope, scope("!a:test"));
    assert!(sidebar.visible_indices().is_empty());
}
