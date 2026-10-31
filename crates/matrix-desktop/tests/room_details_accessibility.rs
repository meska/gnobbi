#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::Role;
use matrix_desktop::Sidebar;
use matrix_desktop::accessibility::details::{DetailsControl, DetailsSnapshot};
use matrix_domain::AccountId;

#[test]
fn details_overlay_replaces_chat_and_routes_only_current_sheet_actions() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView, OPEN};
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot, THEME};
    let panel = panel();
    let scope = panel.scope.clone();
    let mut snapshot = NavigationSnapshot {
        new_chat: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![matrix_desktop::panel_openers::Snapshot {
            control: matrix_desktop::panel_openers::Control::Info,
            source: 1,
            bounds: None,
            focused: false,
        }],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
        unread_filter: None,
        devices: Default::default(),
        details: DetailsView::Closed(scope.clone()),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: true,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    };
    assert!(snapshot.tree().nodes.iter().any(|(id, _)| *id == OPEN));
    snapshot.details = DetailsView::Open(Box::new(panel));
    let tree = snapshot.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Informazioni stanza"));
    assert!(!tree.nodes.iter().any(|(id, _)| *id == THEME));
    let mut request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: THEME,
        data: None,
    };
    assert!(snapshot.action(&request).is_none());
    request.target_node = DetailsControl::Copy.id();
    assert!(
        matches!(snapshot.action(&request), Some(NavigationAction::Details(DetailsAction::Copy(s))) if s == scope)
    );
}

fn panel() -> DetailsSnapshot {
    let sidebar = Sidebar::demo(AccountId::new(18));
    let room = sidebar.selected().unwrap();
    DetailsSnapshot {
        scope: room.scope.clone(),
        name: room.name.clone(),
        info: room.info.clone(),
        focus: DetailsControl::Close,
        bounds: [None; 2],
    }
}

#[test]
fn details_tree_exposes_exact_read_only_room_metadata_and_controls() {
    let mut panel = panel();
    panel.name = "Fixture stanza".into();
    panel.info.topic = Some("Argomento visibile".into());
    panel.info.joined_members = 2;
    panel.info.encrypted = None;
    panel.bounds[1] = Some(accesskit::Rect::new(800., 400., 900., 430.));
    let tree = panel.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Informazioni stanza"));
    assert_eq!(tree.focus, DetailsControl::Close.id());
    for label in [
        "Fixture stanza",
        "Argomento: Argomento visibile",
        "2 membri",
        "Cifratura non confermata dallo stato SDK",
        panel.scope.room_id.as_str(),
    ] {
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.role() == Role::Label && n.value() == Some(label)),
            "missing readable label value: {label}"
        );
    }
    let copy = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == DetailsControl::Copy.id())
        .unwrap()
        .1;
    assert_eq!(copy.role(), Role::Button);
    assert_eq!(copy.label(), Some("Copia ID stanza"));
    assert_eq!(copy.bounds(), panel.bounds[1]);
    assert_eq!(panel.next_focus(), DetailsControl::Copy);
}

#[test]
fn details_actions_revalidate_room_scope_and_reject_hidden_targets() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView, OPEN};
    let request = |id, action| ActionRequest {
        target_tree: TreeId::ROOT,
        target_node: id,
        action,
        data: None,
    };
    let snapshot = panel();
    let scope = snapshot.scope.clone();
    let closed = DetailsView::Closed(scope.clone());
    let open = DetailsView::Open(Box::new(snapshot));
    assert_eq!(
        closed.action(&request(OPEN, Action::Click)),
        Some(DetailsAction::Open(scope.clone()))
    );
    assert!(
        closed
            .action(&request(DetailsControl::Copy.id(), Action::Click))
            .is_none()
    );
    assert!(
        DetailsView::Unavailable
            .action(&request(OPEN, Action::Click))
            .is_none()
    );
    let copy = open
        .action(&request(DetailsControl::Copy.id(), Action::Click))
        .unwrap();
    assert_eq!(copy, DetailsAction::Copy(scope.clone()));
    assert!(open.allows(&copy));
    assert!(!closed.allows(&copy));
    let mut later = panel();
    later.scope.account_id = AccountId::new(99);
    assert!(
        !DetailsView::Open(Box::new(later)).allows(&copy),
        "queued action must not copy another account's ID"
    );
    assert!(!open.allows(&DetailsAction::Open(scope.clone())));
    assert_eq!(
        open.action(&request(DetailsControl::Close.id(), Action::Focus)),
        Some(DetailsAction::Focus {
            scope: scope.clone(),
            control: DetailsControl::Close
        })
    );
    assert_eq!(
        open.action(&request(DetailsControl::Close.id(), Action::Click)),
        Some(DetailsAction::Close(scope))
    );
    assert!(open.action(&request(OPEN, Action::Click)).is_none());
    let mut malformed = request(DetailsControl::Copy.id(), Action::Click);
    malformed.data = Some(accesskit::ActionData::Value("not accepted".into()));
    assert!(open.action(&malformed).is_none());
}
