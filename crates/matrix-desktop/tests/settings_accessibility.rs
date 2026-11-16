#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::accessibility::settings::{SettingsAction, SettingsSnapshot};

fn click(target: accesskit::NodeId, action: Action) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    }
}

#[test]
fn switches_are_labelled_checkboxes_with_state_and_set_explicitly() {
    let snapshot = SettingsSnapshot {
        focus: 2,
        on: [false, true, false, false],
        update_check: None,
        privacy_page: false,
    };
    let tree = snapshot.tree();
    let boxes: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Switch)
        .collect();
    let labels: Vec<_> = boxes.iter().map(|(_, n)| n.label().unwrap()).collect();
    // Read receipts moved to the privacy page: Settings keeps debug, deleted messages, tray.
    assert_eq!(labels, ["Modalità debug", "Messaggi eliminati", "Riduci nel tray alla chiusura"]);
    let toggled: Vec<_> = boxes.iter().map(|(_, n)| n.toggled().unwrap()).collect();
    assert_eq!(toggled, [accesskit::Toggled::False; 3]);
    assert_eq!(tree.focus, boxes[1].0);
    assert_eq!(snapshot.action(&click(boxes[1].0, Action::Click)), Some(SettingsAction::Set(2, true)));
    assert_eq!(snapshot.action(&click(boxes[0].0, Action::Focus)), Some(SettingsAction::Focus(0)));
    // The privacy page exposes «Conferme di lettura» (actionable) and nothing else.
    let page = SettingsSnapshot { privacy_page: true, focus: 1, ..snapshot.clone() };
    let tree = page.tree();
    let boxes: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Switch).collect();
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].1.label(), Some("Conferme di lettura"));
    assert_eq!(boxes[0].1.toggled(), Some(accesskit::Toggled::True));
    assert_eq!(tree.focus, boxes[0].0);
    assert_eq!(page.action(&click(boxes[0].0, Action::Click)), Some(SettingsAction::Set(1, false)));
    assert_eq!(snapshot.action(&click(boxes[0].0, Action::Click)), None, "not actionable from the Settings page");
    assert_eq!(snapshot.action(&click(accesskit::NodeId(9999), Action::Click)), None);
}


#[test]
fn update_switch_is_optional_and_exposes_its_state_and_action() {
    for value in [None, Some(false), Some(true)] {
        let snapshot = SettingsSnapshot { focus: 4, on: [false; 4], update_check: value, privacy_page: false };
        let tree = snapshot.tree();
        let switches: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Switch).collect();
        assert_eq!(switches.len(), if value.is_some() { 4 } else { 3 });
        if let Some(on) = value {
            let (id, node) = switches[3];
            assert_eq!(node.label(), Some(matrix_desktop::update::label::update_check_label()));
            assert_eq!(node.toggled(), Some(if on { accesskit::Toggled::True } else { accesskit::Toggled::False }));
            assert_eq!(tree.focus, *id);
            assert_eq!(snapshot.action(&click(*id, Action::Click)), Some(SettingsAction::Set(4, !on)));
        } else {
            assert_eq!(snapshot.action(&click(accesskit::NodeId(74), Action::Click)), None);
        }
    }
}
