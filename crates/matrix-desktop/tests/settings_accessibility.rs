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
    };
    let tree = snapshot.tree();
    let boxes: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Switch)
        .collect();
    let labels: Vec<_> = boxes.iter().map(|(_, n)| n.label().unwrap()).collect();
    assert_eq!(
        labels,
        [
            "Modalità debug",
            "Conferme di lettura",
            "Messaggi eliminati",
            "Riduci nel tray alla chiusura"
        ]
    );
    let toggled: Vec<_> = boxes.iter().map(|(_, n)| n.toggled().unwrap()).collect();
    assert_eq!(
        toggled,
        [
            accesskit::Toggled::False,
            accesskit::Toggled::True,
            accesskit::Toggled::False,
            accesskit::Toggled::False
        ]
    );
    assert_eq!(tree.focus, boxes[2].0);
    assert_eq!(
        snapshot.action(&click(boxes[1].0, Action::Click)),
        Some(SettingsAction::Set(1, false))
    );
    assert_eq!(
        snapshot.action(&click(boxes[0].0, Action::Focus)),
        Some(SettingsAction::Focus(0))
    );
    assert_eq!(snapshot.action(&click(accesskit::NodeId(9999), Action::Click)), None);
}


#[test]
fn update_switch_is_optional_and_exposes_its_state_and_action() {
    for value in [None, Some(false), Some(true)] {
        let snapshot = SettingsSnapshot { focus: 4, on: [false; 4], update_check: value };
        let tree = snapshot.tree();
        let switches: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Switch).collect();
        assert_eq!(switches.len(), if value.is_some() { 5 } else { 4 });
        if let Some(on) = value {
            let (id, node) = switches[4];
            assert_eq!(node.label(), Some(matrix_desktop::update::label::update_check_label()));
            assert_eq!(node.toggled(), Some(if on { accesskit::Toggled::True } else { accesskit::Toggled::False }));
            assert_eq!(tree.focus, *id);
            assert_eq!(snapshot.action(&click(*id, Action::Click)), Some(SettingsAction::Set(4, !on)));
        } else {
            assert_eq!(snapshot.action(&click(accesskit::NodeId(74), Action::Click)), None);
        }
    }
}
