#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::Role;
use matrix_desktop::Sidebar;
use matrix_desktop::accessibility::details::{DetailsControl, DetailsSnapshot};
use matrix_domain::AccountId;

#[test]
fn details_overlay_replaces_chat_and_routes_only_current_sheet_actions() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView, OPEN};
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot, SEARCH};
    let panel = panel();
    let scope = panel.scope.clone();
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
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
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice: None,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
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
    assert!(!tree.nodes.iter().any(|(id, _)| *id == SEARCH));
    let mut request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: SEARCH,
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
        people: vec![],
        back: false,
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
    panel.people = vec![("Membro: Bruno, @bruno:x".into(), None)];
    let tree = panel.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Informazioni stanza"));
    assert_eq!(tree.focus, DetailsControl::Close.id());
    for label in [
        "Fixture stanza",
        "Argomento: Argomento visibile",
        "2 membri",
        "Cifratura non confermata dallo stato SDK",
        panel.scope.room_id.as_str(),
        "Membro: Bruno, @bruno:x",
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

#[test]
fn member_rows_open_the_profile_and_the_profile_leads_back_to_the_list() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{BACK, DetailsAction, DetailsView};
    let mut panel = panel();
    let scope = panel.scope.clone();
    panel.people = vec![
        ("Membro: Ada, @ada:fixture.invalid".into(), Some("@ada:fixture.invalid".into())),
        ("Membro: Bo, @bo:fixture.invalid".into(), Some("@bo:fixture.invalid".into())),
    ];
    let click = |view: &DetailsView, node| {
        view.action(&ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: node, data: None })
    };
    let tree = panel.tree();
    let buttons: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(id, n)| n.role() == Role::Button && panel.people.iter().any(|(l, _)| n.label() == Some(l)) && id.0 >= 33)
        .collect();
    assert_eq!(buttons.len(), 2);
    assert!(!tree.nodes.iter().any(|(id, _)| *id == BACK));
    let view = DetailsView::Open(Box::new(panel.clone()));
    for ((id, _), user) in buttons.iter().zip(["@ada:fixture.invalid", "@bo:fixture.invalid"]) {
        let action = click(&view, *id).unwrap();
        assert_eq!(action, DetailsAction::Profile { scope: scope.clone(), user_id: user.into() });
        assert!(view.allows(&action));
    }
    assert_eq!(click(&view, BACK), None);
    panel.people = vec![("Profilo: Ada, @ada:fixture.invalid".into(), None)];
    panel.back = true;
    let tree = panel.tree();
    let back = &tree.nodes.iter().find(|(id, _)| *id == BACK).unwrap().1;
    assert_eq!((back.role(), back.label()), (Role::Button, Some("Membri")));
    let view = DetailsView::Open(Box::new(panel));
    assert_eq!(click(&view, BACK), Some(DetailsAction::Members(scope)));
    assert!(tree.nodes.iter().filter(|(_, n)| n.role() == Role::Button).count() >= 3);
}

fn navigation(details: matrix_desktop::accessibility::details::DetailsView) -> matrix_desktop::accessibility::NavigationSnapshot {
    use matrix_desktop::accessibility::NavigationSnapshot;
    NavigationSnapshot {
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
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        details,
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
    }
}

#[test]
fn details_panel_is_a_named_complementary_not_a_modal_dialog() {
    let tree = panel().tree();
    let root = &tree.nodes[0].1;
    assert_eq!(root.role(), Role::Complementary);
    assert_eq!(root.label(), Some("Informazioni stanza"));
    for (id, node) in &tree.nodes {
        assert!(!matches!(node.role(), Role::Dialog | Role::AlertDialog | Role::Window), "{id:?} is a {:?}", node.role());
        assert!(!node.is_modal(), "{id:?} is aria-modal");
        assert!(!node.is_hidden(), "{id:?} is hidden");
    }
}

// Defect (card to mazinga): NavigationSnapshot::tree()/action() still return only the panel while it is open, so the
// screen reader loses the list, the search field and the composer. Remove the ignore when the panel is merged into the app tree.
#[test]
#[ignore = "DIFETTO: the semantic tree still replaces the whole app while the panel is open"]
fn open_panel_does_not_hide_the_rest_of_the_app_from_the_screen_reader() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::DetailsView;
    use matrix_desktop::accessibility::SEARCH;
    let open = navigation(DetailsView::Open(Box::new(panel())));
    let tree = open.tree();
    assert!(tree.nodes.iter().any(|(id, _)| *id == SEARCH), "search field must stay in the tree");
    assert!(tree.nodes.iter().any(|(_, n)| n.label() == Some("Informazioni stanza") && n.role() == Role::Complementary), "panel must stay in the tree");
    let request = ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: SEARCH, data: None };
    assert!(open.action(&request).is_some(), "search stays actionable");
}
