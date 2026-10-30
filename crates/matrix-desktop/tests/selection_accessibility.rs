#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationSnapshot},
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn snapshot() -> NavigationSnapshot {
    let history = DemoHistory::new(
        RoomScope {
            account_id: AccountId::new(64),
            room_id: RoomId::new("!selection:test"),
        },
        20,
    );
    NavigationSnapshot {
        title: "Gnobbi".into(),
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
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(HistoryNodes::default().project(&history, 0, true)),
        list_focused: false,
        composer: None,
    }
}

#[test]
fn server_stamp_has_distinct_named_focusable_exact_id_dispatch() {
    use matrix_desktop::{
        EventIdCopies,
        accessibility::{NavigationAction, event_id_copy::Snapshot},
    };
    let mut current = snapshot();
    let m = &mut current.history.as_mut().unwrap().messages[0];
    let target = EventIdCopies::default().target(&m.event, &m.text).unwrap();
    let rect = accesskit::Rect::new(400., 200., 650., 220.);
    m.event_id_copy = Some(Snapshot {
        target: target.clone(),
        focused: true,
        bounds: Some(rect),
    });
    let tree = current.tree();
    let (id, button) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Copia ID evento server"))
        .expect("server stamp needs its own action");
    assert_eq!(button.role(), Role::Button);
    assert_eq!(button.bounds(), Some(rect));
    assert_eq!(tree.focus, *id);
    for action in [Action::Focus, Action::Click] {
        assert!(button.supports_action(action));
        let request = accesskit::ActionRequest {
            target_tree: accesskit::TreeId::ROOT,
            target_node: *id,
            action,
            data: None,
        };
        let Some(NavigationAction::CopyEventId(intent)) = current.action(&request) else {
            panic!("scoped copy dispatch required")
        };
        assert_eq!(intent.target.instance(), target.instance());
        assert_eq!(intent.target.event(), target.event());
        assert_eq!(intent.focus_only, action == Action::Focus);
    }
    current.history.as_mut().unwrap().mark_offline();
    assert!(
        !current
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Copia ID evento server"))
    );
}

#[test]
fn event_stamp_keyboard_entry_activation_and_exits_use_current_capability() {
    use matrix_desktop::{
        EventIdCopies,
        accessibility::{NavigationAction, event_id_copy::Snapshot},
    };
    let mut current = snapshot();
    let h = current.history.as_mut().unwrap();
    let m = &mut h.messages[0];
    let target = EventIdCopies::default().target(&m.event, &m.text).unwrap();
    let event = m.event.clone();
    m.event_id_copy = Some(Snapshot {
        target: target.clone(),
        focused: false,
        bounds: None,
    });
    assert!(
        matches!(h.event_id_key("f10", false), Some(NavigationAction::CopyEventId(i)) if i.focus_only && i.target.instance() == target.instance())
    );
    h.messages[0].event_id_copy.as_mut().unwrap().focused = true;
    for key in ["space", "enter"] {
        assert!(
            matches!(h.event_id_key(key, false), Some(NavigationAction::CopyEventId(i)) if !i.focus_only && i.target.instance() == target.instance())
        );
    }
    for (key, shift) in [("escape", false), ("tab", true)] {
        assert!(
            matches!(h.event_id_key(key, shift), Some(NavigationAction::FocusMessage(id)) if id == event)
        );
    }
    assert!(matches!(
        h.event_id_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    h.focused = false;
    assert!(h.event_id_key("f10", false).is_none());
    h.mark_offline();
    h.focused = true;
    assert!(h.event_id_key("f10", false).is_none());
}

#[test]
fn semantic_stamp_and_translated_intents_retire_across_source_and_scope_returns() {
    use matrix_desktop::{
        EventIdCopies,
        accessibility::{
            NavigationAction, event_id_copy::Snapshot, semantic_session::SemanticSession,
        },
    };
    for change in ["body", "event", "room", "account", "hidden"] {
        for action in [Action::Focus, Action::Click] {
            let mut copies = EventIdCopies::default();
            let mut current = snapshot();
            let m = &mut current.history.as_mut().unwrap().messages[0];
            let target = copies.target(&m.event, &m.text).unwrap();
            let event = m.event.clone();
            let text = m.text.clone();
            m.event_id_copy = Some(Snapshot {
                target: target.clone(),
                focused: true,
                bounds: None,
            });
            let mut session = SemanticSession::new(current.clone());
            let node = session
                .tree()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Copia ID evento server"))
                .unwrap()
                .0;
            let request = accesskit::ActionRequest {
                target_tree: accesskit::TreeId::ROOT,
                target_node: node,
                action,
                data: None,
            };
            let Some(NavigationAction::CopyEventId(queued)) = session.action(&request) else {
                panic!("current source")
            };
            let mut away = current.clone();
            let m = &mut away.history.as_mut().unwrap().messages[0];
            match change {
                "body" => m.text = "changed".into(),
                "event" => {
                    m.event.id = matrix_domain::TimelineId::Server(matrix_domain::EventId::new(
                        "$replacement",
                    ))
                }
                "room" => m.event.scope.room_id = RoomId::new("!replacement:test"),
                "account" => m.event.scope.account_id = AccountId::new(81),
                "hidden" => copies.clear(),
                _ => unreachable!(),
            }
            copies.retain_current([(&m.event, m.text.as_str())]);
            m.event_id_copy = Some(Snapshot {
                target: copies.target(&m.event, &m.text).unwrap(),
                focused: true,
                bounds: None,
            });
            if change == "hidden" {
                away.history = None;
            }
            session.update(away);
            assert!(session.action(&request).is_none());
            assert!(copies.resolve(&queued.target).is_none());
            copies.retain_current([(&event, text.as_str())]);
            let returned = copies.target(&event, &text).unwrap();
            current.history.as_mut().unwrap().messages[0]
                .event_id_copy
                .as_mut()
                .unwrap()
                .target = returned.clone();
            session.update(current);
            assert!(session.action(&request).is_none());
            assert!(copies.resolve(&queued.target).is_none());
            assert!(copies.resolve(&returned).is_some());
            let mut malformed = request.clone();
            malformed.target_node = session
                .tree()
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Copia ID evento server"))
                .unwrap()
                .0;
            malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
            assert!(session.action(&malformed).is_none());
        }
    }
}

#[test]
fn selection_entry_dispatch_captures_exact_body_and_scope() {
    let current = snapshot();
    let tree = current.tree();
    let message = &current.history.as_ref().unwrap().messages[0];
    let id = *tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Seleziona testo messaggio"))
        .map(|(id, _)| id)
        .unwrap();
    let request = accesskit::ActionRequest {
        target_tree: accesskit::TreeId::ROOT,
        target_node: id,
        action: Action::Click,
        data: None,
    };
    let Some(matrix_desktop::accessibility::NavigationAction::MessageSelection(intent)) =
        current.action(&request)
    else {
        panic!("selection needs scoped dispatch")
    };
    assert_eq!(intent.target, message.event);
    assert_eq!(intent.text, message.text);
    assert_eq!(
        intent.control,
        matrix_desktop::accessibility::selection::Control::Select
    );
    assert!(!intent.focus_only);
    let mut hidden = current.clone();
    hidden.history = None;
    assert!(hidden.action(&request).is_none());
    let mut malformed = request;
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(current.action(&malformed).is_none());
}

#[test]
fn selected_body_exposes_copy_cancel_and_selected_state() {
    use matrix_desktop::accessibility::{NavigationAction, selection::Control};
    let mut current = snapshot();
    let message = &mut current.history.as_mut().unwrap().messages[0];
    message.selection.selected = true;
    message.selection.focused = Some(Control::Copy);
    let rect = accesskit::Rect::new(600., 200., 720., 224.);
    message.selection.bounds[1] = Some(rect);
    let event = message.event.clone();
    let parent_id = message.id;
    let tree = current.tree();
    assert!(
        tree.nodes
            .iter()
            .find(|(id, _)| *id == parent_id)
            .unwrap()
            .1
            .is_selected()
            .unwrap()
    );
    for (control, label) in [
        (Control::Copy, "Copia testo selezionato"),
        (Control::Cancel, "Annulla selezione testo"),
    ] {
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .unwrap();
        assert_eq!(node.role(), Role::Button);
        let request = accesskit::ActionRequest {
            target_tree: accesskit::TreeId::ROOT,
            target_node: *id,
            action: Action::Click,
            data: None,
        };
        assert!(
            matches!(current.action(&request), Some(NavigationAction::MessageSelection(intent)) if intent.target == event && intent.control == control)
        );
        if control == Control::Copy {
            assert_eq!(tree.focus, *id);
            assert_eq!(node.bounds(), Some(rect));
        }
    }
    current.history.as_mut().unwrap().messages[0]
        .selection
        .selected = false;
    assert!(
        !current
            .tree()
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Copia testo selezionato"))
    );
}

#[test]
fn keyboard_selection_ring_enters_without_mouse_and_exits_to_live_controls() {
    use matrix_desktop::accessibility::{NavigationAction, selection::Control};
    let mut current = snapshot();
    let h = current.history.as_mut().unwrap();
    let event = h.messages[0].event.clone();
    assert!(
        matches!(h.selection_key("f8", false), Some(NavigationAction::MessageSelection(i)) if i.control == Control::Select && !i.focus_only && i.target == event)
    );
    h.messages[0].selection.selected = true;
    h.messages[0].selection.focused = Some(Control::Copy);
    assert!(
        matches!(h.selection_key("tab", false), Some(NavigationAction::MessageSelection(i)) if i.control == Control::Cancel && i.focus_only)
    );
    assert!(
        matches!(h.selection_key("tab", true), Some(NavigationAction::MessageSelection(i)) if i.control == Control::Select && i.focus_only)
    );
    for key in ["space", "enter"] {
        assert!(
            matches!(h.selection_key(key, false), Some(NavigationAction::MessageSelection(i)) if i.control == Control::Copy && !i.focus_only)
        );
    }
    assert!(
        matches!(h.selection_key("escape", false), Some(NavigationAction::MessageSelection(i)) if i.control == Control::Cancel && !i.focus_only)
    );
    h.messages[0].selection.focused = Some(Control::Cancel);
    assert!(matches!(
        h.selection_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    h.messages[0].selection.focused = Some(Control::Select);
    assert!(
        matches!(h.selection_key("tab", true), Some(NavigationAction::FocusMessage(id)) if id == event)
    );
    h.focused = false;
    assert!(h.selection_key("f8", false).is_none());
}

#[test]
fn unicode_subrange_description_exposes_exact_text_and_empty_caret() {
    let mut current = snapshot();
    let message = &mut current.history.as_mut().unwrap().messages[0];
    let id = message.id;
    message.selection.selected = true;
    message.selection.text = Some("caffè 🙂\n日本語".into());
    let tree = current.tree();
    let node = &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert!(
        node.description()
            .unwrap()
            .ends_with("Testo selezionato: caffè 🙂\n日本語")
    );
    assert_eq!(node.is_selected(), Some(true));
    let message = &mut current.history.as_mut().unwrap().messages[0];
    message.selection.selected = false;
    message.selection.text = Some(String::new());
    let tree = current.tree();
    let node = &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert!(node.description().unwrap().ends_with("Selezione vuota"));
    assert_eq!(node.is_selected(), Some(false));
}

#[test]
fn replaced_body_retires_selection_handles_without_retiring_the_message() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let current = snapshot();
    let mut session = SemanticSession::new(current.clone());
    let tree = session.tree();
    let entry = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Seleziona testo messaggio"))
        .unwrap()
        .0;
    let parent = tree
        .nodes
        .iter()
        .find(|(_, n)| n.children().contains(&entry))
        .unwrap()
        .0;
    let request = accesskit::ActionRequest {
        target_tree: accesskit::TreeId::ROOT,
        target_node: entry,
        action: Action::Click,
        data: None,
    };
    assert!(session.action(&request).is_some());
    let mut replacement = current.clone();
    replacement.history.as_mut().unwrap().messages[0].text = "nuovo caffè 🙂\nriga".into();
    session.update(replacement);
    assert!(
        session.action(&request).is_none(),
        "old body handle must not select replacement text"
    );
    assert!(session.tree().nodes.iter().any(|(id, _)| *id == parent));
    session.update(current);
    assert!(
        session.action(&request).is_none(),
        "restoring the body must not revive its retired handle"
    );
}

#[test]
fn selection_generation_retires_handles_but_range_motion_keeps_them() {
    use matrix_desktop::accessibility::semantic_session::SemanticSession;
    let mut current = snapshot();
    current.history.as_mut().unwrap().messages[0]
        .selection
        .selected = true;
    current.history.as_mut().unwrap().messages[0]
        .selection
        .generation = 100;
    let mut session = SemanticSession::new(current.clone());
    let requests: Vec<_> = session
        .tree()
        .nodes
        .iter()
        .filter(|(_, n)| {
            [
                "Seleziona testo messaggio",
                "Copia testo selezionato",
                "Annulla selezione testo",
            ]
            .contains(&n.label().unwrap_or(""))
        })
        .take(3)
        .map(|(id, _)| accesskit::ActionRequest {
            target_tree: accesskit::TreeId::ROOT,
            target_node: *id,
            action: Action::Click,
            data: None,
        })
        .collect();
    current.history.as_mut().unwrap().messages[0].selection.text = Some("è🙂".into());
    session.update(current.clone());
    for r in &requests {
        assert!(session.action(r).is_some());
    }
    current.history.as_mut().unwrap().messages[0]
        .selection
        .generation = 101;
    session.update(current);
    for r in &requests {
        assert!(
            session.action(r).is_none(),
            "reopened instance must retire all previous controls"
        );
    }
}

#[test]
fn queued_selection_commands_do_not_apply_to_reopened_or_foreign_instances() {
    use matrix_desktop::{
        TextSelection,
        accessibility::selection::{Control, Intent},
    };
    let current = snapshot();
    let message = &current.history.as_ref().unwrap().messages[0];
    let mut selection = TextSelection::default();
    let entry = Intent {
        target: message.event.clone(),
        text: message.text.clone(),
        generation: selection.generation(),
        control: Control::Select,
        focus_only: false,
    };
    assert!(entry.is_current(&selection));
    selection.begin(entry.target.clone(), &entry.text, 0);
    selection.extend(&entry.target, &entry.text, entry.text.len());
    assert!(!entry.is_current(&selection));
    for control in Control::ALL {
        let mut intent = entry.clone();
        intent.control = control;
        intent.generation = selection.generation();
        assert!(intent.is_current(&selection));
        selection.clear();
        selection.begin(entry.target.clone(), &entry.text, 0);
        selection.extend(&entry.target, &entry.text, entry.text.len());
        assert!(!intent.is_current(&selection));
        assert!(!intent.is_current(&TextSelection::default()));
    }
}

#[test]
fn queued_selection_scope_matrix_never_revives_after_away_back() {
    use matrix_desktop::{
        TextSelection,
        accessibility::{
            NavigationAction,
            selection::{Control, Intent},
            semantic_session::SemanticSession,
        },
    };
    // Same event/body in each scope deliberately removes accidental text/ID protection.
    for change_account in [false, true] {
        for focus_only in [false, true] {
            for control in Control::ALL {
                let mut current = snapshot();
                let message = &mut current.history.as_mut().unwrap().messages[0];
                message.text = "caffè 🙂\n日本語".into();
                let target = message.event.clone();
                let body = message.text.clone();
                let mut saved_selection = TextSelection::default();
                saved_selection.begin(target.clone(), &body, 0);
                saved_selection.extend(&target, &body, body.len());
                message.selection.selected = true;
                message.selection.focused = Some(control);
                message.selection.generation = saved_selection.generation();
                let Some(NavigationAction::MessageSelection(queued_key)) = current
                    .history
                    .as_ref()
                    .unwrap()
                    .selection_key("enter", false)
                else {
                    panic!("keyboard control must dispatch")
                };
                assert!(queued_key.is_current(&saved_selection));
                let mut session = SemanticSession::new(current.clone());
                let node = session
                    .tree()
                    .nodes
                    .into_iter()
                    .find(|(_, n)| n.label() == Some(control.label()))
                    .unwrap()
                    .0;
                let request = accesskit::ActionRequest {
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: node,
                    action: if focus_only {
                        Action::Focus
                    } else {
                        Action::Click
                    },
                    data: None,
                };
                let Some(NavigationAction::MessageSelection(queued)) = session.action(&request)
                else {
                    panic!("live control must dispatch")
                };
                assert!(queued.is_current(&saved_selection));
                let mut away = current.clone();
                let away_message = &mut away.history.as_mut().unwrap().messages[0];
                if change_account {
                    away_message.event.scope.account_id = AccountId::new(67);
                } else {
                    away_message.event.scope.room_id = RoomId::new("!other:test");
                }
                let mut foreign = TextSelection::default();
                foreign.begin(away_message.event.clone(), &body, 0);
                foreign.extend(&away_message.event, &body, body.len());
                away_message.selection.generation = foreign.generation();
                session.update(away);
                assert!(session.action(&request).is_none());
                assert!(!queued.is_current(&foreign));
                assert!(!queued_key.is_current(&foreign));
                saved_selection.clear(); // TimelineView::retire_selection on saved-room departure.
                saved_selection.retain_current([(&target, body.as_str())]);
                assert!(!saved_selection.is_active(&target, &body));
                assert!(!queued.is_current(&saved_selection));
                saved_selection.begin(target.clone(), &body, 0);
                saved_selection.extend(&target, &body, body.len());
                current.history.as_mut().unwrap().messages[0]
                    .selection
                    .generation = saved_selection.generation();
                session.update(current);
                assert!(session.action(&request).is_none());
                assert!(!queued.is_current(&saved_selection));
                assert!(!queued_key.is_current(&saved_selection));
                assert_eq!(saved_selection.copy(&target, &body), Some(body.as_str()));
                let fresh = Intent {
                    generation: saved_selection.generation(),
                    ..queued
                };
                assert!(fresh.is_current(&saved_selection));
            }
        }
    }
}

#[test]
fn message_body_exports_named_selection_entry() {
    let current = snapshot();
    let tree = current.tree();
    let message = &current.history.as_ref().unwrap().messages[0];
    let parent = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == message.id)
        .unwrap()
        .1;
    let (_, entry) = tree
        .nodes
        .iter()
        .find(|(id, node)| {
            parent.children().contains(id) && node.label() == Some("Seleziona testo messaggio")
        })
        .expect("message body needs a named selection entry");
    assert_eq!(entry.role(), Role::Button);
    assert!(entry.supports_action(Action::Click));
    assert!(entry.supports_action(Action::Focus));
    assert!(entry.bounds().is_none());
}
