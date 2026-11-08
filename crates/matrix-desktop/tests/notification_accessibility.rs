#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, Toggled, TreeId};
use matrix_desktop::accessibility::notifications::{
    CLOSE, ENABLED, MUTE, NotificationAction, NotificationSnapshot, NotificationView, PREVIEW,
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn request(id: accesskit::NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    }
}
#[test]
fn notification_actions_reject_hidden_disabled_and_stale_room_targets() {
    use matrix_desktop::accessibility::notifications::OPEN;
    let unavailable = NotificationView::Unavailable;
    assert!(unavailable.tree().is_none());
    assert!(unavailable.action(&request(OPEN)).is_none());
    let closed = NotificationView::Closed;
    assert!(closed.tree().is_none());
    assert_eq!(
        closed.action(&request(OPEN)),
        Some(NotificationAction::Open)
    );
    assert!(closed.action(&request(ENABLED)).is_none());
    let view = panel();
    assert_eq!(
        view.action(&request(PREVIEW)),
        Some(NotificationAction::SetPreview(false))
    );
    assert_eq!(
        view.action(&request(CLOSE)),
        Some(NotificationAction::Close)
    );
    let mute = view.action(&request(MUTE)).unwrap();
    assert!(view.allows(&mute));
    assert!(
        !view.allows(&NotificationAction::SetEnabled(false)),
        "duplicate writes must not toggle"
    );
    let no_room = NotificationView::Open(Box::new(NotificationSnapshot {
        bounds: [None; 4],
        focus: Default::default(),
        enabled: false,
        preview: true,
        selected: None,
        room_name: String::new(),
        service_failed: false,
    }));
    assert!(
        !no_room.allows(&mute),
        "queued mute cannot affect a later selection"
    );
    assert!(no_room.action(&request(MUTE)).is_none());
    let tree = no_room.tree().unwrap();
    let mute_node = &tree.nodes.iter().find(|(id, _)| *id == MUTE).unwrap().1;
    assert!(mute_node.is_disabled());
    assert!(!mute_node.supports_action(Action::Click));
    let mut malformed = request(OPEN);
    malformed.action = Action::Focus;
    assert!(view.action(&malformed).is_none());
    malformed.action = Action::Click;
    malformed.data = Some(accesskit::ActionData::Value("true".into()));
    assert!(view.action(&malformed).is_none());
    assert!(!closed.allows(&mute));
}

#[test]
fn open_notification_panel_replaces_hidden_chat_tree_and_rejects_chat_actions() {
    use matrix_desktop::accessibility::notifications::OPEN;
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot, SEARCH};
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![matrix_desktop::panel_openers::Snapshot {
            control: matrix_desktop::panel_openers::Control::Notifications,
            source: 1,
            bounds: None,
            focused: false,
        }],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        unread_filter: None,
        new_room: None,
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: true,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: NotificationView::Closed,
    };
    assert!(snapshot.tree().nodes.iter().any(|(id, _)| *id == OPEN));
    assert!(matches!(
        snapshot.action(&request(OPEN)),
        Some(NavigationAction::PanelOpener(
            matrix_desktop::panel_openers::Intent {
                control: matrix_desktop::panel_openers::Control::Notifications,
                source: 1,
                focus_only: false
            }
        ))
    ));
    snapshot.notifications = panel();
    assert!(snapshot.tree().nodes.iter().all(|(id, _)| *id != SEARCH));
    assert!(snapshot.action(&request(SEARCH)).is_none());
    assert!(snapshot.action(&request(OPEN)).is_none());
    assert!(matches!(
        snapshot.action(&request(CLOSE)),
        Some(NavigationAction::Notifications(NotificationAction::Close))
    ));
    snapshot.notifications = NotificationView::Unavailable;
    snapshot.openers.clear();
    assert!(snapshot.tree().nodes.iter().all(|(id, _)| *id != OPEN));
}

#[test]
fn notification_focus_traversal_skips_unavailable_mute_and_exports_focus_action() {
    use matrix_desktop::accessibility::notifications::NotificationControl;
    let mut view = panel();
    let NotificationView::Open(ref mut snapshot) = view else {
        unreachable!()
    };
    snapshot.focus = NotificationControl::Enabled;
    assert_eq!(snapshot.next_focus(false), NotificationControl::Mute);
    assert_eq!(snapshot.next_focus(true), NotificationControl::Close);
    snapshot.selected = None;
    assert_eq!(snapshot.next_focus(false), NotificationControl::Preview);
    snapshot.focus = NotificationControl::Close;
    assert_eq!(snapshot.next_focus(false), NotificationControl::Enabled);
    assert_eq!(view.tree().unwrap().focus, CLOSE);
    let mut focus = request(PREVIEW);
    focus.action = Action::Focus;
    assert_eq!(
        view.action(&focus),
        Some(NotificationAction::Focus(NotificationControl::Preview))
    );
    focus.target_node = MUTE;
    assert!(view.action(&focus).is_none());
    assert!(!view.allows(&NotificationAction::Focus(NotificationControl::Mute)));
    assert!(
        view.tree()
            .unwrap()
            .nodes
            .iter()
            .any(|(id, n)| *id == PREVIEW && n.supports_action(Action::Focus))
    );
}

fn panel() -> NotificationView {
    NotificationView::Open(Box::new(NotificationSnapshot {
        bounds: [None; 4],
        focus: Default::default(),
        enabled: false,
        preview: true,
        selected: Some((
            RoomScope {
                account_id: AccountId::new(18),
                room_id: RoomId::new("!privacy:fixture.invalid"),
            },
            false,
        )),
        room_name: "Privacy".into(),
        service_failed: true,
    }))
}

#[test]
fn notification_dialog_exports_current_checkbox_states_and_privacy_text() {
    let mut view = panel();
    if let NotificationView::Open(ref mut p) = view {
        p.bounds = [Some(accesskit::Rect::new(16., 80., 1104., 128.)); 4];
    }
    let tree = view.tree().expect("open notification dialog");
    assert_eq!(tree.nodes[0].1.role(), Role::Window);
    assert_eq!(tree.nodes[0].1.label(), Some("Notifiche locali"));
    for (id, label, state) in [
        (ENABLED, "Notifiche locali attive", Toggled::False),
        (MUTE, "Notifiche della stanza Privacy", Toggled::True),
        (PREVIEW, "Anteprima del solo conteggio", Toggled::True),
    ] {
        let node = &tree.nodes.iter().find(|(n, _)| *n == id).unwrap().1;
        assert_eq!(node.role(), Role::Switch);
        assert_eq!(node.label(), Some(label));
        assert_eq!(node.toggled(), Some(state));
        assert_eq!(
            node.bounds(),
            Some(accesskit::Rect::new(16., 80., 1104., 128.))
        );
        assert!(node.supports_action(Action::Click));
    }
    // The help sentences were removed from the panel; only the error state stays.
    assert!(!tree.nodes.iter().any(|(_, node)| {
        node.value()
            .is_some_and(|s| s.contains("Mai testo, mittente, stanza, media o segreti"))
    }));
    assert!(tree.nodes.iter().any(|(_, node)| {
        node.value()
            .is_some_and(|s| s.contains("Servizio OS non disponibile"))
    }));
    assert!(
        tree.nodes
            .iter()
            .any(|(id, node)| *id == CLOSE && node.supports_action(Action::Click))
    );
    assert!(matches!(
        panel().action(&request(ENABLED)),
        Some(NotificationAction::SetEnabled(true))
    ));
}
