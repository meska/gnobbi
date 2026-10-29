#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, TreeId};
use matrix_desktop::{
    Sidebar,
    accessibility::{
        NavigationAction, NavigationSnapshot, RoomNodes,
        semantic_session::SemanticSession,
        spaces::{SpaceNodes, SpacesSnapshot, TOGGLE},
    },
};
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};

fn summary(id: &str, is_space: bool) -> RoomSummary {
    RoomSummary {
        scope: RoomScope {
            account_id: AccountId::new(17),
            room_id: RoomId::new(id),
        },
        name: id.into(),
        unread: 1,
        info: RoomInfo {
            is_space,
            ..Default::default()
        },
    }
}
fn navigation(
    sidebar: &Sidebar,
    rooms: &mut RoomNodes,
    spaces: &mut SpaceNodes,
    persistent: bool,
    open: bool,
) -> NavigationSnapshot {
    NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: Some(SpacesSnapshot {
            revision: sidebar.navigation_revision(),
            persistent,
            account: AccountId::new(17),
            open,
            active: sidebar.active_space().cloned(),
            focused: None,
            bounds: Default::default(),
            choices: spaces.rail_choices(sidebar),
        }),
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        icon_choice: None,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: sidebar.query().into(),
        search_focused: false,
        search_bounds: None,
        rooms: rooms.project(sidebar),
        history: None,
        list_focused: false,
        composer: None,
    }
}

#[test]
fn room_focus_translates_without_activating_the_conversation() {
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    let mut rooms = RoomNodes::default();
    let mut spaces = SpaceNodes::default();
    let snapshot = navigation(&sidebar, &mut rooms, &mut spaces, true, false);
    let selected = sidebar.selected().unwrap().scope.clone();
    let target = snapshot.rooms[1].clone();
    let session = SemanticSession::new(snapshot);
    let (id, node) = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, node)| node.label() == Some(target.name.as_str()))
        .unwrap();
    assert!(node.supports_action(Action::Focus));
    let intent = session.action(&ActionRequest {
        action: Action::Focus,
        target_node: id,
        target_tree: TreeId::ROOT,
        data: None,
    });
    let Some(NavigationAction::FocusRoom(intent)) = intent else {
        panic!("room Focus must translate independently of Click");
    };
    assert!(rooms.focus_action(&intent, &sidebar));
    let bounds = accesskit::Rect::new(72., 100., 400., 168.);
    assert!(rooms.set_bounds(&intent, &sidebar, bounds));
    assert_eq!(selected, sidebar.selected().unwrap().scope);
    let mut focused = navigation(&sidebar, &mut rooms, &mut spaces, true, false);
    focused.list_focused = true;
    assert_eq!(focused.tree().focus, target.id);
    assert_eq!(
        focused
            .tree()
            .nodes
            .iter()
            .find(|(id, _)| *id == target.id)
            .unwrap()
            .1
            .bounds(),
        Some(bounds)
    );
    assert_eq!(rooms.focused(&sidebar), Some(&target.scope));
    sidebar.set_query("no matches");
    sidebar.set_query("");
    assert!(!rooms.focus_action(&intent, &sidebar));
    assert!(!rooms.set_bounds(&intent, &sidebar, bounds));
    assert!(rooms.focused(&sidebar).is_none());
}

#[test]
fn room_keyboard_activation_uses_the_focused_not_selected_conversation() {
    let sidebar = Sidebar::demo(AccountId::new(17));
    let mut rooms = RoomNodes::default();
    let target = rooms.project(&sidebar)[1].clone();
    let intent = matrix_desktop::accessibility::RoomIntent {
        id: target.id,
        scope: target.scope.clone(),
        revision: target.revision,
    };
    assert!(rooms.focus_action(&intent, &sidebar));
    for key in ["enter", "space"] {
        let activation = rooms
            .activation_key(&sidebar, key)
            .expect("focused room keyboard activation");
        assert_eq!(activation.scope, target.scope);
        assert!(rooms.resolve_action(&activation, &sidebar).is_some());
    }
    assert!(rooms.activation_key(&sidebar, "tab").is_none());
    assert_ne!(sidebar.selected().unwrap().scope, target.scope);
}

#[test]
fn queued_room_focus_rejects_removed_replaced_hidden_foreign_sources() {
    for case in [
        "remove-return",
        "space-return",
        "unread-return",
        "search-return",
        "account",
        "node",
        "scope",
    ] {
        let mut sidebar = Sidebar::connected(AccountId::new(17));
        let room = summary("!room:test", false);
        sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
        sidebar.apply_diff(RoomListDiff::Upsert(summary("!other:test", false)));
        sidebar.set_navigation_visibility(true, true, false);
        if case == "unread-return" {
            sidebar.set_unread_only(true);
        }
        let selected = sidebar.selected().unwrap().scope.clone();
        let mut rooms = RoomNodes::default();
        let mut spaces = SpaceNodes::default();
        let mut session =
            SemanticSession::new(navigation(&sidebar, &mut rooms, &mut spaces, true, false));
        let id = session
            .tree()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("!room:test"))
            .unwrap()
            .0;
        let request = ActionRequest {
            action: Action::Focus,
            target_node: id,
            target_tree: TreeId::ROOT,
            data: None,
        };
        let Some(NavigationAction::FocusRoom(mut intent)) = session.action(&request) else {
            panic!("Focus translation");
        };
        match case {
            "remove-return" => {
                sidebar.apply_diff(RoomListDiff::Remove(room.scope.clone()));
                sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
            }
            "space-return" => {
                let mut changed = room.clone();
                changed.info.is_space = true;
                sidebar.apply_diff(RoomListDiff::Upsert(changed));
                sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
            }
            "unread-return" => {
                let mut changed = room.clone();
                changed.unread = 0;
                sidebar.apply_diff(RoomListDiff::Upsert(changed));
                sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
            }
            "search-return" => {
                sidebar.set_query("other");
                sidebar.set_query("");
            }
            "account" => intent.scope.account_id = AccountId::new(18),
            "node" => intent.id = accesskit::NodeId(99999),
            "scope" => intent.scope.room_id = RoomId::new("!other:test"),
            _ => unreachable!(),
        }
        assert!(!rooms.focus_action(&intent, &sidebar), "{case}");
        if !matches!(case, "account" | "node" | "scope") {
            session.update(navigation(&sidebar, &mut rooms, &mut spaces, true, false));
            assert!(session.action(&request).is_none(), "retired path {case}");
        }
        // Removal legitimately picks a surviving conversation; all other Focus attempts leave it alone.
        if case != "remove-return" {
            assert_eq!(selected, sidebar.selected().unwrap().scope);
        }
        let current = rooms
            .project(&sidebar)
            .into_iter()
            .find(|r| r.scope == room.scope)
            .unwrap();
        let fresh = matrix_desktop::accessibility::RoomIntent {
            id: current.id,
            scope: current.scope,
            revision: current.revision,
        };
        assert!(rooms.focus_action(&fresh, &sidebar));
        assert!(!rooms.focus_action(&intent, &sidebar));
        assert_eq!(rooms.focused(&sidebar), Some(&room.scope));
    }
}

#[test]
fn retired_space_and_room_paths_cannot_reacquire_the_returned_navigation_visit() {
    // No intermediate snapshot: queued callbacks and native-path translation must both reject.
    for (persistent, open, away) in [
        (false, true, (false, true, false)), // menu close/reopen
        (false, true, (false, false, true)), // compact chat away/back
        (true, true, (false, true, true)),   // rail/menu resize
        (true, true, (true, false, false)),  // covering panel
    ] {
        let mut sidebar = Sidebar::connected(AccountId::new(17));
        sidebar.apply_diff(RoomListDiff::Upsert(summary("!room:test", false)));
        sidebar.apply_diff(RoomListDiff::Upsert(summary("!space:test", true)));
        sidebar.set_navigation_visibility(persistent, true, open);
        let mut rooms = RoomNodes::default();
        let mut spaces = SpaceNodes::default();
        let initial = navigation(&sidebar, &mut rooms, &mut spaces, persistent, open);
        let mut session = SemanticSession::new(initial);
        let mut requests = vec![];
        let mut intents = vec![];
        for (id, node) in &session.tree().nodes {
            if node.label() == Some("!room:test")
                || node.role() == accesskit::Role::RadioButton
                || node.label() == Some("Spazi: Tutte le conversazioni · 2 non letti")
            {
                for action in [Action::Focus, Action::Click] {
                    let request = ActionRequest {
                        action,
                        target_node: *id,
                        target_tree: TreeId::ROOT,
                        data: None,
                    };
                    if let Some(intent) = session.action(&request) {
                        requests.push(request);
                        intents.push(intent);
                    }
                }
            }
        }
        assert!(
            intents
                .iter()
                .any(|i| matches!(i, NavigationAction::ActivateRoom(_)))
        );
        assert!(
            intents
                .iter()
                .any(|i| matches!(i, NavigationAction::Spaces(_)))
        );
        sidebar.set_navigation_visibility(away.0, away.1, away.2);
        sidebar.set_navigation_visibility(persistent, true, open);
        let returned = navigation(&sidebar, &mut rooms, &mut spaces, persistent, open);
        for intent in &intents {
            match intent {
                NavigationAction::Spaces(action) => {
                    assert!(!returned.spaces.as_ref().unwrap().allows(action))
                }
                NavigationAction::ActivateRoom(action) | NavigationAction::FocusRoom(action) => {
                    assert!(rooms.resolve_action(action, &sidebar).is_none())
                }
                _ => unreachable!(),
            }
        }
        session.update(returned.clone());
        for request in &requests {
            assert!(session.action(request).is_none());
        }
        let room = &returned.rooms[0];
        assert!(matches!(
            returned.action(&ActionRequest {
                action: Action::Click,
                target_node: room.id,
                target_tree: TreeId::ROOT,
                data: None
            }),
            Some(NavigationAction::ActivateRoom(_))
        ));
        let revision = sidebar.navigation_revision();
        sidebar.set_navigation_visibility(persistent, true, open);
        assert_eq!(revision, sidebar.navigation_revision());
        if persistent {
            sidebar.set_navigation_visibility(true, true, !open);
            assert_eq!(
                revision,
                sidebar.navigation_revision(),
                "a hidden compact menu is not a rail transition"
            );
        }
        assert_eq!(
            sidebar.selected().unwrap().scope.room_id.as_str(),
            "!room:test"
        );
        assert_eq!(sidebar.active_space(), None);
        if !persistent {
            assert!(
                returned
                    .spaces
                    .unwrap()
                    .nodes()
                    .iter()
                    .any(|(id, _)| *id == TOGGLE)
            );
        }
    }
}

#[test]
fn upsert_search_and_direct_child_eligibility_retire_without_changing_hidden_chat() {
    let mut sidebar = Sidebar::connected(AccountId::new(17));
    let room = summary("!room:test", false);
    let mut space = summary("!space:test", true);
    space.info.children = vec![room.scope.room_id.clone()];
    sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
    sidebar.apply_diff(RoomListDiff::Upsert(space.clone()));
    sidebar.set_space(Some(space.scope.clone()));
    let selected = sidebar.selected().unwrap().scope.clone();
    let before = sidebar.navigation_revision();
    space.info.children.clear();
    sidebar.apply_diff(RoomListDiff::Upsert(space.clone()));
    space.info.children.push(room.scope.room_id.clone());
    sidebar.apply_diff(RoomListDiff::Upsert(space));
    assert_ne!(before, sidebar.navigation_revision());
    assert_eq!(selected, sidebar.selected().unwrap().scope);
    sidebar.set_query("!room:");
    let before = sidebar.navigation_revision();
    let mut renamed = room.clone();
    renamed.name = "hidden".into();
    sidebar.apply_diff(RoomListDiff::Upsert(renamed));
    assert!(sidebar.visible_indices().is_empty());
    sidebar.apply_diff(RoomListDiff::Upsert(room.clone()));
    assert_ne!(before, sidebar.navigation_revision());
    assert_eq!(selected, sidebar.selected().unwrap().scope);
    let stable = sidebar.navigation_revision();
    let mut ordinary = room;
    ordinary.info.topic = Some("updated read-only details".into());
    sidebar.apply_diff(RoomListDiff::Upsert(ordinary.clone()));
    assert_eq!(stable, sidebar.navigation_revision());
    ordinary.scope.account_id = AccountId::new(18);
    assert!(!sidebar.apply_diff(RoomListDiff::Upsert(ordinary)));
    assert_eq!(stable, sidebar.navigation_revision());
}
