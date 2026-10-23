#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, Rect, Role, TreeId};
use matrix_desktop::accessibility::compact::BackNodes;
use matrix_domain::{AccountId, RoomId, RoomScope};

fn scope(account: u64, room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    }
}

#[test]
fn compact_back_exports_scoped_focus_click_and_painted_bounds() {
    let mut nodes = BackNodes::default();
    let room = scope(17, "!room:test");
    let mut view = nodes.project(Some(room.clone())).unwrap();
    view.focused = true;
    view.bounds = Some(Rect::new(24., 12., 86., 44.));
    let node = view.node();
    assert_eq!(node.role(), Role::Button);
    assert_eq!(node.label(), Some("Indietro alle conversazioni"));
    assert_eq!(node.bounds(), view.bounds);
    for action in [Action::Click, Action::Focus] {
        let request = ActionRequest {
            action,
            target_node: view.id,
            target_tree: TreeId::ROOT,
            data: None,
        };
        let intent = view.action(&request).unwrap();
        assert_eq!(intent.scope, room);
        assert_eq!(intent.id, view.id);
        assert_eq!(intent.focus_only, action == Action::Focus);
    }
    assert_eq!(nodes.project(Some(room)).unwrap().id, view.id);
    assert!(nodes.project(None).is_none());
}

#[test]
fn compact_back_keyboard_has_bounded_entry_activation_and_exits() {
    use matrix_desktop::accessibility::compact::BackKey;
    let mut view = BackNodes::default()
        .project(Some(scope(17, "!room:test")))
        .unwrap();
    assert!(matches!(
        view.key("tab", true, true, false),
        Some(BackKey::Focus)
    ));
    assert!(matches!(
        view.key("tab", false, false, true),
        Some(BackKey::Focus)
    ));
    assert!(view.key("enter", false, false, false).is_none());
    view.focused = true;
    for key in ["enter", "space"] {
        assert!(matches!(
            view.key(key, false, false, false),
            Some(BackKey::Activate)
        ));
    }
    assert!(matches!(
        view.key("tab", false, false, false),
        Some(BackKey::Composer)
    ));
    assert!(matches!(
        view.key("tab", true, false, false),
        Some(BackKey::List)
    ));
    assert!(matches!(
        view.key("escape", false, false, false),
        Some(BackKey::List)
    ));
    assert!(view.key("space", true, false, false).is_none());
}

fn navigation() -> matrix_desktop::accessibility::NavigationSnapshot {
    matrix_desktop::accessibility::NavigationSnapshot {
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
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
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
fn compact_back_retires_handles_and_captured_intents_across_replacement_visits() {
    use matrix_desktop::accessibility::{NavigationAction, semantic_session::SemanticSession};
    let mut nodes = BackNodes::default();
    let original = nodes.project(Some(scope(17, "!room:test"))).unwrap();
    let mut view = navigation();
    view.back = Some(original.clone());
    let mut session = SemanticSession::new(view.clone());
    let handle = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Indietro alle conversazioni"))
        .unwrap()
        .0;
    let request = ActionRequest {
        action: Action::Click,
        target_node: handle,
        target_tree: TreeId::ROOT,
        data: None,
    };
    let Some(NavigationAction::CompactBack(queued)) = session.action(&request) else {
        panic!("missing Back intent")
    };
    assert!(original.allows(&queued));
    for replacement in [
        None,
        Some(scope(17, "!other:test")),
        Some(scope(18, "!room:test")),
        None,
        Some(scope(17, "!room:test")),
    ] {
        view.back = nodes.project(replacement);
        session.update(view.clone());
        assert!(session.action(&request).is_none());
        if let Some(current) = &view.back {
            assert!(!current.allows(&queued));
        }
    }
    let current = view.back.as_ref().unwrap();
    let handle = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Indietro alle conversazioni"))
        .unwrap()
        .0;
    let Some(NavigationAction::CompactBack(fresh)) = session.action(&ActionRequest {
        target_node: handle,
        ..request
    }) else {
        panic!("replacement missing")
    };
    assert!(current.allows(&fresh));
    assert_eq!(fresh.scope, original.scope);
    assert_ne!(fresh.id, original.id);
}
