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
        room_search: Default::default(),
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
        acts: vec![],
        focus: DetailsControl::Close,
        profile_focus: 0,
        bounds: [None; 5],
        hide_status: false,
        hide_redacted: false,
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

#[test]
fn profile_actions_are_buttons_with_names_and_route_to_act() {
    use matrix_desktop::ProfileAct;
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
    let mut panel = panel();
    panel.people = vec![("Profilo: Ada, @ada:fixture.invalid".into(), None)];
    panel.back = true;
    panel.acts = vec![ProfileAct::CopyId, ProfileAct::Message, ProfileAct::Link, ProfileAct::Quote, ProfileAct::Verify];
    let click = |view: &DetailsView, node| {
        view.action(&accesskit::ActionRequest { action: accesskit::Action::Click, target_tree: accesskit::TreeId::ROOT, target_node: node, data: None })
    };
    let tree = panel.tree();
    let scope = panel.scope.clone();
    let view = DetailsView::Open(Box::new(panel));
    for act in [ProfileAct::CopyId, ProfileAct::Message, ProfileAct::Link, ProfileAct::Quote, ProfileAct::Verify] {
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button && n.label() == Some(act.label()))
            .unwrap_or_else(|| panic!("no button named {}", act.label()));
        assert!(node.supports_action(accesskit::Action::Click));
        let action = click(&view, *id).unwrap();
        assert_eq!(action, DetailsAction::Act { scope: scope.clone(), act });
        assert!(view.allows(&action));
    }
}

#[test]
fn toggles_and_edit_are_in_the_focus_cycle_and_follow_the_visible_label() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
    let mut panel = panel();
    let scope = panel.scope.clone();
    // Tab reaches every control once, in visual order, and wraps.
    let mut order = vec![panel.focus];
    for _ in 0..DetailsControl::ALL.len() {
        panel.focus = panel.next_focus();
        order.push(panel.focus);
    }
    assert_eq!(order[..5], DetailsControl::ALL);
    assert_eq!(order[5], DetailsControl::Close);
    let ids: std::collections::HashSet<_> = DetailsControl::ALL.iter().map(|c| c.id()).collect();
    assert_eq!(ids.len(), 5);
    panel.people = vec![("Membro: Ada, @ada:x".into(), Some("@ada:x".into()))];
    let label = |panel: &DetailsSnapshot, c: DetailsControl| {
        let tree = panel.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == c.id()).unwrap().1;
        assert_eq!(node.role(), Role::Button);
        assert!(node.supports_action(Action::Focus) && node.supports_action(Action::Click));
        node.label().unwrap().to_owned()
    };
    assert_eq!(label(&panel, DetailsControl::HideStatus), "Nascondi messaggi di stato");
    assert_eq!(label(&panel, DetailsControl::HideRedacted), "Nascondi messaggi eliminati");
    assert_eq!(label(&panel, DetailsControl::Edit), "Modifica");
    panel.hide_status = true;
    panel.hide_redacted = true;
    assert_eq!(label(&panel, DetailsControl::HideStatus), "Mostra messaggi di stato");
    assert_eq!(label(&panel, DetailsControl::HideRedacted), "Mostra messaggi eliminati");
    let view = DetailsView::Open(Box::new(panel));
    for (control, want) in [
        (DetailsControl::HideStatus, DetailsAction::HideStatus(scope.clone())),
        (DetailsControl::HideRedacted, DetailsAction::HideRedacted(scope.clone())),
        (DetailsControl::Edit, DetailsAction::Edit(scope.clone())),
    ] {
        let request = ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: control.id(),
            data: None,
        };
        let got = view.action(&request).unwrap();
        assert_eq!(got, want);
        assert!(view.allows(&got));
    }
    // The member row (id 33+) is still a profile click, not a control.
    assert!(matches!(
        view.action(&ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: accesskit::NodeId(33 + 6), data: None }),
        Some(DetailsAction::Profile { .. })
    ));
}
