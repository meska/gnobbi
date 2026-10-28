#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};

fn snapshot(history: matrix_desktop::accessibility::HistorySnapshot) -> NavigationSnapshot {
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
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(history),
        list_focused: false,
        composer: None,
    }
}

fn server_history(account: u64, room: &str) -> DemoHistory {
    let mut history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(account),
            room_id: RoomId::new(room),
        },
        0,
    );
    history.prepend_server(vec![matrix_engine::history::HistoryEvent {
        event_id: EventId::new("$same:test"),
        body: "same text".into(),
        outgoing: true,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
    }]);
    history
}

#[test]
fn actions_opener_has_independent_focus_bounds_and_keyboard_navigation() {
    let mut nodes = HistoryNodes::default();
    let history = server_history(36, "!alpha:test");
    let target = history.events()[0].id.clone();
    let mut projected = nodes.project(&history, 0, true);
    assert!(
        matches!(projected.actions_key("tab", false), Some(NavigationAction::FocusMessageActions(id)) if id == target)
    );
    assert!(projected.actions_key("enter", false).is_none());
    let rect = accesskit::Rect::new(900., 610., 970., 634.);
    projected.messages[0].actions_bounds = Some(rect);
    projected.messages[0].actions_focused = true;
    let current = snapshot(projected);
    let tree = current.tree();
    let (id, button) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Apri azioni messaggio"))
        .unwrap();
    assert!(button.supports_action(Action::Focus));
    assert_eq!(button.bounds(), Some(rect));
    assert_eq!(tree.focus, *id);
    let request = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Focus,
        data: None,
    };
    assert!(
        matches!(current.action(&request), Some(NavigationAction::FocusMessageActions(id)) if id == target)
    );
    let projected = current.history.as_ref().unwrap();
    assert!(projected.allows_actions(&target));
    let mut other = target.clone();
    other.scope.account_id = AccountId::new(37);
    assert!(!projected.allows_actions(&other));
    assert!(
        matches!(projected.actions_key("tab", true), Some(NavigationAction::FocusMessage(id)) if id == target)
    );
    assert!(matches!(
        projected.actions_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    for key in ["space", "enter"] {
        assert!(
            matches!(projected.actions_key(key, false), Some(NavigationAction::OpenMessageActions(id)) if id == target)
        );
    }
    let mut hidden = projected.clone();
    hidden.focused = false;
    assert!(hidden.actions_key("enter", false).is_none());
    assert!(projected.actions_key("a", false).is_none());
    assert!(projected.actions_key("enter", true).is_none());
}

#[test]
fn actions_opener_identity_is_stable_while_visible_and_retired_after_hide() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut nodes = HistoryNodes::default();
    let history = server_history(36, "!alpha:test");
    let projected = snapshot(nodes.project(&history, 0, false));
    let mut session = SemanticSession::new(projected.clone());
    let opener_id = |tree: accesskit::TreeUpdate| {
        tree.nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Apri azioni messaggio"))
            .unwrap()
            .0
    };
    let original = opener_id(session.tree());
    let unique: std::collections::HashSet<_> =
        projected.tree().nodes.iter().map(|(id, _)| *id).collect();
    assert_eq!(unique.len(), projected.tree().nodes.len());
    session.update(projected.clone());
    assert_eq!(opener_id(session.tree()), original);
    let request = |id| ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: id,
        action: Action::Click,
        data: None,
    };
    let mut hidden = projected.clone();
    hidden.history = None;
    session.update(hidden);
    assert!(session.action(&request(original)).is_none());
    session.update(projected);
    let restored = opener_id(session.tree());
    assert_ne!(restored, original);
    assert!(session.action(&request(original)).is_none());
    assert!(
        matches!(session.action(&request(restored)), Some(NavigationAction::OpenMessageActions(id)) if id == history.events()[0].id)
    );
}

#[test]
fn server_message_exports_a_scoped_actions_opener() {
    let mut nodes = HistoryNodes::default();
    let history = server_history(36, "!alpha:test");
    let mut current = snapshot(nodes.project(&history, 0, false));
    let mut operations = matrix_desktop::LiveOperations::default();
    operations.select(history.events()[0].id.clone());
    current.operation_panel =
        matrix_desktop::accessibility::operation_panel::PanelSnapshot::project(
            &operations,
            true,
            None,
        );
    let close = current.operation_panel.as_ref().unwrap().close;
    assert!(current.tree().nodes.iter().any(|(id, _)| *id == close));
    let close_request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: close,
        data: None,
    };
    assert!(matches!(
        current.action(&close_request),
        Some(NavigationAction::OperationPanel(_))
    ));
    current.operation_panel = None;
    assert!(current.action(&close_request).is_none());
    let tree = current.tree();
    let (id, button) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Apri azioni messaggio"))
        .expect("server message needs its own actions opener");
    assert_eq!(button.role(), Role::Button);
    assert!(button.supports_action(Action::Click));
    assert!(
        button.bounds().is_none(),
        "unpainted opener cannot invent geometry"
    );
    let parent = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("same text"))
        .unwrap();
    assert!(parent.1.children().contains(id));
    let request = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Click,
        data: None,
    };
    assert!(
        matches!(current.action(&request), Some(NavigationAction::OpenMessageActions(target)) if target == history.events()[0].id)
    );
    let mut hidden = current.clone();
    hidden.history = None;
    assert!(hidden.action(&request).is_none());
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(current.action(&malformed).is_none());
    let pending = {
        let mut h = DemoHistory::new(
            server_history(36, "!alpha:test").events()[0]
                .id
                .scope
                .clone(),
            0,
        );
        h.submit_text("pending".into());
        h
    };
    assert!(
        snapshot(nodes.project(&pending, 0, false))
            .tree()
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Apri azioni messaggio"))
    );
    assert!(
        snapshot(nodes.project(&server_history(37, "!alpha:test"), 0, false))
            .action(&request)
            .is_none()
    );
    assert!(
        snapshot(nodes.project(&server_history(36, "!beta:test"), 0, false))
            .action(&request)
            .is_none()
    );
}
