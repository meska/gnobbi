#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role, Toggled};
use matrix_desktop::accessibility::filters::{UNREAD, UnreadFilter};
use matrix_domain::AccountId;

#[test]
fn unread_control_is_absent_when_hidden_or_covered_by_a_sheet() {
    use matrix_desktop::accessibility::{
        NavigationAction, NavigationSnapshot, notifications::NotificationView,
    };
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
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
        unread_filter: Some(UnreadFilter {
            account: AccountId::new(17),
            revision: 0,
            checked: false,
            focused: true,
            bounds: None,
        }),
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
        target_node: UNREAD,
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    assert!(matches!(
        snapshot.action(&request),
        Some(NavigationAction::Filter(_))
    ));
    assert_eq!(snapshot.tree().focus, UNREAD);
    snapshot.visible = false;
    assert!(snapshot.action(&request).is_none());
    assert!(!snapshot.tree().nodes.iter().any(|(id, _)| *id == UNREAD));
    snapshot.visible = true;
    snapshot.notifications = NotificationView::Open(Box::new(
        matrix_desktop::accessibility::notifications::NotificationSnapshot {
            bounds: [None; 4],
            focus: Default::default(),
            enabled: false,
            preview: false,
            selected: None,
            service_failed: false,
        },
    ));
    assert!(snapshot.action(&request).is_none());
    assert!(!snapshot.tree().nodes.iter().any(|(id, _)| *id == UNREAD));
    snapshot.notifications = Default::default();
    snapshot.unread_filter = None;
    assert!(snapshot.action(&request).is_none());
    assert!(!snapshot.tree().nodes.iter().any(|(id, _)| *id == UNREAD));
}

#[test]
fn unread_intents_are_account_scoped_and_duplicate_sets_are_rejected() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::filters::FilterAction;
    let mut current = UnreadFilter {
        account: AccountId::new(17),
        revision: 0,
        checked: false,
        focused: false,
        bounds: None,
    };
    let request = |action| ActionRequest {
        action,
        target_node: UNREAD,
        target_tree: TreeId::ROOT,
        data: None,
    };
    let set = current.action(&request(Action::Click)).unwrap();
    assert_eq!(
        set,
        FilterAction::SetUnread {
            account: AccountId::new(17),
            revision: 0,
            checked: true
        }
    );
    assert!(current.allows(&set));
    current.checked = true;
    assert!(!current.allows(&set));
    current.account = AccountId::new(18);
    current.checked = false;
    assert!(!current.allows(&set));
    let focus = current.action(&request(Action::Focus)).unwrap();
    assert!(current.allows(&focus));
    current.account = AccountId::new(19);
    assert!(!current.allows(&focus));
    let mut malformed = request(Action::Click);
    malformed.target_node = accesskit::NodeId(51);
    assert!(current.action(&malformed).is_none());
    malformed.target_node = UNREAD;
    malformed.data = Some(accesskit::ActionData::Value("unsafe".into()));
    assert!(current.action(&malformed).is_none());
}

#[test]
fn unread_filter_projects_checkbox_state_focus_and_observed_bounds() {
    for checked in [false, true] {
        let bounds = accesskit::Rect::new(10., 20., 70., 40.);
        let filter = UnreadFilter {
            account: AccountId::new(17),
            revision: 0,
            checked,
            focused: true,
            bounds: Some(bounds),
        };
        let (id, node) = filter.node();
        assert_eq!(id, UNREAD);
        assert_eq!(node.role(), Role::CheckBox);
        assert_eq!(node.label(), Some("Solo stanze non lette"));
        assert_eq!(
            node.toggled(),
            Some(if checked {
                Toggled::True
            } else {
                Toggled::False
            })
        );
        assert_eq!(node.bounds(), Some(bounds));
        assert!(node.supports_action(Action::Click));
        assert!(node.supports_action(Action::Focus));
    }
}
