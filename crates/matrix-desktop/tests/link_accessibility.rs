#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::{
    DemoHistory, MessageLinkRequests,
    accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn snapshot() -> NavigationSnapshot {
    NavigationSnapshot {
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        theme_focused: false,
        theme_bounds: None,
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
        history: Some(HistoryNodes::default().project(
            &DemoHistory::new(
                RoomScope {
                    account_id: AccountId::new(67),
                    room_id: RoomId::new("!links:test"),
                },
                5,
            ),
            4,
            true,
        )),
        list_focused: false,
        composer: None,
    }
}

#[test]
fn confirmation_exports_literal_address_cancel_focus_and_instance_bound_actions() {
    use matrix_desktop::{
        LinkConfirmation,
        accessibility::links::{ConfirmationControl, ConfirmationSnapshot},
    };
    let mut snapshot = snapshot();
    let history = snapshot.history.as_mut().unwrap();
    let message = &history.messages[0];
    let mut sources = MessageLinkRequests::default();
    let target = sources.targets(&message.event, &message.text).remove(0);
    let mut confirmation = LinkConfirmation::default();
    assert!(sources.request(&target, &mut confirmation));
    let request = confirmation.capture().unwrap();
    let bounds = accesskit::Rect::new(500., 600., 620., 640.);
    history.confirmation = Some(ConfirmationSnapshot {
        request: request.clone(),
        focused: Some(ConfirmationControl::Cancel),
        bounds: [Some(bounds); 3],
    });
    let tree = snapshot.tree();
    let address = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Label && n.value() == Some(request.url().as_str()))
        .expect("literal validated address");
    assert_eq!(
        address.1.description(),
        Some("Conferma apertura link richiesta")
    );
    for (control, label) in [
        (ConfirmationControl::Cancel, "Annulla apertura link"),
        (ConfirmationControl::Open, "Apri link nel browser esterno"),
    ] {
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .expect("named confirmation control");
        assert_eq!(node.role(), Role::Button);
        assert_eq!(node.bounds(), Some(bounds));
        if control == ConfirmationControl::Cancel {
            assert_eq!(tree.focus, *id);
        }
        for action in [Action::Focus, Action::Click] {
            let Some(NavigationAction::LinkConfirmation(intent)) =
                snapshot.action(&accesskit::ActionRequest {
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: *id,
                    action,
                    data: None,
                })
            else {
                panic!("confirmation intent")
            };
            assert_eq!(intent.request.generation(), request.generation());
            assert_eq!(intent.control, control);
            assert_eq!(intent.focus_only, action == Action::Focus);
        }
    }
}

#[test]
fn confirmation_keyboard_cycles_safe_default_and_carries_captured_instance() {
    use matrix_desktop::{
        LinkConfirmation,
        accessibility::links::{ConfirmationControl, ConfirmationSnapshot},
    };
    let mut snapshot = snapshot();
    let history = snapshot.history.as_mut().unwrap();
    let message = &history.messages[0];
    let mut sources = MessageLinkRequests::default();
    let target = sources.targets(&message.event, &message.text).remove(0);
    let mut confirmation = LinkConfirmation::default();
    sources.request(&target, &mut confirmation);
    let request = confirmation.capture().unwrap();
    let mut c = ConfirmationSnapshot {
        request: request.clone(),
        focused: Some(ConfirmationControl::Cancel),
        bounds: [None; 3],
    };
    for key in ["space", "enter", "escape"] {
        let intent = c.key(key, false).unwrap();
        assert_eq!(intent.control, ConfirmationControl::Cancel);
        assert!(!intent.focus_only);
        assert_eq!(intent.request.generation(), request.generation());
    }
    for shift in [true, false] {
        let intent = c.key("tab", shift).unwrap();
        assert_eq!(intent.control, ConfirmationControl::Open);
        assert!(intent.focus_only);
    }
    c.focused = Some(ConfirmationControl::Open);
    for key in ["space", "enter"] {
        let intent = c.key(key, false).unwrap();
        assert_eq!(intent.control, ConfirmationControl::Open);
        assert!(!intent.focus_only);
    }
    assert_eq!(
        c.key("escape", false).unwrap().control,
        ConfirmationControl::Cancel
    );
    assert_eq!(
        c.key("tab", true).unwrap().control,
        ConfirmationControl::Cancel
    );
    c.focused = None;
    assert!(c.key("enter", false).is_none());
}

#[test]
fn queued_link_dispatch_rejects_retired_sources_without_mutating_newer_requests() {
    use matrix_desktop::{LinkConfirmation, accessibility::semantic_session::SemanticSession};
    use matrix_domain::{EventId, TimelineId};
    for change in ["body", "message", "room", "account", "removed"] {
        for action in [Action::Focus, Action::Click] {
            let mut current = snapshot();
            let m = &mut current.history.as_mut().unwrap().messages[0];
            let event = m.event.clone();
            let text = m.text.clone();
            let mut sources = MessageLinkRequests::default();
            let old = sources.targets(&event, &text)[0].clone();
            m.links = vec![matrix_desktop::accessibility::links::Snapshot {
                target: old.clone(),
                bounds: None,
                focused: true,
                requested: false,
            }];
            let queued_key = match current.history.as_ref().unwrap().link_key("enter", false) {
                Some(NavigationAction::MessageLink(intent)) => intent,
                _ => panic!("focused link must dispatch keyboard request"),
            };
            let queued_focus = match current.history.as_ref().unwrap().link_key("f9", false) {
                Some(NavigationAction::MessageLink(intent)) => intent,
                _ => panic!("message link must dispatch keyboard focus"),
            };
            let mut session = SemanticSession::new(current.clone());
            let node = session
                .tree()
                .nodes
                .into_iter()
                .find(|(_, n)| n.role() == Role::Link)
                .unwrap()
                .0;
            let request = accesskit::ActionRequest {
                target_tree: accesskit::TreeId::ROOT,
                target_node: node,
                action,
                data: None,
            };
            let Some(NavigationAction::MessageLink(queued)) = session.action(&request) else {
                panic!("live link must dispatch");
            };
            assert_eq!(queued.focus_only, action == Action::Focus);
            let mut confirmation = LinkConfirmation::default();
            assert!(sources.request(&queued_key.target, &mut confirmation));
            let captured_confirmation = confirmation.capture().unwrap();
            let mut away = current.clone();
            let m = &mut away.history.as_mut().unwrap().messages[0];
            match change {
                "body" => m.text.push_str(" replacement https://example.invalid/new"),
                "message" => m.event.id = TimelineId::Server(EventId::new("$other")),
                "room" => m.event.scope.room_id = RoomId::new("!other:test"),
                "account" => m.event.scope.account_id = AccountId::new(69),
                "removed" => sources.clear(),
                _ => unreachable!(),
            }
            let next = sources.targets(&m.event, &m.text)[0].clone();
            assert!(sources.focus(&next));
            m.links[0].target = next.clone();
            session.update(away);
            assert!(
                session.action(&request).is_none(),
                "{change} old semantic handle"
            );
            assert!(
                confirmation
                    .confirm_current(&captured_confirmation, &sources)
                    .is_none()
            );
            assert!(!confirmation.cancel_current(&captured_confirmation, &sources));
            assert!(sources.request(&next, &mut confirmation));
            let next_confirmation = confirmation.capture().unwrap();
            assert!(
                confirmation
                    .confirm_current(&captured_confirmation, &sources)
                    .is_none()
            );
            assert!(!confirmation.cancel_current(&captured_confirmation, &sources));
            assert_eq!(
                confirmation.capture().unwrap().generation(),
                next_confirmation.generation()
            );
            for target in [
                &queued.target,
                &queued_key.target,
                &queued_focus.target,
                &old,
            ] {
                assert!(!sources.focus(target), "{change} queued foreign focus");
                assert!(
                    !sources.request(target, &mut confirmation),
                    "{change} queued foreign request"
                );
                assert!(sources.is_current(&next));
                assert_eq!(
                    confirmation.pending().unwrap().as_str(),
                    next.url().as_str()
                );
            }
            let fresh = sources.targets(&event, &text)[0].clone();
            assert!(sources.focus(&fresh));
            current.history.as_mut().unwrap().messages[0].links[0].target = fresh.clone();
            session.update(current);
            assert!(
                session.action(&request).is_none(),
                "{change} returned handle"
            );
            assert!(sources.request(&fresh, &mut confirmation));
            let fresh_confirmation = confirmation.capture().unwrap();
            assert!(
                confirmation
                    .confirm_current(&captured_confirmation, &sources)
                    .is_none()
            );
            assert!(!confirmation.cancel_current(&captured_confirmation, &sources));
            assert!(
                confirmation
                    .confirm_current(&next_confirmation, &sources)
                    .is_none()
            );
            assert!(!confirmation.cancel_current(&next_confirmation, &sources));
            for target in [
                &queued.target,
                &queued_key.target,
                &queued_focus.target,
                &old,
            ] {
                assert!(!sources.focus(target), "{change} queued focus");
                assert!(
                    !sources.request(target, &mut confirmation),
                    "{change} queued click/key/mouse"
                );
                assert!(sources.is_current(&fresh));
                assert_eq!(
                    confirmation.pending().unwrap().as_str(),
                    fresh.url().as_str()
                );
            }
            let live = session
                .tree()
                .nodes
                .into_iter()
                .find(|(_, n)| n.role() == Role::Link)
                .unwrap()
                .0;
            assert_eq!(
                confirmation
                    .confirm_current(&fresh_confirmation, &sources)
                    .unwrap()
                    .as_str(),
                fresh.url().as_str()
            );
            assert!(
                confirmation
                    .confirm_current(&fresh_confirmation, &sources)
                    .is_none()
            );
            assert!(matches!(
                session.action(&accesskit::ActionRequest {
                    target_node: live,
                    ..request
                }),
                Some(NavigationAction::MessageLink(_))
            ));
        }
    }
}

// Characterize guards: no new permission or UI path is introduced.
#[test]
fn queued_confirmation_semantic_and_keyboard_actions_cannot_consume_newer_requests() {
    use matrix_desktop::{
        LinkConfirmation,
        accessibility::{
            links::{ConfirmationControl, ConfirmationSnapshot},
            semantic_session::SemanticSession,
        },
    };
    use matrix_domain::{EventId, TimelineId};
    for change in [
        "cancel", "consume", "replace", "body", "message", "room", "account", "removed",
    ] {
        for control in [ConfirmationControl::Open, ConfirmationControl::Cancel] {
            let mut current = snapshot();
            let message = &current.history.as_ref().unwrap().messages[0];
            let event = message.event.clone();
            let text = message.text.clone();
            let mut sources = MessageLinkRequests::default();
            let source = sources.targets(&event, &text)[0].clone();
            let mut confirmation = LinkConfirmation::default();
            assert!(sources.request(&source, &mut confirmation));
            let captured = confirmation.capture().unwrap();
            let old = ConfirmationSnapshot {
                request: captured.clone(),
                focused: Some(control),
                bounds: [None; 3],
            };
            current.history.as_mut().unwrap().confirmation = Some(old.clone());
            let mut session = SemanticSession::new(current.clone());
            let label = if control == ConfirmationControl::Open {
                "Apri link nel browser esterno"
            } else {
                "Annulla apertura link"
            };
            let node = session
                .tree()
                .nodes
                .into_iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .0;
            let native_requests: Vec<_> = [Action::Click, Action::Focus]
                .into_iter()
                .map(|action| accesskit::ActionRequest {
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: node,
                    action,
                    data: None,
                })
                .collect();
            let mut queued: Vec<_> = native_requests
                .iter()
                .map(|request| {
                    let Some(NavigationAction::LinkConfirmation(intent)) = session.action(request)
                    else {
                        panic!("current confirmation must dispatch")
                    };
                    intent
                })
                .collect();
            queued.extend(
                ["enter", "space", "escape", "tab"].map(|key| old.key(key, false).unwrap()),
            );
            let mut away = event.clone();
            let mut away_text = text.clone();
            match change {
                "cancel" => assert!(confirmation.cancel_current(&captured, &sources)),
                "consume" => assert_eq!(
                    confirmation
                        .confirm_current(&captured, &sources)
                        .unwrap()
                        .as_str(),
                    source.url().as_str()
                ),
                "replace" => {
                    assert!(sources.request(&source, &mut confirmation));
                }
                "body" => away_text.push_str(" changed"),
                "message" => away.id = TimelineId::Server(EventId::new("$other")),
                "room" => away.scope.room_id = RoomId::new("!other:test"),
                "account" => away.scope.account_id = AccountId::new(73),
                "removed" => sources.clear(),
                _ => unreachable!(),
            }
            let next = sources.targets(&away, &away_text)[0].clone();
            sources.retain_event(&away);
            assert!(sources.request(&next, &mut confirmation));
            current.history.as_mut().unwrap().confirmation = Some(ConfirmationSnapshot {
                request: confirmation.capture().unwrap(),
                ..old.clone()
            });
            session.update(current.clone());
            for request in &native_requests {
                assert!(
                    session.action(request).is_none(),
                    "{change}: retired {control:?}"
                );
            }
            // Return to precisely the same account/room/event/body and URL.
            let fresh = sources.targets(&event, &text)[0].clone();
            sources.retain_event(&event);
            assert!(sources.request(&fresh, &mut confirmation));
            let live = confirmation.capture().unwrap();
            current.history.as_mut().unwrap().confirmation = Some(ConfirmationSnapshot {
                request: live.clone(),
                ..old
            });
            session.update(current);
            for request in &native_requests {
                assert!(
                    session.action(request).is_none(),
                    "{change}: old path revived"
                );
            }
            for intent in queued {
                assert!(
                    !confirmation.cancel_current(&intent.request, &sources),
                    "{change}: stale cancel"
                );
                assert!(
                    confirmation
                        .confirm_current(&intent.request, &sources)
                        .is_none(),
                    "{change}: stale open"
                );
                assert_eq!(
                    confirmation.capture().unwrap().generation(),
                    live.generation()
                );
                assert!(sources.is_current(&fresh));
            }
            let live_node = session
                .tree()
                .nodes
                .into_iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .0;
            let Some(NavigationAction::LinkConfirmation(intent)) =
                session.action(&accesskit::ActionRequest {
                    target_node: live_node,
                    ..native_requests[0].clone()
                })
            else {
                panic!("live reacquired control must dispatch")
            };
            assert_eq!(intent.request.generation(), live.generation());
            match control {
                ConfirmationControl::Open => assert_eq!(
                    confirmation
                        .confirm_current(&intent.request, &sources)
                        .unwrap()
                        .as_str(),
                    fresh.url().as_str()
                ),
                ConfirmationControl::Cancel => {
                    assert!(confirmation.cancel_current(&intent.request, &sources))
                }
            }
            assert!(
                confirmation
                    .confirm_current(&intent.request, &sources)
                    .is_none()
            );
            assert!(!confirmation.cancel_current(&intent.request, &sources));
        }
    }
}

#[test]
fn keyboard_link_entry_cycles_occurrences_and_exits_without_opening() {
    let mut snapshot = snapshot();
    let h = snapshot.history.as_mut().unwrap();
    let m = &mut h.messages[0];
    m.text = "https://example.invalid/a https://example.invalid/a".into();
    let mut sources = MessageLinkRequests::default();
    m.links = sources
        .targets(&m.event, &m.text)
        .into_iter()
        .map(|target| matrix_desktop::accessibility::links::Snapshot {
            target,
            bounds: None,
            focused: false,
            requested: false,
        })
        .collect();
    assert!(
        matches!(h.link_key("f9", false), Some(NavigationAction::MessageLink(i)) if i.focus_only)
    );
    h.messages[0].links[0].focused = true;
    assert!(
        matches!(h.link_key("tab", false), Some(NavigationAction::MessageLink(i)) if i.target.instance() == h.messages[0].links[1].target.instance() && i.focus_only)
    );
    for key in ["enter", "space"] {
        assert!(
            matches!(h.link_key(key, false), Some(NavigationAction::MessageLink(i)) if !i.focus_only)
        );
    }
    assert!(matches!(
        h.link_key("tab", true),
        Some(NavigationAction::FocusMessage(_))
    ));
    h.messages[0].links[0].focused = false;
    h.messages[0].links[1].focused = true;
    assert!(matches!(
        h.link_key("tab", false),
        Some(NavigationAction::FocusComposer)
    ));
    assert!(matches!(
        h.link_key("escape", false),
        Some(NavigationAction::FocusMessage(_))
    ));
    h.focused = false;
    assert!(h.link_key("f9", false).is_none());
}

#[test]
fn link_exports_exact_address_name_focus_bounds_and_scoped_request() {
    let mut snapshot = snapshot();
    let message = &mut snapshot.history.as_mut().unwrap().messages[0];
    let mut sources = MessageLinkRequests::default();
    let target = sources.targets(&message.event, &message.text).remove(0);
    let rect = accesskit::Rect::new(500., 250., 780., 280.);
    message
        .links
        .push(matrix_desktop::accessibility::links::Snapshot {
            target: target.clone(),
            bounds: Some(rect),
            focused: true,
            requested: false,
        });
    let tree = snapshot.tree();
    let (id, node) = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == Role::Link)
        .expect("detected safe URL needs an accessible link");
    assert_eq!(
        node.label(),
        Some(format!("Apri link: {}", target.url().as_str()).as_str())
    );
    assert_eq!(node.url(), Some(target.url().as_str()));
    assert_eq!(node.bounds(), Some(rect));
    assert_eq!(tree.focus, *id);
    for action in [Action::Focus, Action::Click] {
        let request = accesskit::ActionRequest {
            target_tree: accesskit::TreeId::ROOT,
            target_node: *id,
            action,
            data: None,
        };
        let Some(NavigationAction::MessageLink(intent)) = snapshot.action(&request) else {
            panic!("link needs source-bound dispatch")
        };
        assert_eq!(intent.target.event(), target.event());
        assert_eq!(intent.target.text(), target.text());
        assert_eq!(intent.target.range(), target.range());
        assert_eq!(intent.target.instance(), target.instance());
        assert_eq!(intent.focus_only, action == Action::Focus);
    }
}
