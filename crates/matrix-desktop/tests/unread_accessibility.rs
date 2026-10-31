#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use matrix_desktop::{
    Sidebar,
    accessibility::{NavigationSnapshot, RoomNodes},
};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomListDiff, RoomSummary};

#[test]
fn unread_description_updates_without_renaming_or_reidentifying_room() {
    let mut sidebar = Sidebar::connected(AccountId::new(18));
    let mut nodes = RoomNodes::default();
    let mut stable = None;
    for (count, expected) in [
        (0, "Nessun messaggio non letto"),
        (1, "1 messaggio non letto"),
        (42, "42 messaggi non letti"),
        (0, "Nessun messaggio non letto"),
    ] {
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: RoomScope {
                account_id: AccountId::new(18),
                room_id: RoomId::new("!unread:fixture.invalid"),
            },
            name: "Stanza esatta".into(),
            unread: count,
            info: Default::default(),
        }));
        let rooms = nodes.project(&sidebar);
        let id = rooms[0].id;
        assert_eq!(*stable.get_or_insert(id), id);
        let tree = NavigationSnapshot {
            title: "Gnobbi".into(),
            #[cfg(all(feature = "live", unix))]
            signin: None,
            openers: vec![],
            back: None,
            scrollbars: vec![],
            operation_panel: None,
            spaces: None,
            navigation_revision: 0,
            icon_choice: None,
            unread_filter: None,
            new_room: None,
            devices: Default::default(),
            settings: None,
            details: Default::default(),
            notifications: Default::default(),
            dark: false,
            visible: true,
            query: String::new(),
            search_focused: false,
            search_bounds: None,
            rooms,
            history: None,
            list_focused: true,
            composer: None,
        }
        .tree();
        let node = &tree
            .nodes
            .iter()
            .find(|(node_id, _)| *node_id == id)
            .unwrap()
            .1;
        assert_eq!(node.label(), Some("Stanza esatta"));
        assert_eq!(node.description(), Some(expected));
        assert_eq!(tree.focus, id);
        assert_eq!(node.is_selected(), Some(true));
    }
}
