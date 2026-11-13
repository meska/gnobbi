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
        unread_filter: Some(UnreadFilter {
            account: AccountId::new(17),
            revision: 0,
            checked: false,
            focused: true,
            bounds: None,
            name: "Non lette".into(),
            buttons: vec![],
        }),
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
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
            room_name: String::new(),
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
        name: "Non lette".into(),
        buttons: vec![],
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
            name: "Non lette".into(),
            buttons: vec![],
        };
        let (id, node) = filter.node();
        assert_eq!(id, UNREAD);
        assert_eq!(node.role(), Role::Switch);
        assert_eq!(node.label(), Some("Non lette"));
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

#[test]
fn every_row_button_has_a_node_with_the_button_name_and_state() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::filters::FilterAction;
    use matrix_desktop::{FilterId, Sidebar};
    let account = AccountId::new(17);
    let mut sidebar = Sidebar::connected(account);
    sidebar.activate(&FilterId::Unread);
    let row = sidebar.filter_buttons();
    let filter = UnreadFilter::from_buttons(account, 0, false, None, &row);
    let nodes = filter.nodes();
    assert_eq!(nodes.len(), row.len());
    let mut ids = std::collections::HashSet::new();
    for (index, b) in row.iter().enumerate() {
        let (id, node) = &nodes[if b.id == FilterId::Unread { 0 } else { nodes.iter().position(|(_, n)| n.label() == Some(b.accessible_name().as_str())).unwrap() }];
        assert!(ids.insert(*id), "duplicate node id");
        assert_eq!(node.label(), Some(b.accessible_name().as_str()));
        assert_eq!(node.toggled(), Some(if b.selected { Toggled::True } else { Toggled::False }), "{}", b.name);
        let expected = match b.id {
            FilterId::Unread => Role::Switch,
            _ => Role::RadioButton,
        };
        assert_eq!(node.role(), expected, "{}", b.name);
        if b.id != FilterId::Unread {
            let click = ActionRequest { action: Action::Click, target_node: *id, target_tree: TreeId::ROOT, data: None };
            let action = filter.action(&click).unwrap();
            assert_eq!(action, FilterAction::Activate { account, revision: 0, index });
            assert!(filter.allows(&action));
        }
    }
}
