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
        on: [false, true, false],
    };
    let tree = snapshot.tree();
    let boxes: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::CheckBox)
        .collect();
    let labels: Vec<_> = boxes.iter().map(|(_, n)| n.label().unwrap()).collect();
    assert_eq!(
        labels,
        ["Modalità debug", "Conferme di lettura", "Nascondi messaggi eliminati"]
    );
    let toggled: Vec<_> = boxes.iter().map(|(_, n)| n.toggled().unwrap()).collect();
    assert_eq!(
        toggled,
        [accesskit::Toggled::False, accesskit::Toggled::True, accesskit::Toggled::False]
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
