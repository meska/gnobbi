#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::{
    DemoHistory, OfflineDelivery,
    accessibility::{
        HistoryNodes, NavigationAction, NavigationSnapshot, offline_delivery::Snapshot,
    },
};
use matrix_domain::{AccountId, RoomId, RoomScope};
fn snapshot() -> NavigationSnapshot {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(64),
            room_id: RoomId::new("!selection:test"),
        },
        20,
    );
    NavigationSnapshot {
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
        disconnected: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(HistoryNodes::default().project(&history, 0, true)),
        list_focused: false,
        composer: None,
    }
}

#[test]
fn semantic_paths_and_queued_local_intents_do_not_survive_state_or_visit_return() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut h = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(81),
            room_id: RoomId::new("!offline:test"),
        },
        0,
    );
    h.submit_text("locale".into());
    let mut sources = OfflineDelivery::default();
    let target = sources.target(&h.events()[0]).unwrap();
    let mut current = snapshot();
    let mut history = HistoryNodes::default().project(&h, 0, true);
    history.mark_offline();
    history.messages[0].offline_delivery = Some(Snapshot {
        target: target.clone(),
        bounds: None,
        focused: false,
    });
    current.history = Some(history);
    let mut session = SemanticSession::new(current.clone());
    let id = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Simula errore · demo offline"))
        .unwrap()
        .0;
    let request = accesskit::ActionRequest {
        target_tree: accesskit::TreeId::ROOT,
        target_node: id,
        action: Action::Click,
        data: None,
    };
    let Some(NavigationAction::OfflineDelivery(queued)) = session.action(&request) else {
        panic!("local intent")
    };
    sources.apply(&target, &mut h).unwrap();
    let failed = sources.target(&h.events()[0]).unwrap();
    current.history.as_mut().unwrap().messages[0]
        .offline_delivery
        .as_mut()
        .unwrap()
        .target = failed.clone();
    session.update(current.clone());
    assert!(session.action(&request).is_none());
    sources.apply(&failed, &mut h).unwrap();
    sources.clear();
    let mut away = current.clone();
    away.history = None;
    session.update(away);
    let returned = sources.target(&h.events()[0]).unwrap();
    current.history.as_mut().unwrap().messages[0]
        .offline_delivery
        .as_mut()
        .unwrap()
        .target = returned.clone();
    session.update(current);
    assert!(session.action(&request).is_none());
    assert!(!sources.is_current(&queued.target, &h));
    assert!(sources.is_current(&returned, &h));
    let mut malformed = request;
    malformed.target_node = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Simula errore · demo offline"))
        .unwrap()
        .0;
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(session.action(&malformed).is_none());
}

#[test]
fn keyboard_simulation_is_separate_from_real_retry() {
    let mut h = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(81),
            room_id: RoomId::new("!offline:test"),
        },
        0,
    );
    h.submit_text("locale".into());
    let target = OfflineDelivery::default().target(&h.events()[0]).unwrap();
    let mut history = HistoryNodes::default().project(&h, 0, true);
    history.mark_offline();
    history.messages[0].offline_delivery = Some(Snapshot {
        target: target.clone(),
        bounds: None,
        focused: false,
    });
    assert!(
        matches!(history.offline_delivery_key("f9", false), Some(NavigationAction::OfflineDelivery(i)) if i.focus_only)
    );
    history.messages[0]
        .offline_delivery
        .as_mut()
        .unwrap()
        .focused = true;
    for key in ["space", "enter"] {
        assert!(
            matches!(history.offline_delivery_key(key, false), Some(NavigationAction::OfflineDelivery(i)) if !i.focus_only && i.target.instance() == target.instance())
        );
        assert!(history.retry_key(key, false).is_none());
    }
    assert!(matches!(
        history.offline_delivery_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    assert!(matches!(
        history.offline_delivery_key("escape", false),
        Some(NavigationAction::FocusMessage(_))
    ));
    assert!(matches!(
        history.offline_delivery_key("tab", true),
        Some(NavigationAction::FocusMessage(_))
    ));
    history.focused = false;
    assert!(history.offline_delivery_key("f9", false).is_none());
}

#[test]
fn local_transitions_have_truthful_named_scoped_focus_and_click() {
    let mut h = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(81),
            room_id: RoomId::new("!offline:test"),
        },
        0,
    );
    h.submit_text("locale 🙂".into());
    let mut sources = OfflineDelivery::default();
    let mut nodes = HistoryNodes::default();
    for label in ["Simula errore · demo offline", "Riprova · demo offline"] {
        let target = sources.target(&h.events()[0]).unwrap();
        let mut current = snapshot();
        let mut history = nodes.project(&h, 0, true);
        history.mark_offline();
        let rect = accesskit::Rect::new(400., 200., 650., 224.);
        history.messages[0].offline_delivery = Some(Snapshot {
            target: target.clone(),
            bounds: Some(rect),
            focused: true,
        });
        assert!(history.messages[0].retry.is_none());
        assert!(history.messages[0].actions.is_none());
        current.history = Some(history);
        let tree = current.tree();
        let (id, button) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .expect("local action required");
        assert_eq!(button.role(), Role::Button);
        assert_eq!(button.bounds(), Some(rect));
        assert_eq!(
            button.description(),
            Some("Simulazione locale · Nessuna consegna Matrix")
        );
        assert_eq!(tree.focus, *id);
        for action in [Action::Focus, Action::Click] {
            assert!(button.supports_action(action));
            let request = accesskit::ActionRequest {
                target_tree: accesskit::TreeId::ROOT,
                target_node: *id,
                action,
                data: None,
            };
            let Some(NavigationAction::OfflineDelivery(intent)) = current.action(&request) else {
                panic!("distinct local dispatch required")
            };
            assert_eq!(intent.target.instance(), target.instance());
            assert_eq!(intent.focus_only, action == Action::Focus);
        }
        sources.apply(&target, &mut h).unwrap();
    }
}

#[test]
fn visible_tree_has_unique_node_ids_and_children() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut nav = snapshot();
    nav.visible = true;
    let tree = SemanticSession::new(nav).tree();
    let mut ids = std::collections::HashSet::new();
    for (id, node) in &tree.nodes {
        assert!(ids.insert(*id), "duplicate node {id:?}");
        let mut kids = std::collections::HashSet::new();
        assert!(node.children().iter().all(|c| kids.insert(*c)), "duplicate child of {id:?}");
    }
}
