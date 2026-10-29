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
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
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
fn an_invitation_row_exposes_accept_and_reject_buttons_that_answer_it() {
    let account = AccountId::new(21);
    let scope = RoomScope { account_id: account, room_id: RoomId::new("!inv:fixture.invalid") };
    let mut sidebar = Sidebar::connected(account);
    sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
        scope: scope.clone(),
        name: "Invitata".into(),
        unread: 0,
        info: RoomInfo { invited: true, ..Default::default() },
    }));
    let snap = snapshot(RoomNodes::default().project(&sidebar));
    let tree = snap.tree();
    let row = &tree.nodes.iter().find(|(id, _)| *id == snap.rooms[0].id).unwrap().1;
    assert_eq!(row.label(), Some("Invitata"));
    assert_eq!(row.description(), Some("Invito"));
    let buttons: Vec<_> = row
        .children()
        .iter()
        .map(|id| &tree.nodes.iter().find(|(n, _)| n == id).unwrap().1)
        .collect();
    assert_eq!(buttons.iter().map(|b| b.role()).collect::<Vec<_>>(), [Role::Button; 2]);
    assert_eq!(buttons.iter().map(|b| b.label().unwrap()).collect::<Vec<_>>(), ["Accetta", "Rifiuta"]);
    for (id, accept) in row.children().iter().zip([true, false]) {
        let request = ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: *id,
            data: None::<ActionData>,
        };
        match snap.action(&request) {
            Some(NavigationAction::AnswerInvite { scope: s, accept: a }) => {
                assert_eq!((s, a), (scope.clone(), accept))
            }
            _ => panic!("button does not answer the invitation"),
        }
    }
}
