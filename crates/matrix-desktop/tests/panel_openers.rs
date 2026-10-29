#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, Rect, TreeId};
use matrix_desktop::accessibility::{
    NavigationAction, NavigationSnapshot,
    panel_openers::{Control, Snapshot},
    semantic_session::SemanticSession,
};

#[test]
fn painted_root_openers_offer_focus_and_click_from_the_same_source() {
    for control in Control::ALL {
        let mut navigation = navigation();
        let bounds = Rect::new(10., 20., 80., 44.);
        navigation.openers = vec![Snapshot {
            control,
            source: 17,
            bounds: Some(bounds),
            focused: true,
        }];
        let session = SemanticSession::new(navigation);
        let tree = session.tree();
        let (id, node) = tree
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(control.label()))
            .unwrap();
        assert_eq!(tree.focus, *id);
        assert_eq!(node.bounds(), Some(bounds));
        for (action, focus_only) in [(Action::Focus, true), (Action::Click, false)] {
            assert!(node.supports_action(action));
            let Some(NavigationAction::PanelOpener(intent)) = session.action(&ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: *id,
                data: None,
            }) else {
                panic!("missing source-bound opener action")
            };
            assert_eq!(intent.control, control);
            assert_eq!(intent.source, 17);
            assert_eq!(intent.focus_only, focus_only);
        }
    }
}

fn navigation() -> NavigationSnapshot {
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
fn same_button_paths_retire_when_the_captured_source_changes_without_hidden_projection() {
    for control in Control::ALL {
        let mut navigation = navigation();
        navigation.openers = vec![Snapshot {
            control,
            source: 1,
            bounds: None,
            focused: false,
        }];
        let mut session = SemanticSession::new(navigation.clone());
        let old = session
            .tree()
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(control.label()))
            .unwrap()
            .0;
        navigation.openers[0].source = 2;
        session.update(navigation);
        let new = session
            .tree()
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(control.label()))
            .unwrap()
            .0;
        assert_ne!(
            old, new,
            "same visible button cannot revive a retired source"
        );
        for action in [Action::Focus, Action::Click] {
            let request = ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: old,
                data: None,
            };
            assert!(session.action(&request).is_none());
            assert!(
                session
                    .action(&ActionRequest {
                        target_node: new,
                        ..request
                    })
                    .is_some()
            );
        }
    }
}
