#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, TreeId};
use matrix_desktop::accessibility::{NavigationSnapshot, semantic_session::SemanticSession};

fn snapshot(visible: bool) -> NavigationSnapshot {
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
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible,
        query: String::new(),
        search_focused: visible,
        search_bounds: None,
        rooms: vec![],
        history: None,
        list_focused: false,
        composer: None,
    }
}

#[test]
fn theme_handle_retires_when_the_filtered_sidebar_source_changes() {
    let mut view = snapshot(true);
    let mut session = SemanticSession::new(view.clone());
    let theme = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Menu elenco"))
        .unwrap()
        .0;
    view.query = "different eligibility".into();
    view.navigation_revision += 1;
    session.update(view.clone());
    for action in [Action::Focus, Action::Click] {
        assert!(
            session
                .action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: theme,
                    data: None
                })
                .is_none(),
            "retired theme handle must not translate against a replacement sidebar source"
        );
    }
}

#[test]
fn unread_handle_retires_when_the_sidebar_revision_changes() {
    let mut view = snapshot(true);
    view.unread_filter = Some(matrix_desktop::accessibility::filters::UnreadFilter {
        account: matrix_domain::AccountId::new(1),
        revision: 0,
        checked: false,
        focused: false,
        bounds: None,
    });
    let mut session = SemanticSession::new(view.clone());
    let id = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Solo stanze non lette"))
        .unwrap()
        .0;
    view.navigation_revision += 1;
    view.unread_filter.as_mut().unwrap().revision += 1;
    session.update(view.clone());
    for action in [Action::Focus, Action::Click] {
        assert!(
            session
                .action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: id,
                    data: None
                })
                .is_none(),
            "retired native Unread path must not translate fresh intents"
        );
    }
    view.navigation_revision += 1;
    view.unread_filter.as_mut().unwrap().revision += 1;
    session.update(view);
    assert_ne!(
        session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Solo stanze non lette"))
            .unwrap()
            .0,
        id
    );
}

fn composer_snapshot(room: &str) -> NavigationSnapshot {
    let mut result = snapshot(true);
    result.composer = Some(matrix_desktop::accessibility::ComposerSnapshot {
        send_source: Some(1),
        send_focused: false,
        send_bounds: None,
        attachment: None,
        mentions: None,
        recording: None,
        emoji_open: false,
        scope: matrix_domain::RoomScope {
            account_id: matrix_domain::AccountId::new(1),
            room_id: matrix_domain::RoomId::new(room),
        },
        composing: false,
        text: " caffè 🦀\nbozza ".into(),
        anchor: 0,
        cursor: 0,
        focused: false,
        bounds: None,
    });
    result
}

#[test]
fn send_handle_cannot_target_a_replacement_room_with_the_same_draft() {
    let original = composer_snapshot("!first:fixture.invalid");
    let mut session = SemanticSession::new(original.clone());
    let send_id = |session: &SemanticSession| {
        session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Invia messaggio"))
            .unwrap()
            .0
    };
    let old = send_id(&session);
    let request = |id| ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    };
    assert!(session.action(&request(old)).is_some());
    session.update(composer_snapshot("!second:fixture.invalid"));
    assert!(
        session.action(&request(old)).is_none(),
        "old send must not consume another room's identical draft"
    );
    assert!(session.action(&request(send_id(&session))).is_some());
    session.update(original);
    assert!(
        session.action(&request(old)).is_none(),
        "away/back must not revive the old send"
    );
    assert!(session.action(&request(send_id(&session))).is_some());
}

#[test]
fn send_handle_cannot_target_a_replacement_account_with_the_same_room_and_draft() {
    let original = composer_snapshot("!same:fixture.invalid");
    let mut session = SemanticSession::new(original.clone());
    let old = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Invia messaggio"))
        .unwrap()
        .0;
    let mut other = original.clone();
    other.composer.as_mut().unwrap().scope.account_id = matrix_domain::AccountId::new(2);
    session.update(other);
    for action in [Action::Focus, Action::Click] {
        let request = ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: old,
            data: None,
        };
        assert!(session.action(&request).is_none());
    }
    session.update(original);
    for action in [Action::Focus, Action::Click] {
        let request = ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: old,
            data: None,
        };
        assert!(
            session.action(&request).is_none(),
            "identical account away-back must not revive a native Send handle"
        );
    }
}

#[test]
fn send_handle_cannot_revive_after_a_hidden_visit_with_identical_room_and_draft() {
    let original = composer_snapshot("!same:fixture.invalid");
    let mut session = SemanticSession::new(original.clone());
    let send_id = |session: &SemanticSession| {
        session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Invia messaggio"))
            .unwrap()
            .0
    };
    let old = send_id(&session);
    let mut hidden = original.clone();
    hidden.composer.as_mut().unwrap().send_source = None;
    session.update(hidden);
    for action in [Action::Focus, Action::Click] {
        assert!(
            session
                .action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: old,
                    data: None
                })
                .is_none()
        );
    }
    let mut returned = original;
    returned.composer.as_mut().unwrap().send_source = Some(3);
    session.update(returned);
    for action in [Action::Focus, Action::Click] {
        assert!(
            session
                .action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: old,
                    data: None
                })
                .is_none()
        );
        assert!(
            session
                .action(&ActionRequest {
                    action,
                    target_tree: TreeId::ROOT,
                    target_node: send_id(&session),
                    data: None
                })
                .is_some()
        );
    }
}

#[test]
fn send_handle_cannot_target_a_replaced_unicode_draft() {
    let original = composer_snapshot("!first:fixture.invalid");
    let mut session = SemanticSession::new(original.clone());
    let send_id = |session: &SemanticSession| {
        session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Invia messaggio"))
            .unwrap()
            .0
    };
    let request = |id| ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    };
    let old = send_id(&session);
    session.update(original.clone());
    assert_eq!(
        send_id(&session),
        old,
        "unchanged paint preserves the source"
    );
    let mut replacement = original.clone();
    replacement.composer.as_mut().unwrap().text = " nuova città 🌍 ".into();
    session.update(replacement);
    assert!(
        session.action(&request(old)).is_none(),
        "retained AT action must not send a replacement draft"
    );
    assert!(session.action(&request(send_id(&session))).is_some());
    session.update(original);
    assert!(session.action(&request(old)).is_none());
}

#[test]
fn current_send_exposes_focus_without_consuming_the_unicode_draft() {
    let snapshot = composer_snapshot("!first:fixture.invalid");
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: matrix_desktop::accessibility::SEND,
        data: None,
    };
    assert!(
        snapshot.action(&request).is_some(),
        "current send must accept Focus independently of Click"
    );
    let tree = snapshot.tree();
    let send = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == matrix_desktop::accessibility::SEND)
        .unwrap()
        .1;
    assert!(send.supports_action(Action::Focus));
    assert_eq!(snapshot.composer.unwrap().text, " caffè 🦀\nbozza ");
}

#[test]
fn send_projection_reports_its_own_painted_bounds_and_focus() {
    let mut snapshot = composer_snapshot("!first:fixture.invalid");
    let bounds = accesskit::Rect::new(900., 650., 1000., 690.);
    let composer = snapshot.composer.as_mut().unwrap();
    composer.send_bounds = Some(bounds);
    composer.send_focused = true;
    let tree = snapshot.tree();
    let send = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == matrix_desktop::accessibility::SEND)
        .unwrap()
        .1;
    assert_eq!(send.bounds(), Some(bounds));
    assert_eq!(tree.focus, matrix_desktop::accessibility::SEND);
}

#[test]
fn translated_send_does_not_match_a_new_visit_with_identical_room_and_body() {
    use matrix_desktop::accessibility::NavigationAction;
    let original = composer_snapshot("!first:fixture.invalid");
    let session = SemanticSession::new(original.clone());
    let id = session
        .tree()
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some("Invia messaggio"))
        .unwrap()
        .0;
    for action in [Action::Focus, Action::Click] {
        let translated = session
            .action(&ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: id,
                data: None,
            })
            .unwrap();
        let (scope, text, source) = match translated {
            NavigationAction::SubmitComposer {
                scope,
                text,
                source,
            }
            | NavigationAction::FocusSend {
                scope,
                text,
                source,
            } => (scope, text, source),
            _ => panic!("Send must translate to source-bound intent"),
        };
        let mut current = original.composer.clone().unwrap();
        assert!(current.matches_submission(&scope, &text, source));
        current.send_source = Some(source + 1);
        assert!(
            !current.matches_submission(&scope, &text, source),
            "translated intent must not revive on identical room/body away-back"
        );
        current.send_source = None;
        assert!(!current.matches_submission(&scope, &text, source));
    }
}

#[test]
fn retired_search_object_is_not_reused_after_hidden_view_returns() {
    let mut session = SemanticSession::new(snapshot(true));
    let search_id = |session: &SemanticSession| {
        session
            .tree()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Cerca conversazioni"))
            .unwrap()
            .0
    };
    let original = search_id(&session);
    session.update(snapshot(true));
    assert_eq!(
        search_id(&session),
        original,
        "continuous visibility preserves identity"
    );
    session.update(snapshot(false));
    session.update(snapshot(true));
    let restored = search_id(&session);
    assert_ne!(
        restored, original,
        "removed AT objects must never be resurrected"
    );
    let request = |id| ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    };
    assert!(session.action(&request(original)).is_none());
    assert!(matches!(
        session.action(&request(restored)),
        Some(matrix_desktop::accessibility::NavigationAction::FocusSearch)
    ));
    let tree = session.tree();
    assert_eq!(tree.focus, restored);
    let ids: std::collections::HashSet<_> = tree.nodes.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids.len(), tree.nodes.len());
    assert!(ids.contains(&tree.tree.as_ref().unwrap().root));
    for (_, node) in &tree.nodes {
        assert!(node.children().iter().all(|child| ids.contains(child)));
        if let Some(selection) = node.text_selection() {
            assert!(ids.contains(&selection.anchor.node));
            assert!(ids.contains(&selection.focus.node));
            assert!(node.children().contains(&selection.anchor.node));
            assert!(node.children().contains(&selection.focus.node));
        }
    }
    let mut malformed = request(restored);
    malformed.data = Some(accesskit::ActionData::Value("not supported".into()));
    assert!(session.action(&malformed).is_none());
}
