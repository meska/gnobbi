#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use matrix_desktop::{
    Sidebar,
    accessibility::{NavigationAction, NavigationSnapshot, RoomNodes},
};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

fn snapshot(rooms: Vec<matrix_desktop::accessibility::SemanticRoom>) -> NavigationSnapshot {
    NavigationSnapshot {
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
        new_chat: Default::default(),
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
}

#[test]
fn an_open_room_menu_exposes_an_exit_button_that_asks_to_leave() {
    let account = AccountId::new(22);
    let scope = RoomScope { account_id: account, room_id: RoomId::new("!room:fixture.invalid") };
    let mut sidebar = Sidebar::connected(account);
    sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
        scope: scope.clone(),
        name: "Stanza".into(),
        unread: 0,
        info: RoomInfo::default(),
    }));
    let mut rooms = RoomNodes::default().project(&sidebar);
    let button_of = |snap: &NavigationSnapshot| {
        let tree = snap.tree();
        let row = &tree.nodes.iter().find(|(id, _)| *id == snap.rooms[0].id).unwrap().1;
        row.children().first().map(|id| (*id, tree.nodes.iter().find(|(n, _)| n == id).unwrap().1.clone()))
    };
    assert!(button_of(&snapshot(rooms.clone())).is_none(), "no button while the menu is closed");
    for (confirming, label) in [(false, "Esci"), (true, "Uscire?")] {
        rooms[0].leave = Some(confirming);
        let snap = snapshot(rooms.clone());
        let (id, button) = button_of(&snap).unwrap();
        assert_eq!((button.role(), button.label()), (Role::Button, Some(label)));
        let request = ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: id, data: None::<ActionData> };
        assert!(matches!(snap.action(&request), Some(NavigationAction::LeaveRoom(s)) if s == scope));
    }
}
