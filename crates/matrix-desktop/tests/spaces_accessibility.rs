#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, NodeId, Role};
use matrix_desktop::accessibility::spaces::{SpaceChoice, SpacesSnapshot, TOGGLE};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn scope(account: u64, room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    }
}
fn snapshot() -> SpacesSnapshot {
    SpacesSnapshot {
        revision: 0,
        persistent: false,
        account: AccountId::new(17),
        open: true,
        active: Some(scope(17, "!space:test")),
        focused: Some(TOGGLE),
        bounds: Default::default(),
        choices: vec![
            SpaceChoice {
                id: NodeId(61),
                scope: None,
                name: "Tutte le stanze".into(),
            },
            SpaceChoice {
                id: NodeId(1 << 57),
                scope: Some(scope(17, "!space:test")),
                name: "Spazio progetto".into(),
            },
        ],
    }
}
#[test]
fn rail_always_exports_all_with_unread_count_and_no_invented_space() {
    use matrix_desktop::{
        Sidebar,
        accessibility::spaces::{ALL, SpaceNodes},
    };
    let sidebar = Sidebar::demo(AccountId::new(17));
    let choices = SpaceNodes::default().rail_choices(&sidebar);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].id, ALL);
    assert_eq!(choices[0].scope, None);
    assert_eq!(choices[0].name, "Tutte le conversazioni · 0 non letti");
}

#[test]
fn persistent_rail_exports_choices_without_a_hidden_toggle() {
    let mut view = snapshot();
    view.persistent = true;
    view.focused = Some(NodeId(61));
    let nodes = view.nodes();
    assert_eq!(nodes.len(), 3);
    assert!(!nodes.iter().any(|(id, _)| *id == TOGGLE));
    assert_eq!(nodes[1].1.role(), Role::RadioButton);
    // «Modifica» is the rail's last Tab stop, exported as a focus-only button.
    assert_eq!(nodes[2].0, matrix_desktop::accessibility::spaces::EDIT);
    assert_eq!(nodes[2].1.role(), Role::Button);
    assert_eq!(view.next_focus(true), None);
    assert_eq!(view.next_focus(false), Some(NodeId(1 << 57)));
    let request = accesskit::ActionRequest {
        action: Action::Click,
        target_node: TOGGLE,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    assert!(view.action(&request).is_none());
}

#[test]
fn space_choice_ids_never_alias_sas_flow_handles() {
    use matrix_desktop::{
        Sidebar,
        accessibility::{
            devices::{DeviceControl, DeviceNodes, DeviceOverview},
            spaces::SpaceNodes,
        },
    };
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
    let mut sidebar = Sidebar::connected(AccountId::new(17));
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(RoomSummary {
        scope: scope(17, "!space:test"),
        name: "Space".into(),
        unread: 0,
        info: RoomInfo {
            is_space: true,
            ..Default::default()
        },
    })));
    let choices = SpaceNodes::default().choices(&sidebar);
    let account = AccountId::new(17);
    let panel = DeviceNodes::default().scope_panel(DeviceOverview::project(
        account,
        &matrix_desktop::DeviceVerification::new(account),
        DeviceControl::Close,
        [None; 6],
    ));
    for control in [
        DeviceControl::Confirm,
        DeviceControl::Mismatch,
        DeviceControl::Cancel,
        DeviceControl::Comparison,
    ] {
        assert_ne!(choices[1].id, panel.control_id(control));
    }
    assert!(!(1 << 55..1 << 57).contains(&choices[1].id.0));
}

#[test]
fn space_focus_order_has_both_exits_and_skips_closed_choices() {
    let mut view = snapshot();
    assert_eq!(view.next_focus(false), Some(NodeId(61)));
    assert_eq!(view.next_focus(true), None);
    view.focused = Some(NodeId(61));
    assert_eq!(view.next_focus(false), Some(NodeId(1 << 57)));
    assert_eq!(view.next_focus(true), Some(TOGGLE));
    view.focused = Some(NodeId(1 << 57));
    assert_eq!(view.next_focus(false), None);
    view.open = false;
    view.focused = Some(TOGGLE);
    assert_eq!(view.next_focus(false), None);
}

#[test]
fn spaces_are_excluded_when_sidebar_hidden_or_sheet_covers_it() {
    use matrix_desktop::accessibility::{
        NavigationAction, NavigationSnapshot,
        notifications::{NotificationSnapshot, NotificationView},
    };
    let mut view = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: Some(snapshot()),
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
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    };
    let request = accesskit::ActionRequest {
        action: Action::Click,
        target_node: TOGGLE,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    let tree = view.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Gnobbi"));
    assert_eq!(tree.focus, TOGGLE);
    assert!(matches!(
        view.action(&request),
        Some(NavigationAction::Spaces(_))
    ));
    view.visible = false;
    assert!(!view.tree().nodes.iter().any(|(id, _)| *id == TOGGLE));
    assert!(view.action(&request).is_none());
    view.visible = true;
    view.notifications = NotificationView::Open(Box::new(NotificationSnapshot {
        bounds: [None; 4],
        focus: Default::default(),
        enabled: false,
        preview: false,
        selected: None,
        room_name: String::new(),
        service_failed: false,
    }));
    assert!(!view.tree().nodes.iter().any(|(id, _)| *id == TOGGLE));
    assert!(view.action(&request).is_none());
}

#[test]
fn joined_space_choices_have_scoped_stable_nonrecycled_ids() {
    use matrix_desktop::{
        Sidebar,
        accessibility::spaces::{ALL, SpaceNodes},
    };
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
    let room = |account, id: &str, name: &str, is_space| RoomSummary {
        scope: scope(account, id),
        name: name.into(),
        unread: 0,
        info: RoomInfo {
            is_space,
            ..Default::default()
        },
    };
    let mut sidebar = Sidebar::connected(AccountId::new(17));
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room(
        17,
        "!ordinary:test",
        "Not a Space",
        false,
    ))));
    let mut nodes = SpaceNodes::default();
    assert!(nodes.choices(&sidebar).is_empty());
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room(
        17,
        "!space:test",
        "Project",
        true,
    ))));
    let choices = nodes.choices(&sidebar);
    assert_eq!(choices.len(), 2);
    assert_eq!(choices[0].id, ALL);
    let old_id = choices[1].id;
    assert!(old_id.0 >= (1 << 57) && old_id.0 < (1 << 58));
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room(
        17,
        "!space:test",
        "Renamed",
        true,
    ))));
    assert_eq!(nodes.choices(&sidebar)[1].id, old_id);
    sidebar.apply_diff(RoomListDiff::Remove(scope(17, "!space:test")));
    assert!(nodes.choices(&sidebar).is_empty());
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(room(
        17,
        "!space:test",
        "Returned",
        true,
    ))));
    assert_ne!(nodes.choices(&sidebar)[1].id, old_id);
    let mut other = Sidebar::connected(AccountId::new(18));
    other.apply_diff(RoomListDiff::Upsert(Box::new(room(
        18,
        "!space:test",
        "Other account",
        true,
    ))));
    assert_ne!(nodes.choices(&other)[1].id, old_id);
}

#[test]
fn space_intents_reject_wrong_accounts_removed_choices_duplicate_and_hidden_actions() {
    use matrix_desktop::accessibility::spaces::SpacesAction;
    let mut current = snapshot();
    let request = |id, action| accesskit::ActionRequest {
        action,
        target_node: id,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    let choose = current.action(&request(NodeId(61), Action::Click)).unwrap();
    assert!(
        matches!(&choose, SpacesAction::Choose { account, scope: None, previous: Some(_), .. } if *account == AccountId::new(17))
    );
    assert!(current.allows(&choose));
    current.open = false;
    assert!(!current.allows(&choose));
    assert!(
        current
            .action(&request(NodeId(61), Action::Focus))
            .is_none()
    );
    let open = current.action(&request(TOGGLE, Action::Click)).unwrap();
    assert!(current.allows(&open));
    current.open = true;
    assert!(!current.allows(&open));
    current.active = None;
    assert!(!current.allows(&choose));
    let space = current
        .action(&request(NodeId(1 << 57), Action::Click))
        .unwrap();
    current.account = AccountId::new(18);
    assert!(!current.allows(&space));
    current.account = AccountId::new(17);
    current.choices.pop();
    assert!(!current.allows(&space));
    let mut malformed = request(TOGGLE, Action::Click);
    malformed.data = Some(accesskit::ActionData::Value("not supported".into()));
    assert!(current.action(&malformed).is_none());
    malformed.data = None;
    malformed.target_node = NodeId(99);
    assert!(current.action(&malformed).is_none());
}

fn navigation() -> matrix_desktop::accessibility::NavigationSnapshot {
    matrix_desktop::accessibility::NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
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
        settings: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    }
}

#[test]
fn semantic_space_paths_retire_when_the_filter_revision_changes() {
    use matrix_desktop::accessibility::{NavigationAction, semantic_session::SemanticSession};
    let mut view = navigation();
    view.spaces = Some(snapshot());
    let mut session = SemanticSession::new(view.clone());
    for action in [Action::Focus, Action::Click] {
        let handle = session
            .tree()
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Spazio progetto"))
            .unwrap()
            .0;
        let request = accesskit::ActionRequest {
            action,
            target_node: handle,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        };
        let Some(NavigationAction::Spaces(queued)) = session.action(&request) else {
            panic!("Space missing")
        };
        let mut newer = view.clone();
        newer.spaces.as_mut().unwrap().revision += 1;
        session.update(newer.clone());
        assert!(
            session.action(&request).is_none(),
            "old native path must be retired, not translated into a new intent"
        );
        assert!(!newer.spaces.as_ref().unwrap().allows(&queued));
        view = newer;
    }
}

#[test]
fn queued_room_selection_cannot_revive_after_filter_away_back() {
    use matrix_desktop::{
        Sidebar,
        accessibility::{NavigationAction, RoomNodes, semantic_session::SemanticSession},
    };
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    let mut nodes = RoomNodes::default();
    let mut view = navigation();
    view.rooms = nodes.project(&sidebar);
    let mut session = SemanticSession::new(view.clone());
    let handle = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Design desktop"))
        .unwrap()
        .0;
    let request = accesskit::ActionRequest {
        action: Action::Click,
        target_node: handle,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    let Some(NavigationAction::ActivateRoom(queued)) = session.action(&request) else {
        panic!("room action missing")
    };
    assert!(nodes.resolve_action(&queued, &sidebar).is_some());
    sidebar.set_query("missing");
    view.rooms = nodes.project(&sidebar);
    session.update(view.clone());
    assert!(session.action(&request).is_none());
    sidebar.set_query("");
    view.rooms = nodes.project(&sidebar);
    session.update(view);
    assert!(session.action(&request).is_none());
    assert!(nodes.resolve_action(&queued, &sidebar).is_none());
}

#[test]
fn native_room_path_cannot_translate_a_fresh_intent_after_unobserved_filter_return() {
    use matrix_desktop::{
        Sidebar,
        accessibility::{RoomNodes, semantic_session::SemanticSession},
    };
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    let mut nodes = RoomNodes::default();
    let mut view = navigation();
    view.rooms = nodes.project(&sidebar);
    let mut session = SemanticSession::new(view.clone());
    let old = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Design desktop"))
        .unwrap()
        .0;
    let request = accesskit::ActionRequest {
        action: Action::Click,
        target_node: old,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    assert!(session.action(&request).is_some());
    sidebar.set_query("missing");
    sidebar.set_query("");
    view.rooms = nodes.project(&sidebar);
    session.update(view);
    assert!(
        session.action(&request).is_none(),
        "retired room path must not be rebound to the new navigation revision"
    );
}

#[test]
fn queued_space_actions_cannot_revive_after_menu_close_reopen_without_paint() {
    use matrix_desktop::Sidebar;
    let mut sidebar = Sidebar::demo(AccountId::new(17));
    sidebar.set_navigation_visibility(false, true, true);
    let mut original = snapshot();
    original.revision = sidebar.navigation_revision();
    let queued = original
        .action(&accesskit::ActionRequest {
            action: Action::Click,
            target_node: original.choices[1].id,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        })
        .unwrap();
    sidebar.set_navigation_visibility(false, true, false);
    sidebar.set_navigation_visibility(false, true, true);
    let mut current = original.clone();
    current.revision = sidebar.navigation_revision();
    assert!(!current.allows(&queued));
    let stable = sidebar.navigation_revision();
    sidebar.set_navigation_visibility(false, true, true);
    assert_eq!(stable, sidebar.navigation_revision());
}

#[test]
fn space_revision_retires_filter_away_back_and_unobserved_removal() {
    use matrix_desktop::Sidebar;
    use matrix_engine::{RoomInfo, RoomListDiff, RoomSummary};
    let mut sidebar = Sidebar::connected(AccountId::new(17));
    let summary = || RoomSummary {
        scope: scope(17, "!space:test"),
        name: "Workspace".into(),
        unread: 0,
        info: RoomInfo {
            is_space: true,
            ..Default::default()
        },
    };
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(summary())));
    let initial = sidebar.navigation_revision();
    assert!(sidebar.set_space(Some(scope(17, "!space:test"))));
    assert!(sidebar.set_space(None));
    assert_ne!(initial, sidebar.navigation_revision());
    let initial = sidebar.navigation_revision();
    sidebar.apply_diff(RoomListDiff::Remove(scope(17, "!space:test")));
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(summary())));
    assert_ne!(initial, sidebar.navigation_revision());
    let current = sidebar.navigation_revision();
    assert!(!sidebar.set_space(Some(scope(18, "!space:test"))));
    sidebar.set_space(None);
    sidebar.apply_diff(RoomListDiff::Upsert(Box::new(summary())));
    assert_eq!(current, sidebar.navigation_revision());
}

#[test]
fn queued_space_actions_reject_a_new_filter_revision() {
    let original = snapshot();
    for action in [Action::Focus, Action::Click] {
        for id in [TOGGLE, original.choices[0].id, original.choices[1].id] {
            let queued = original
                .action(&accesskit::ActionRequest {
                    action,
                    target_node: id,
                    target_tree: accesskit::TreeId::ROOT,
                    data: None,
                })
                .unwrap();
            assert!(original.allows(&queued));
            let mut replacement = original.clone();
            replacement.revision += 1;
            assert!(!replacement.allows(&queued));
        }
    }
}

#[test]
fn queued_space_choice_cannot_target_a_replacement_with_the_same_scope() {
    let original = snapshot();
    let queued = original
        .action(&accesskit::ActionRequest {
            action: Action::Click,
            target_node: original.choices[1].id,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        })
        .unwrap();
    assert!(original.allows(&queued));
    let mut replacement = original.clone();
    replacement.choices[1].id = NodeId((1 << 57) + 1);
    assert!(
        !replacement.allows(&queued),
        "a removed Space capability must not select its replacement"
    );
}

#[test]
fn spaces_export_expanded_state_selected_choices_focus_and_observed_bounds() {
    let mut snapshot = snapshot();
    let bounds = accesskit::Rect::new(0., 64., 320., 96.);
    snapshot.bounds.insert(TOGGLE, bounds);
    let nodes = snapshot.nodes();
    let toggle = &nodes.iter().find(|(id, _)| *id == TOGGLE).unwrap().1;
    assert_eq!(toggle.role(), Role::Button);
    assert_eq!(toggle.label(), Some("Spazi: Spazio progetto"));
    assert_eq!(toggle.is_expanded(), Some(true));
    assert_eq!(
        toggle.description(),
        Some("Filtro degli Spazi joined · Aperto")
    );
    assert_eq!(toggle.bounds(), Some(bounds));
    assert!(toggle.supports_action(Action::Focus));
    assert!(toggle.supports_action(Action::Click));
    let choice = &nodes
        .iter()
        .find(|(id, _)| *id == NodeId(1 << 57))
        .unwrap()
        .1;
    assert_eq!(choice.role(), Role::RadioButton);
    assert_eq!(choice.label(), Some("Spazio progetto"));
    assert_eq!(choice.toggled(), Some(accesskit::Toggled::True));
    assert_eq!(nodes.len(), 3);
    snapshot.open = false;
    let nodes = snapshot.nodes();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].1.is_expanded(), Some(false));
}
