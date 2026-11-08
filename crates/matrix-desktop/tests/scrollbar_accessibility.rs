#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use matrix_desktop::accessibility::scrollbars::{Pane, ScrollbarSnapshot};

#[test]
fn navigation_routes_scrollbar_value_before_rejecting_other_action_data() {
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot};
    let bar = ScrollbarSnapshot {
        pane: Pane::Rooms(vec![]),
        fraction: 0.,
        step: 0.1,
        bounds: accesskit::Rect::new(100., 20., 114., 620.),
    };
    let snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![bar.clone()],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        unread_filter: None,
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
    assert!(
        snapshot
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::ScrollBar)
    );
    let request = ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: bar.node().0,
        data: Some(ActionData::NumericValue(0.8)),
    };
    assert!(
        matches!(snapshot.action(&request), Some(NavigationAction::Scrollbar(a)) if a.fraction == 0.8)
    );
    for data in [
        Some(ActionData::NumericValue(f64::NAN)),
        Some(ActionData::Value("1".into())),
        None,
    ] {
        let mut invalid = request.clone();
        invalid.data = data;
        assert!(snapshot.action(&invalid).is_none());
    }
    let mut hidden = snapshot.clone();
    hidden.visible = false;
    assert!(hidden.action(&request).is_none());
    assert!(
        !hidden
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.role() == Role::ScrollBar)
    );
    let mut session =
        matrix_desktop::accessibility::semantic_session::SemanticSession::new(snapshot.clone());
    let id = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::ScrollBar)
        .unwrap()
        .0;
    session.update(hidden);
    session.update(snapshot.clone());
    let mut retired = request;
    retired.target_node = id;
    assert!(session.action(&retired).is_none());
    assert_ne!(
        session
            .tree()
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::ScrollBar)
            .unwrap()
            .0,
        id
    );
    let prior = session
        .tree()
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::ScrollBar)
        .unwrap()
        .0;
    let mut changed = snapshot;
    changed.scrollbars[0].pane = Pane::Rooms(vec![matrix_domain::RoomScope {
        account_id: matrix_domain::AccountId::new(18),
        room_id: matrix_domain::RoomId::new("!other:local.invalid"),
    }]);
    session.update(changed);
    retired.target_node = prior;
    assert!(
        session.action(&retired).is_none(),
        "an old scrollbar must not target a new filter or conversation"
    );
}

#[test]
fn scrollbar_exposes_bounded_vertical_value_and_exact_target() {
    let view = ScrollbarSnapshot {
        pane: Pane::Rooms(vec![]),
        fraction: 0.25,
        step: 0.1,
        bounds: accesskit::Rect::new(100., 20., 114., 620.),
    };
    let (id, node) = view.node();
    assert_eq!(node.role(), Role::ScrollBar);
    assert_eq!(node.label(), Some("Scorrimento conversazioni"));
    assert_eq!(node.numeric_value(), Some(0.25));
    assert_eq!(node.min_numeric_value(), Some(0.));
    assert_eq!(node.max_numeric_value(), Some(1.));
    let action = view
        .action(&ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: id,
            data: Some(ActionData::NumericValue(1.)),
        })
        .unwrap();
    assert_eq!(action.pane, view.pane);
    assert_eq!(action.fraction, 1.);
}
