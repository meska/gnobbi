#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::DeviceVerification;
use matrix_desktop::accessibility::devices::{DeviceControl, DeviceOverview};
use matrix_domain::AccountId;
use matrix_engine::verification::{VerificationDevice, VerificationEvent};

use matrix_desktop::PopupButton;
use matrix_engine::verification::{VerificationSnapshot, VerificationState};

const EMOJI: [(&str, &str); 7] = [
    ("💡", "Light Bulb"),
    ("🐱", "Cat"),
    ("🦋", "Butterfly"),
    ("🎅", "Santa"),
    ("🎧", "Headphone"),
    ("✏️", "Pencil"),
    ("✂️", "Scissors"),
];

fn snapshot(account: AccountId, state: VerificationState, emoji: bool) -> VerificationSnapshot {
    VerificationSnapshot {
        account_id: account,
        user_id: "@alice:matrix-desktop.test".into(),
        device_id: "FIXTURE_DEVICE".into(),
        flow_id: "fixture-flow-1".into(),
        state,
        device_verified: false,
        sas_emoji: emoji.then(|| EMOJI.iter().map(|(s, n)| (s.to_string(), n.to_string())).collect()),
    }
}

/// A request from the other side opens the popup; then the comparison starts.
fn open_compare(model: &mut DeviceVerification, snapshot: VerificationSnapshot) {
    let mut request = snapshot.clone();
    request.state = VerificationState::Requested;
    model.apply(VerificationEvent::Snapshot(request));
    model.apply(VerificationEvent::Snapshot(snapshot));
}

const COMPARE: VerificationState = VerificationState::Compare { decimals: (1234, 5678, 9012) };

#[test]
fn old_popup_node_handles_cannot_press_a_replaced_flow_or_changed_emoji() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceNodes, DeviceView};
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    let mut ids = DeviceNodes::default();
    let original = snapshot(account, COMPARE, true);
    open_compare(&mut model, original.clone());
    let project = |model: &DeviceVerification| DeviceOverview::project(account, model, DeviceControl::Close, [None; 2]);
    let yes = DeviceControl::Popup(PopupButton::Match);
    let first = ids.scope_panel(project(&model));
    let request = ActionRequest {
        action: Action::Click,
        target_node: first.control_id(yes),
        target_tree: TreeId::ROOT,
        data: None,
    };
    assert!(DeviceView::Open(Box::new(first.clone())).action(&request).is_some());
    assert_eq!(ids.scope_panel(project(&model)).control_id(yes), request.target_node);
    for change in 0..3 {
        let mut next = original.clone();
        match change {
            0 => next.flow_id = "flow-two".into(),
            1 => next.state = VerificationState::Compare { decimals: (1234, 5678, 9013) },
            _ => next.sas_emoji.as_mut().unwrap()[0].0 = "🔥".into(),
        }
        // Returning to an older snapshot must not resurrect an old handle.
        model.apply(VerificationEvent::Snapshot(next));
        let panel = ids.scope_panel(project(&model));
        assert_ne!(panel.control_id(yes), request.target_node);
        assert!(DeviceView::Open(Box::new(panel)).action(&request).is_none());
    }
}

#[test]
fn device_initiation_uses_stable_scoped_ids_and_rejects_removed_or_busy_targets() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceNodes, DeviceView};
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    let a = VerificationDevice {
        user_id: "@alice:matrix-desktop.test".into(),
        device_id: "A".into(),
        verified: false,
        display_name: None,
        last_seen_ts: None,
        last_seen_ip: None,
    };
    let b = VerificationDevice {
        device_id: "B".into(),
        ..a.clone()
    };
    let mut ids = DeviceNodes::default();
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![a.clone(), b.clone()],
    });
    let rows = ids.project(account, &model);
    let a_id = rows[0].id;
    let b_id = rows[1].id;
    assert_ne!(a_id, b_id);
    let make_view = |rows| {
        DeviceView::Open(Box::new(
            DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2])
                .with_devices(rows),
        ))
    };
    let original = make_view(rows);
    let request = ActionRequest {
        action: Action::Click,
        target_node: a_id,
        target_tree: TreeId::ROOT,
        data: None,
    };
    let begin = original
        .action(&request)
        .expect("explicit device initiation missing");
    assert!(
        matches!(&begin, DeviceAction::Begin { account: captured, device, .. } if *captured == account && device.device_id == "A")
    );
    assert!(original.allows(&begin));
    assert!(!DeviceView::Closed(account).allows(&begin));
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![b.clone(), a.clone()],
    });
    let rows = ids.project(account, &model);
    assert_eq!(rows[0].id, b_id);
    assert_eq!(rows[1].id, a_id);
    let view = DeviceView::Open(Box::new(
        DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2])
            .with_devices(rows),
    ));
    assert!(
        view.allows(&begin),
        "reorder must retain exact device identity"
    );
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![b.clone()],
    });
    let view = DeviceView::Open(Box::new(
        DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2])
            .with_devices(ids.project(account, &model)),
    ));
    assert!(!view.allows(&begin));
    assert!(view.action(&request).is_none());
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![a],
    });
    assert_eq!(ids.project(account, &model)[0].id, a_id);
    assert!(model.begin(0));
    let view = DeviceView::Open(Box::new(
        DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2])
            .with_devices(ids.project(account, &model)),
    ));
    assert!(!view.allows(&begin));
    assert!(view.action(&request).is_none());
    let foreign = ids.project(AccountId::new(19), &model);
    assert_ne!(
        foreign[0].id, a_id,
        "account identity must participate in ID allocation"
    );
    assert!(!format!("{begin:?}").contains("@alice"));
}

#[test]
fn popup_ring_starts_on_the_first_button_and_never_leaves_the_popup() {
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![VerificationDevice {
            user_id: "@alice:matrix-desktop.test".into(),
            device_id: "FIXTURE_DEVICE".into(),
            verified: false,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    open_compare(&mut model, snapshot(account, COMPARE, true));
    let panel = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 2]);
    let (yes, no, x) = (PopupButton::Match, PopupButton::NoMatch, PopupButton::Close);
    assert_eq!(panel.controls(), vec![DeviceControl::Popup(yes), DeviceControl::Popup(no), DeviceControl::Popup(x)]);
    assert_eq!(panel.focus, DeviceControl::Popup(yes), "initial focus: first button");
    assert_eq!(panel.tree().focus, panel.control_id(DeviceControl::Popup(yes)));
    assert_eq!(panel.next_control(false), DeviceControl::Popup(no));
    assert_eq!(panel.next_control(true), DeviceControl::Popup(x));
    model.popup_tab(false);
    let panel = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 2]);
    assert_eq!(panel.focus, DeviceControl::Popup(no));
    // Outcome: OK and the X only.
    model.apply(VerificationEvent::Snapshot(snapshot(account, VerificationState::Verified, true)));
    let panel = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 2]);
    assert_eq!(panel.controls(), vec![DeviceControl::Popup(PopupButton::Ok), DeviceControl::Popup(x)]);
    assert_eq!(panel.focus, DeviceControl::Popup(PopupButton::Ok));
}

#[test]
fn popup_presses_are_explicit_and_revalidated_against_the_exact_snapshot() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceView};
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    let original = snapshot(account, COMPARE, true);
    open_compare(&mut model, original.clone());
    let view = |model: &DeviceVerification| {
        DeviceView::Open(Box::new(DeviceOverview::project(account, model, DeviceControl::Close, [None; 2])))
    };
    let confirm = DeviceControl::Popup(PopupButton::Match);
    let request = ActionRequest { target_node: confirm.id(), action: Action::Click, target_tree: TreeId::ROOT, data: None };
    let current = view(&model);
    let action = current.action(&request).expect("the comparison must offer an explicit «Corrispondono»");
    assert!(matches!(&action, DeviceAction::Popup { button: PopupButton::Match, snapshot: Some(captured), .. } if **captured == original));
    assert!(current.allows(&action));
    assert!(!DeviceView::Closed(account).allows(&action));
    assert!(!DeviceView::Unavailable.allows(&action));
    for changed in 0..5 {
        let mut next = original.clone();
        match changed {
            0 => next.flow_id = "fixture-flow-2".into(),
            1 => next.device_id = "OTHER_DEVICE".into(),
            2 => next.user_id = "@bob:matrix-desktop.test".into(),
            3 => next.state = VerificationState::Compare { decimals: (1234, 5678, 9013) },
            _ => next.state = VerificationState::Confirmed,
        }
        model.apply(VerificationEvent::Snapshot(next));
        assert!(!view(&model).allows(&action), "stale press accepted: {changed}");
    }
    model.apply(VerificationEvent::Snapshot(original));
    assert!(model.popup_press(PopupButton::Match));
    assert!(!view(&model).allows(&action), "a pending decision must reject duplicates");
    assert!(view(&model).action(&request).is_none());
    assert!(!format!("{action:?}").contains("fixture-flow"), "diagnostics must redact SAS identities");
}

#[test]
fn popup_is_a_named_dialog_with_readable_emoji_names_numbers_outcome_and_actions() {
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    open_compare(&mut model, snapshot(account, COMPARE, true));
    let tree = DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2]).tree();
    let dialog = tree.nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Dialog).expect("dialog node");
    assert_eq!(dialog.label(), Some("Verifica"));
    // One readable node per emoji, in order, named with the SDK description; the symbol itself is not read twice.
    let labels: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Label).map(|(_, n)| n.value().unwrap()).collect();
    assert_eq!(labels, EMOJI.map(|(_, n)| n));
    let buttons: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Button).map(|(_, n)| (n.label().unwrap(), n.supports_action(Action::Click))).collect();
    assert_eq!(buttons, vec![("Corrispondono", true), ("Non corrispondono", true), ("Chiudi", true)]);
    assert_eq!(dialog.children().len(), 7 + 3);
    // The peer negotiated decimals only: the numbers are read.
    let mut decimals = DeviceVerification::new(account);
    open_compare(&mut decimals, snapshot(account, COMPARE, false));
    let tree = DeviceOverview::project(account, &decimals, DeviceControl::Close, [None; 2]).tree();
    assert!(tree.nodes.iter().any(|(_, n)| n.value() == Some("Numeri SAS: 1234, 5678, 9012")));
    // Outcome and waiting.
    model.apply(VerificationEvent::Snapshot(snapshot(account, VerificationState::Mismatched, true)));
    let tree = DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2]).tree();
    assert!(tree.nodes.iter().any(|(_, n)| n.value() == Some("Gli emoji non corrispondono")));
    assert!(tree.nodes.iter().any(|(_, n)| n.label() == Some("OK")));
    // Closed popup: no dialog at all, the panel tree is back.
    assert!(model.popup_press(PopupButton::Ok));
    let tree = DeviceOverview::project(account, &model, DeviceControl::Close, [None; 2]).tree();
    assert!(!tree.nodes.iter().any(|(_, n)| n.role() == Role::Dialog));
    assert_eq!(tree.nodes[0].1.label(), Some("Verifica dispositivi"));
}

#[test]
fn device_sheet_replaces_chat_tree_and_routes_only_its_controls() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceView, OPEN};
    use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot, SEARCH};
    let account = AccountId::new(18);
    let mut snapshot = NavigationSnapshot {
        title: "Gnobbi".into(),
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![matrix_desktop::panel_openers::Snapshot {
            control: matrix_desktop::panel_openers::Control::Devices,
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
        devices: DeviceView::Closed(account),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: true,
        query: String::new(),
        search_focused: true,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    };
    assert!(snapshot.tree().nodes.iter().any(|(id, _)| *id == OPEN));
    snapshot.devices = DeviceView::Open(Box::new(DeviceOverview::project(
        account,
        &DeviceVerification::new(account),
        DeviceControl::Close,
        [None; 2],
    )));
    let tree = snapshot.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Verifica dispositivi"));
    assert!(!tree.nodes.iter().any(|(id, _)| *id == SEARCH));
    let mut request = ActionRequest {
        target_node: SEARCH,
        action: Action::Click,
        target_tree: TreeId::ROOT,
        data: None,
    };
    assert!(snapshot.action(&request).is_none());
    request.target_node = DeviceControl::Refresh.id();
    assert!(
        matches!(snapshot.action(&request), Some(NavigationAction::Devices(DeviceAction::Refresh(a))) if a == account)
    );
}

#[test]
fn overview_actions_reject_closed_foreign_account_and_malformed_requests() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceView, OPEN};
    let account = AccountId::new(18);
    let request = |target_node, action| ActionRequest {
        target_node,
        action,
        target_tree: TreeId::ROOT,
        data: None,
    };
    let closed = DeviceView::Closed(account);
    assert_eq!(
        closed.action(&request(OPEN, Action::Click)),
        Some(DeviceAction::Open(account))
    );
    assert!(
        closed
            .action(&request(DeviceControl::Refresh.id(), Action::Click))
            .is_none()
    );
    let open = DeviceView::Open(Box::new(DeviceOverview::project(
        account,
        &DeviceVerification::new(account),
        DeviceControl::Close,
        [None; 2],
    )));
    let refresh = DeviceAction::Refresh(account);
    assert_eq!(
        open.action(&request(DeviceControl::Refresh.id(), Action::Click)),
        Some(refresh.clone())
    );
    assert!(open.allows(&refresh));
    assert!(!closed.allows(&refresh));
    assert!(!DeviceView::Unavailable.allows(&refresh));
    assert!(!open.allows(&DeviceAction::Refresh(AccountId::new(19))));
    assert!(!open.allows(&DeviceAction::Open(account)));
    assert!(open.action(&request(OPEN, Action::Click)).is_none());
    assert_eq!(
        open.action(&request(DeviceControl::Refresh.id(), Action::Focus)),
        Some(DeviceAction::Focus {
            account,
            control: DeviceControl::Refresh
        })
    );
    let mut malformed = request(DeviceControl::Refresh.id(), Action::Click);
    malformed.data = Some(accesskit::ActionData::Value("ignored".into()));
    assert!(open.action(&malformed).is_none());
}

#[test]
fn device_overview_reads_sdk_devices_without_offering_unverified_sas_actions() {
    let account = AccountId::new(18);
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![VerificationDevice {
            user_id: "@alice:matrix-desktop.test".into(),
            device_id: "FIXTURE_DEVICE".into(),
            verified: false,
            display_name: None,
            last_seen_ts: None,
            last_seen_ip: None,
        }],
    });
    let panel = DeviceOverview::project(account, &model, DeviceControl::Refresh, [None; 2]);
    let tree = panel.tree();
    assert_eq!(tree.nodes[0].1.label(), Some("Verifica dispositivi"));
    assert_eq!(tree.focus, DeviceControl::Refresh.id());
    for value in [
        "Altri dispositivi",
        "FIXTURE_DEVICE · Non verificato",
    ] {
        assert!(
            tree.nodes
                .iter()
                .any(|(_, node)| node.role() == Role::Label && node.value() == Some(value)),
            "missing readable value: {value}"
        );
    }
    assert!(
        tree.nodes
            .iter()
            .all(|(_, n)| !n.value().is_some_and(|v| v.contains("Avvia solo") || v.contains("canale fidato"))),
        "explanatory copy must not be exposed"
    );
    let controls: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.supports_action(Action::Click))
        .collect();
    assert_eq!(
        controls.len(),
        2,
        "overview must not invent actionable SAS buttons"
    );
    assert!(
        controls
            .iter()
            .any(|(_, n)| n.label() == Some("Aggiorna dispositivi"))
    );
    assert!(
        controls
            .iter()
            .any(|(_, n)| n.label() == Some("Chiudi verifica dispositivi"))
    );
}

#[test]
fn disconnect_buttons_and_password_dialog_are_reachable_and_never_expose_the_password() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceNodes, DeviceView};
    let account = AccountId::new(18);
    let device = |id: &str, name: &str| VerificationDevice {
        user_id: "@alice:matrix-desktop.test".into(),
        device_id: id.into(),
        verified: true,
        display_name: Some(name.into()),
        last_seen_ts: None,
        last_seen_ip: None,
    };
    let mut model = DeviceVerification::new(account);
    model.apply(VerificationEvent::Profile {
        account_id: account,
        user_id: "@alice:matrix-desktop.test".into(),
        display_name: None,
        device_id: Some("OWN".into()),
        avatar: None,
    });
    // The current device is even listed here: it still gets no «Disconnetti».
    model.apply(VerificationEvent::Devices {
        account_id: account,
        devices: vec![device("A", "Telefono"), device("OWN", "Questo")],
    });
    let mut ids = DeviceNodes::default();
    let view = |model: &DeviceVerification, ids: &mut DeviceNodes, focus| {
        let rows = ids.project(account, model);
        DeviceView::Open(Box::new(
            DeviceOverview::project(account, model, focus, [None; 2]).with_devices(rows),
        ))
    };
    let DeviceView::Open(panel) = view(&model, &mut ids, DeviceControl::Close) else { unreachable!() };
    let rows = ids.project(account, &model);
    let delete_a = DeviceControl::Delete(rows[0].id);
    assert_eq!(panel.controls(), vec![DeviceControl::Close, DeviceControl::Refresh, delete_a]);
    let tree = panel.tree();
    let node = &tree.nodes.iter().find(|(id, _)| *id == delete_a.id()).unwrap().1;
    assert_eq!((node.role(), node.label()), (Role::Button, Some("Disconnetti Telefono")));
    assert!(node.supports_action(Action::Click));
    assert!(!tree.nodes.iter().any(|(_, n)| n.label().is_some_and(|l| l.contains("Questo"))));
    // Keyboard ring reaches it.
    assert_eq!(panel.next_control(true), delete_a);
    // Click → same open-dialog request the pointer produces.
    let view_open = DeviceView::Open(panel.clone());
    let click = ActionRequest { action: Action::Click, target_node: delete_a.id(), target_tree: TreeId::ROOT, data: None };
    let action = view_open.action(&click).expect("click maps");
    assert!(matches!(&action, DeviceAction::OpenDelete { device, .. } if device.device_id == "A"));
    assert!(view_open.allows(&action));
    assert!(!view_open.allows(&DeviceAction::OpenDelete { account, device: device("OWN", "Questo") }));
    // Dialog open: Dialog named after the device, password field without value, cancel/confirm, error line.
    assert!(model.open_delete(0));
    let DeviceView::Open(panel) = view(&model, &mut ids, DeviceControl::Close) else { unreachable!() };
    assert_eq!(panel.focus, DeviceControl::DialogField);
    assert_eq!(panel.controls(), vec![DeviceControl::DialogField, DeviceControl::DialogCancel, DeviceControl::DialogConfirm]);
    let tree = panel.tree();
    assert_eq!(tree.focus, DeviceControl::DialogField.id());
    let by = |c: DeviceControl| &tree.nodes.iter().find(|(id, _)| *id == c.id()).unwrap().1;
    let dialog = tree.nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Dialog).unwrap();
    assert_eq!(dialog.label(), Some("Telefono"));
    let field = by(DeviceControl::DialogField);
    assert_eq!((field.role(), field.value(), field.label()), (Role::PasswordInput, None, Some("Password")));
    assert_eq!(by(DeviceControl::DialogCancel).label(), Some("Annulla"));
    assert_eq!(by(DeviceControl::DialogConfirm).label(), Some("Disconnetti"));
    let open = DeviceView::Open(panel);
    let click = |c: DeviceControl| ActionRequest { action: Action::Click, target_node: c.id(), target_tree: TreeId::ROOT, data: None };
    assert!(matches!(open.action(&click(DeviceControl::DialogCancel)), Some(DeviceAction::CancelDelete(a)) if a == account));
    assert!(matches!(open.action(&click(DeviceControl::DialogConfirm)), Some(DeviceAction::SubmitDelete(a)) if a == account));
    assert!(open.action(&click(DeviceControl::DialogField)).is_none());
    assert!(!open.allows(&DeviceAction::OpenDelete { account, device: device("A", "Telefono") }));
}

#[test]
fn rename_button_and_name_dialog_are_reachable() {
    use accesskit::{ActionRequest, TreeId};
    use matrix_desktop::accessibility::devices::{DeviceAction, DeviceNodes, DeviceView};
    let account = AccountId::new(19);
    let mut model = DeviceVerification::new(account);
    let view = |model: &DeviceVerification| {
        let rows = DeviceNodes::default().project(account, model);
        DeviceView::Open(Box::new(DeviceOverview::project(account, model, DeviceControl::Close, [None; 2]).with_devices(rows)))
    };
    let DeviceView::Open(panel) = view(&model) else { unreachable!() };
    assert!(!panel.controls().contains(&DeviceControl::Rename), "own device not known yet");
    model.apply(VerificationEvent::OwnDevice {
        account_id: account,
        device: VerificationDevice {
            user_id: "@alice:matrix-desktop.test".into(),
            device_id: "OWN".into(),
            verified: true,
            display_name: Some("Mac".into()),
            last_seen_ts: None,
            last_seen_ip: None,
        },
    });
    let click = |c: DeviceControl| ActionRequest { action: Action::Click, target_node: c.id(), target_tree: TreeId::ROOT, data: None };
    let open = view(&model);
    let DeviceView::Open(panel) = &open else { unreachable!() };
    assert_eq!(panel.controls(), vec![DeviceControl::Close, DeviceControl::Rename, DeviceControl::Refresh]);
    let tree = panel.tree();
    let node = &tree.nodes.iter().find(|(id, _)| *id == DeviceControl::Rename.id()).unwrap().1;
    assert_eq!((node.role(), node.label()), (Role::Button, Some("Rinomina")));
    assert!(node.supports_action(Action::Click));
    let action = open.action(&click(DeviceControl::Rename)).expect("click maps");
    assert!(matches!(action, DeviceAction::OpenRename(a) if a == account) && open.allows(&action));
    // Dialog: titled «Rinomina», plain text field «Nome», Annulla / Salva.
    assert!(model.open_rename());
    let open = view(&model);
    let DeviceView::Open(panel) = &open else { unreachable!() };
    assert!(!open.allows(&DeviceAction::OpenRename(account)));
    let tree = panel.tree();
    let by = |c: DeviceControl| &tree.nodes.iter().find(|(id, _)| *id == c.id()).unwrap().1;
    let dialog = tree.nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Dialog).unwrap();
    assert_eq!(dialog.label(), Some("Rinomina"));
    assert_eq!((by(DeviceControl::DialogField).role(), by(DeviceControl::DialogField).label()), (Role::TextInput, Some("Nome")));
    assert_eq!(by(DeviceControl::DialogCancel).label(), Some("Annulla"));
    assert_eq!(by(DeviceControl::DialogConfirm).label(), Some("Salva"));
    assert!(matches!(open.action(&click(DeviceControl::DialogConfirm)), Some(DeviceAction::SubmitDelete(a)) if a == account));
}
