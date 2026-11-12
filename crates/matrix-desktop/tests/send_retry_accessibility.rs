#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationSnapshot},
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn snapshot(history: matrix_desktop::accessibility::HistorySnapshot) -> NavigationSnapshot {
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
        history: Some(history),
        list_focused: false,
        composer: None,
    }
}

#[test]
fn failed_local_message_exports_an_explicit_scoped_retry_action() {
    let scope = RoomScope {
        account_id: AccountId::new(36),
        room_id: RoomId::new("!retry:fixture.invalid"),
    };
    let mut history = DemoHistory::new(scope, 0);
    history.submit_text("caffè 🙂".into());
    let target = history.events()[0].id.clone();
    let mut nodes = HistoryNodes::default();
    assert!(
        snapshot(nodes.project(&history, 0, false))
            .tree()
            .nodes
            .iter()
            .all(|(_, node)| node.label() != Some("Riprova invio messaggio"))
    );
    assert!(history.fail_local(&target));
    let current = snapshot(nodes.project(&history, 0, true));
    let tree = current.tree();
    let (id, retry) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Riprova invio messaggio"))
        .expect("failed message needs a retry control");
    assert_eq!(retry.role(), Role::Button);
    assert!(retry.supports_action(Action::Click));
    let parent = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("caffè 🙂"))
        .unwrap();
    assert!(parent.1.children().contains(id));
    let request = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Click,
        data: None,
    };
    assert!(
        matches!(current.action(&request), Some(matrix_desktop::accessibility::NavigationAction::RetryMessage(id)) if id == target),
        "retry must carry the exact message scope and transaction"
    );
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(current.action(&malformed).is_none());
    let mut hidden = current.clone();
    hidden.history = None;
    assert!(hidden.action(&request).is_none());
    assert!(current.history.as_ref().unwrap().allows_retry(&target));
    assert!(!matrix_desktop::accessibility::HistorySnapshot::default().allows_retry(&target));
    let mut changed_scope = target.clone();
    changed_scope.scope.room_id = RoomId::new("!other:fixture.invalid");
    assert!(
        !current
            .history
            .as_ref()
            .unwrap()
            .allows_retry(&changed_scope)
    );
    assert!(history.retry_local(&target));
    assert!(!nodes.project(&history, 0, false).allows_retry(&target));
    assert!(
        snapshot(nodes.project(&history, 0, false))
            .action(&request)
            .is_none(),
        "duplicate queued intent must not retry pending state"
    );
}

#[test]
fn failed_retry_is_an_independently_focusable_observed_control() {
    let scope = RoomScope {
        account_id: AccountId::new(36),
        room_id: RoomId::new("!focus:test"),
    };
    let mut history = DemoHistory::new(scope, 0);
    history.submit_text("focus fixture".into());
    let target = history.events()[0].id.clone();
    assert!(history.fail_local(&target));
    let mut nodes = HistoryNodes::default();
    let current = snapshot(nodes.project(&history, 0, true));
    let tree = current.tree();
    let (id, retry) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Riprova invio messaggio"))
        .unwrap();
    assert!(
        retry.supports_action(Action::Focus),
        "retry needs its own focus intent"
    );
    let request = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Focus,
        data: None,
    };
    assert!(
        current.action(&request).is_some(),
        "focus must resolve only the failed transaction"
    );
    assert!(
        retry.bounds().is_none(),
        "unpainted controls must not invent geometry"
    );
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(current.action(&malformed).is_none());
    let mut hidden = current.clone();
    hidden.history = None;
    assert!(hidden.action(&request).is_none());
    assert!(history.retry_local(&target));
    assert!(
        snapshot(nodes.project(&history, 0, true))
            .action(&request)
            .is_none()
    );
}

#[test]
fn retry_focus_and_bounds_belong_to_the_button_not_the_message() {
    let mut history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(36),
            room_id: RoomId::new("!bounds:test"),
        },
        0,
    );
    history.submit_text("bounds fixture".into());
    assert!(history.fail_local(&history.events()[0].id.clone()));
    let mut nodes = HistoryNodes::default();
    let mut projected = nodes.project(&history, 0, true);
    let rect = accesskit::Rect::new(900., 610., 960., 634.);
    projected.messages[0].retry_bounds = Some(rect);
    projected.messages[0].retry_focused = true;
    let current = snapshot(projected);
    let tree = current.tree();
    let (id, retry) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Riprova invio messaggio"))
        .unwrap();
    assert_eq!(retry.bounds(), Some(rect));
    assert_eq!(tree.focus, *id);
    let request = ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: *id,
        action: Action::Focus,
        data: None,
    };
    assert!(
        matches!(current.action(&request), Some(matrix_desktop::accessibility::NavigationAction::FocusMessageRetry(id)) if id == history.events()[0].id)
    );
}

#[test]
fn retry_keyboard_routes_only_the_current_failed_message() {
    use matrix_desktop::accessibility::NavigationAction;
    let mut history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(36),
            room_id: RoomId::new("!keys:test"),
        },
        0,
    );
    history.submit_text("keys fixture".into());
    let id = history.events()[0].id.clone();
    assert!(history.fail_local(&id));
    let mut nodes = HistoryNodes::default();
    let mut view = nodes.project(&history, 0, true);
    assert!(
        matches!(view.retry_key("tab", false), Some(NavigationAction::FocusMessageRetry(target)) if target == id)
    );
    assert!(view.retry_key("space", false).is_none());
    view.messages[0].retry_focused = true;
    assert!(
        matches!(view.retry_key("tab", true), Some(NavigationAction::FocusMessage(target)) if target == id)
    );
    assert!(matches!(
        view.retry_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    for key in ["space", "enter"] {
        assert!(
            matches!(view.retry_key(key, false), Some(NavigationAction::RetryMessage(target)) if target == id)
        );
    }
    assert!(view.retry_key("a", false).is_none());
    view.focused = false;
    assert!(view.retry_key("enter", false).is_none());
    view.focused = true;
    view.messages[0].retry = None;
    assert!(view.retry_key("enter", false).is_none());
}

#[test]
fn retry_objects_are_scoped_and_retired_between_failure_attempts() {
    use matrix_desktop::accessibility::{NavigationAction, semantic_session::SemanticSession};
    let mut nodes = HistoryNodes::default();
    let mut snapshots = Vec::new();
    for (account, room) in [(36, "!alpha:test"), (36, "!beta:test"), (37, "!alpha:test")] {
        let mut history = DemoHistory::new(
            RoomScope {
                account_id: AccountId::new(account),
                room_id: RoomId::new(room),
            },
            0,
        );
        history.submit_text("same fixture text".into());
        let id = history.events()[0].id.clone();
        assert!(history.fail_local(&id));
        snapshots.push((snapshot(nodes.project(&history, 0, false)), id));
    }
    let mut ids = std::collections::HashSet::new();
    for (current, target) in &snapshots {
        let tree = current.tree();
        let retry = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Riprova invio messaggio"))
            .unwrap()
            .0;
        assert!(ids.insert(retry));
        let request = ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: retry,
            data: None,
        };
        assert!(
            matches!(current.action(&request), Some(NavigationAction::RetryMessage(id)) if id == *target)
        );
        for (other, other_target) in &snapshots {
            if other_target != target {
                assert!(other.action(&request).is_none());
            }
        }
        let unique: std::collections::HashSet<_> = tree.nodes.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            unique.len(),
            tree.nodes.len(),
            "retry cannot alias a message or fixed control"
        );
    }
    let current = snapshots[0].0.clone();
    let mut session = SemanticSession::new(current.clone());
    let original = session
        .tree()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Riprova invio messaggio"))
        .unwrap()
        .0;
    let mut pending = current.clone();
    pending.history.as_mut().unwrap().messages[0].retry = None;
    session.update(pending);
    session.update(current);
    let restored = session
        .tree()
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Riprova invio messaggio"))
        .unwrap()
        .0;
    assert_ne!(original, restored);
    let request = |id| ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    };
    assert!(
        session.action(&request(original)).is_none(),
        "old failed attempt must not retry a later failed attempt"
    );
    assert!(session.action(&request(restored)).is_some());
}
