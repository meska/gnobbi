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
        drawer: None,
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
    assert_eq!(row.label(), Some("Invitata, Invito"));
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

#[test]
fn the_invitation_state_is_in_the_row_name_and_the_buttons_follow_it() {
    let account = AccountId::new(22);
    let scope = RoomScope { account_id: account, room_id: RoomId::new("!inv:fixture.invalid") };
    let mut sidebar = Sidebar::connected(account);
    sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
        scope: scope.clone(),
        name: "Invitata".into(),
        unread: 0,
        info: RoomInfo { invited: true, ..Default::default() },
    }));
    let view = |sidebar: &Sidebar| {
        let snap = snapshot(RoomNodes::default().project(sidebar));
        let tree = snap.tree();
        let row = tree.nodes.iter().find(|(id, _)| *id == snap.rooms[0].id).unwrap().1.clone();
        let labels: Vec<String> = row
            .children()
            .iter()
            .map(|id| tree.nodes.iter().find(|(n, _)| n == id).unwrap().1.label().unwrap().to_owned())
            .collect();
        (row.label().unwrap().to_owned(), labels)
    };
    sidebar.answer_invite(&scope, false);
    assert_eq!(view(&sidebar), ("Invitata, Rifiutare?".into(), vec!["Accetta".into(), "Conferma".into()]));
    sidebar.answer_invite(&scope, false);
    assert_eq!(view(&sidebar), ("Invitata, Attendi…".into(), vec![]));
    sidebar.invite_answered(&scope, false);
    assert_eq!(view(&sidebar), ("Invitata, Non riuscito".into(), vec!["Riprova".into(), "Rifiuta".into()]));
}

#[test]
fn a_room_row_name_carries_the_last_message_time_only_when_known() {
    let account = AccountId::new(22);
    let mut sidebar = Sidebar::connected(account);
    for (name, ts) in [("ConOra", Some(chrono::Utc::now().timestamp_millis() as u64)), ("SenzaOra", None)] {
        sidebar.apply_diff(RoomListDiff::Upsert(RoomSummary {
            scope: RoomScope { account_id: account, room_id: RoomId::new(format!("!{name}:fixture.invalid")) },
            name: name.into(),
            unread: 1,
            info: RoomInfo { last_event_ts: ts, ..Default::default() },
        }));
    }
    let snap = snapshot(RoomNodes::default().project(&sidebar));
    let tree = snap.tree();
    let label = |n: &str| {
        let id = snap.rooms.iter().find(|r| r.name == n).unwrap().id;
        tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1.label().map(str::to_owned)
    };
    assert!(label("ConOra").unwrap().starts_with("ConOra, ultimo messaggio alle "));
    assert_eq!(label("SenzaOra").as_deref(), Some("SenzaOra"));
}
