#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Role, Toggled};
use matrix_desktop::Sidebar;
use matrix_desktop::accessibility::details::{DetailsAction, DetailsControl, DetailsSnapshot, DetailsView};
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
        disconnected: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
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
    let mut info = room.info.clone();
    info.topic.get_or_insert_with(|| "Argomento".into());
    info.edit_rights = matrix_engine::EditRights { name: true, topic: true, avatar: true };
    DetailsSnapshot {
        scope: room.scope.clone(),
        name: room.name.clone(),
        info,
        people: vec![],
        back: false,
        acts: vec![],
        focus: DetailsControl::Close,
        profile_focus: 0,
        bounds: [None; 9],
        hide_status: false,
        hide_redacted: false,
        favourite: false,
        low_priority: false,
        links: vec![],
        confirmation: None,
    }
}

#[test]
fn details_tree_exposes_exact_read_only_room_metadata_and_controls() {
    let mut panel = panel();
    panel.name = "Fixture stanza".into();
    panel.info.topic = Some("Argomento visibile".into());
    panel.info.joined_members = 2;
    panel.info.encrypted = None;
    panel.bounds[5] = Some(accesskit::Rect::new(800., 400., 900., 430.));
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
    let close = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::Close.id()).unwrap().1;
    assert_eq!(close.role(), Role::Button);
    assert_eq!(close.label(), Some("Chiudi informazioni stanza"));
    assert!(close.supports_action(accesskit::Action::Click));
    assert!(close.supports_action(accesskit::Action::Focus));
    let copy = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == DetailsControl::Copy.id())
        .unwrap()
        .1;
    assert_eq!(copy.role(), Role::Button);
    assert_eq!(copy.label(), Some("Copia ID stanza"));
    assert_eq!(copy.bounds(), panel.bounds[5]);
    assert_eq!(panel.next_focus(false), DetailsControl::Favourite);
}

#[test]
fn details_tree_names_encryption_without_jargon() {
    for (state, label) in [(Some(true), "Cifratura attiva"), (Some(false), "Cifratura non attiva")] {
        let mut panel = panel();
        panel.info.encrypted = state;
        assert!(panel.tree().nodes.iter().any(|(_, n)| n.value() == Some(label)));
    }
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

fn navigation(details: matrix_desktop::accessibility::details::DetailsView) -> matrix_desktop::accessibility::NavigationSnapshot {
    use matrix_desktop::accessibility::NavigationSnapshot;
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
fn toggles_and_edit_are_in_the_focus_cycle_and_follow_the_visible_label() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
    let mut panel = panel();
    let scope = panel.scope.clone();
    // Tab reaches every control once, in visual order, and wraps.
    let mut order = vec![panel.focus];
    for _ in 0..DetailsControl::ALL.len() {
        panel.focus = panel.next_focus(false);
        order.push(panel.focus);
    }
    assert_eq!(order[..9], DetailsControl::ALL);
    assert_eq!(order[9], DetailsControl::Close);
    // Shift+Tab is the exact inverse, wrapping from Close to Edit.
    for want in order.iter().rev().skip(1) {
        panel.focus = panel.next_focus(true);
        assert_eq!(panel.focus, *want);
    }
    let ids: std::collections::HashSet<_> = DetailsControl::ALL.iter().map(|c| c.id()).collect();
    assert_eq!(ids.len(), 9);
    panel.people = vec![("Membro: Ada, @ada:x".into(), Some("@ada:x".into()))];
    let label = |panel: &DetailsSnapshot, c: DetailsControl| {
        let tree = panel.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == c.id()).unwrap().1;
        assert!(node.supports_action(Action::Focus) && node.supports_action(Action::Click));
        node.label().unwrap().to_owned()
    };
    assert_eq!(label(&panel, DetailsControl::HideStatus), "Messaggi di stato");
    assert_eq!(label(&panel, DetailsControl::HideRedacted), "Messaggi eliminati");
    assert_eq!(label(&panel, DetailsControl::Edit), "Modifica");
    let toggled = |panel: &DetailsSnapshot, c: DetailsControl| {
        let tree = panel.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == c.id()).unwrap().1;
        assert_eq!(node.role(), Role::Switch);
        node.toggled()
    };
    assert_eq!(toggled(&panel, DetailsControl::HideStatus), Some(Toggled::True));
    panel.hide_status = true;
    panel.hide_redacted = true;
    assert_eq!(toggled(&panel, DetailsControl::HideStatus), Some(Toggled::False));
    assert_eq!(toggled(&panel, DetailsControl::HideRedacted), Some(Toggled::False));
    assert_eq!(label(&panel, DetailsControl::HideStatus), "Messaggi di stato");
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

#[test]
fn topic_is_a_labelled_focusable_scroll_area_without_help_text_or_click() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::details::{DetailsAction, DetailsView};
    let panel = panel();
    let scope = panel.scope.clone();
    let tree = panel.tree();
    let node = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::Topic.id()).unwrap().1;
    assert_eq!((node.role(), node.label()), (Role::ScrollView, Some("Argomento")));
    assert!(node.supports_action(Action::Focus) && !node.supports_action(Action::Click));
    assert_eq!(node.description(), None);
    let view = DetailsView::Open(Box::new(panel));
    let request = |action| ActionRequest { action, target_tree: TreeId::ROOT, target_node: DetailsControl::Topic.id(), data: None };
    assert_eq!(
        view.action(&request(Action::Focus)),
        Some(DetailsAction::Focus { scope, control: DetailsControl::Topic })
    );
    assert_eq!(view.action(&request(Action::Click)), None);
}

#[test]
fn details_switches_never_rename_and_show_on_when_the_content_is_visible() {
    for (hide_status, hide_redacted) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut p = panel();
        p.hide_status = hide_status;
        p.hide_redacted = hide_redacted;
        let tree = p.tree();
        let sw: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Switch).collect();
        let got: Vec<_> = sw.iter().map(|(_, n)| (n.label().unwrap(), n.toggled())).collect();
        let on = |hide: bool| Some(if hide { Toggled::False } else { Toggled::True });
        assert_eq!(
            got,
            [("Messaggi di stato", on(hide_status)), ("Messaggi eliminati", on(hide_redacted))],
            "hide_status={hide_status} hide_redacted={hide_redacted}"
        );
    }
}

#[test]
fn topic_links_are_link_children_of_the_unchanged_topic_node() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::{
        MessageLinkRequests,
        accessibility::{details::{DetailsAction, DetailsView}, links::Snapshot},
    };
    let mut panel = panel();
    let text = "Vedi https://example.invalid/x";
    panel.info.topic = Some(text.into());
    let target = MessageLinkRequests::default().topic_targets(&panel.scope, text)[0].clone();
    panel.links = vec![Snapshot { target: target.clone(), bounds: None, focused: false, requested: false }];
    let scope = panel.scope.clone();
    let tree = panel.tree();
    let topic = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::Topic.id()).unwrap().1;
    assert_eq!((topic.role(), topic.label(), topic.description()), (Role::ScrollView, Some("Argomento"), None));
    assert!(topic.supports_action(Action::Focus) && !topic.supports_action(Action::Click));
    let link_id = panel.links[0].id();
    assert_eq!(topic.children(), [link_id]);
    let link = &tree.nodes.iter().find(|(id, _)| *id == link_id).unwrap().1;
    assert_eq!((link.role(), link.url()), (Role::Link, Some("https://example.invalid/x")));
    let view = DetailsView::Open(Box::new(panel));
    let request = |action, node| ActionRequest { action, target_tree: TreeId::ROOT, target_node: node, data: None };
    let instance = target.instance();
    assert_eq!(view.action(&request(Action::Click, link_id)), Some(DetailsAction::Link { scope: scope.clone(), instance, focus_only: false }));
    assert_eq!(view.action(&request(Action::Focus, link_id)), Some(DetailsAction::Link { scope, instance, focus_only: true }));
    // A node of a link the panel no longer lists leads nowhere.
    let gone = DetailsView::Open(Box::new(crate::panel()));
    assert_eq!(gone.action(&request(Action::Click, link_id)), None);
}

#[test]
fn without_a_topic_tab_skips_the_hidden_argomento_and_the_tree_has_no_node_for_it() {
    let mut panel = panel();
    panel.info.topic = None;
    assert_eq!(panel.next_focus(false), DetailsControl::Favourite);
    panel.focus = DetailsControl::Notify;
    assert_eq!(panel.next_focus(false), DetailsControl::Copy);
    panel.focus = DetailsControl::Copy;
    assert_eq!(panel.next_focus(true), DetailsControl::Notify);
    panel.info.topic = Some("  ".into());
    assert_eq!(panel.next_focus(true), DetailsControl::Notify);
    // The topic vanished while it had focus: both directions and the tree land on existing controls.
    panel.focus = DetailsControl::Topic;
    assert_eq!(panel.next_focus(false), DetailsControl::Copy);
    assert_eq!(panel.next_focus(true), DetailsControl::Notify);
    let tree = panel.tree();
    assert!(!tree.nodes.iter().any(|(id, _)| *id == DetailsControl::Topic.id()));
    assert_eq!(tree.focus, DetailsControl::Copy.id());
    panel.info.topic = Some("Ciao".into());
    panel.focus = DetailsControl::Notify;
    assert_eq!(panel.next_focus(false), DetailsControl::Topic);
}

#[test]
fn bell_is_named_with_the_mode_and_clicks_to_step_it() {
    use matrix_domain::notifications::RoomNotifyMode;
    for mode in RoomNotifyMode::ALL {
        let mut p = panel();
        p.info.notification_mode = mode;
        let tree = p.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::Notify.id()).unwrap().1;
        assert_eq!((node.role(), node.label()), (accesskit::Role::Button, Some(format!("Notifiche: {}", mode.label()).as_str())));
        let click = accesskit::ActionRequest { action: accesskit::Action::Click, target_tree: accesskit::TreeId::ROOT, target_node: DetailsControl::Notify.id(), data: None };
        assert_eq!(DetailsView::Open(Box::new(p.clone())).action(&click), Some(DetailsAction::Notify(p.scope.clone())));
    }
}

#[test]
fn favourite_star_is_named_for_its_state_and_clicks_to_toggle() {
    for (favourite, name) in [(false, "Fissa in alto"), (true, "Stacca")] {
        let mut p = panel();
        p.favourite = favourite;
        let tree = p.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::Favourite.id()).unwrap().1;
        assert_eq!((node.role(), node.label()), (accesskit::Role::Button, Some(name)));
        assert!(node.supports_action(accesskit::Action::Click) && node.supports_action(accesskit::Action::Focus));
        let click = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: DetailsControl::Favourite.id(),
            data: None,
        };
        let view = DetailsView::Open(Box::new(p.clone()));
        assert_eq!(view.action(&click), Some(DetailsAction::Favourite(p.scope.clone())));
    }
}

#[test]
fn tab_reaches_the_star_right_after_close() {
    let mut p = panel();
    assert_eq!(p.next_focus(false), DetailsControl::Favourite);
    p.focus = DetailsControl::Favourite;
    assert_eq!(p.next_focus(false), DetailsControl::LowPriority);
    assert_eq!(p.next_focus(true), DetailsControl::Close);
    p.focus = DetailsControl::LowPriority;
    assert_eq!(p.next_focus(false), DetailsControl::Notify);
    p.focus = DetailsControl::Notify;
    assert_eq!(p.next_focus(false), DetailsControl::Topic);
    assert_eq!(p.next_focus(true), DetailsControl::LowPriority);
}

#[test]
fn low_priority_icon_is_named_for_its_state_and_clicks_to_toggle() {
    for (low, name) in [(false, "Non a bassa priorità"), (true, "Bassa priorità")] {
        let mut p = panel();
        p.low_priority = low;
        let tree = p.tree();
        let node = &tree.nodes.iter().find(|(id, _)| *id == DetailsControl::LowPriority.id()).unwrap().1;
        assert_eq!((node.role(), node.label()), (accesskit::Role::Button, Some(name)));
        assert!(node.supports_action(accesskit::Action::Click) && node.supports_action(accesskit::Action::Focus));
        let click = accesskit::ActionRequest {
            action: accesskit::Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: DetailsControl::LowPriority.id(),
            data: None,
        };
        let view = DetailsView::Open(Box::new(p.clone()));
        assert_eq!(view.action(&click), Some(DetailsAction::LowPriority(p.scope.clone())));
    }
}

#[test]
fn edit_control_follows_the_rights_in_tree_and_tab_ring() {
    let mut panel = panel();
    panel.info.edit_rights = matrix_engine::EditRights::default();
    let ids: Vec<_> = panel.tree().nodes.iter().map(|(id, _)| *id).collect();
    assert!(!ids.contains(&DetailsControl::Edit.id()), "no ghost node without rights");
    panel.focus = DetailsControl::HideRedacted;
    assert_eq!(panel.next_focus(false), DetailsControl::Close, "Tab skips the hidden control");
    panel.focus = DetailsControl::Close;
    assert_eq!(panel.next_focus(true), DetailsControl::HideRedacted, "Shift+Tab too");
    // One right is enough.
    panel.info.edit_rights.topic = true;
    assert!(panel.tree().nodes.iter().any(|(id, _)| *id == DetailsControl::Edit.id()));
}

#[test]
fn edit_control_appears_and_vanishes_with_rights_at_runtime() {
    let has_edit = |p: &DetailsSnapshot| p.tree().nodes.iter().any(|(id, _)| *id == DetailsControl::Edit.id());
    let mut p = panel();
    p.info.edit_rights = matrix_engine::EditRights::default();
    assert!(!has_edit(&p), "member");
    // promotion with the same open panel: only an avatar right is enough
    p.info.edit_rights = matrix_engine::EditRights { name: false, topic: false, avatar: true };
    assert!(has_edit(&p), "promoted");
    p.focus = DetailsControl::Edit;
    // demotion while «Modifica» has focus: focus falls back to the control before it
    p.info.edit_rights = matrix_engine::EditRights::default();
    assert!(!has_edit(&p), "demoted");
    assert_eq!(p.next_focus(false), DetailsControl::Close);
}
