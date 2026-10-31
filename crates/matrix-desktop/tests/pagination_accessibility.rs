#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, NodeId, Rect, Role, TreeId};
use matrix_desktop::{
    HistorySession, HistoryStatus,
    accessibility::{
        HistorySnapshot, NavigationAction, NavigationSnapshot, pagination::ServerPagination,
    },
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn snapshot(tag: matrix_engine::RequestTag, status: HistoryStatus) -> NavigationSnapshot {
    NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
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
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        list_focused: false,
        composer: None,
        history: Some(HistorySnapshot {
            server: Some(ServerPagination {
                tag,
                status,
                bounds: Some(Rect::new(320., 60., 1100., 92.)),
                focused: false,
            }),
            ..Default::default()
        }),
    }
}
fn scope(name: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(36),
        room_id: RoomId::new(name),
    }
}
fn request(id: NodeId, action: Action) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    }
}
fn server_id(snapshot: &NavigationSnapshot) -> NodeId {
    snapshot
        .history
        .as_ref()
        .unwrap()
        .server
        .as_ref()
        .unwrap()
        .node()
        .unwrap()
        .0
}

#[test]
fn server_pagination_exports_status_and_callable_action_only_when_available() {
    let mut session = HistorySession::new();
    let tag = session.select(Some(scope("!alpha:test"))).unwrap().tag;
    for (status, label, actionable) in [
        (HistoryStatus::Loading, "Caricamento storico…", false),
        (
            HistoryStatus::More,
            "Carica messaggi precedenti dal server",
            true,
        ),
        (
            HistoryStatus::Start,
            "Inizio dello storico raggiunto",
            false,
        ),
        (
            HistoryStatus::Failed,
            "Storico non disponibile · riprova",
            true,
        ),
    ] {
        let snapshot = snapshot(tag.clone(), status);
        let tree = snapshot.tree();
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .expect("server pagination node");
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.bounds(), Some(Rect::new(320., 60., 1100., 92.)));
        assert_eq!(node.supports_action(Action::Click), actionable);
        assert_eq!(
            snapshot.action(&request(*id, Action::Click)).is_some(),
            actionable
        );
    }
}

#[test]
fn server_status_can_receive_semantic_focus_even_when_no_more_pages_exist() {
    let mut session = HistorySession::new();
    let tag = session.select(Some(scope("!alpha:test"))).unwrap().tag;
    for status in [
        HistoryStatus::Loading,
        HistoryStatus::More,
        HistoryStatus::Start,
        HistoryStatus::Failed,
    ] {
        let mut snapshot = snapshot(tag.clone(), status);
        let id = server_id(&snapshot);
        assert!(matches!(
            snapshot.action(&request(id, Action::Focus)),
            Some(NavigationAction::FocusServerHistory(_))
        ));
        snapshot
            .history
            .as_mut()
            .unwrap()
            .server
            .as_mut()
            .unwrap()
            .focused = true;
        assert_eq!(snapshot.tree().focus, id);
    }
}

#[test]
fn pagination_handle_never_aliases_a_device_overview_label() {
    use matrix_desktop::accessibility::devices::{DeviceControl, DeviceOverview};
    let mut session = HistorySession::new();
    let tag = session.select(Some(scope("!alpha:test"))).unwrap().tag;
    let id = server_id(&snapshot(tag, HistoryStatus::More));
    let account = AccountId::new(36);
    let model = matrix_desktop::DeviceVerification::new(account);
    let panel = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 6]);
    assert!(
        !panel.tree().nodes.iter().any(|(other, _)| *other == id),
        "old AT page object must not be a label in another panel"
    );
}

#[test]
fn retired_page_identity_and_hidden_history_do_not_accept_assistive_requests() {
    let mut session = HistorySession::new();
    let tag = session.select(Some(scope("!alpha:test"))).unwrap().tag;
    let old = snapshot(tag, HistoryStatus::More);
    let id = server_id(&old);
    let next = session.select(Some(scope("!beta:test"))).unwrap().tag;
    let current = snapshot(next.clone(), HistoryStatus::More);
    assert_ne!(server_id(&current), id);
    for action in [Action::Click, Action::Focus] {
        assert!(
            old.action(&request(id, action)).is_none(),
            "cancelled snapshot"
        );
        assert!(
            current.action(&request(id, action)).is_none(),
            "retired AT object"
        );
        let mut hidden = current.clone();
        hidden.history = None;
        assert!(
            hidden
                .action(&request(server_id(&current), action))
                .is_none(),
            "hidden chat"
        );
    }
    let mut malformed = request(server_id(&current), Action::Click);
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(current.action(&malformed).is_none());
    assert!(
        snapshot(next, HistoryStatus::Empty)
            .history
            .unwrap()
            .server
            .unwrap()
            .node()
            .is_none()
    );
}
