#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::accessibility::install_offer::Snapshot;
use matrix_desktop::install_offer::Choice;

fn click(node: accesskit::NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: node,
        data: None,
    }
}

#[test]
fn dialog_has_a_dialog_with_three_labelled_buttons_and_maps_clicks() {
    let s = Snapshot {
        generation: 1,
        dialog: Some([None; 3]),
        ..Default::default()
    };
    let tree = s.tree().unwrap();
    let buttons: Vec<_> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::Button)
        .collect();
    let labels: Vec<_> = buttons.iter().map(|(_, n)| n.label().unwrap()).collect();
    assert_eq!(labels, ["Aggiungi al menu", "Non ora", "Non chiedere più"]);
    assert!(tree.nodes.iter().any(|(_, n)| n.role() == Role::Dialog));
    let choices: Vec<_> = buttons
        .iter()
        .map(|(id, _)| s.action(&click(*id)))
        .collect();
    assert_eq!(
        choices,
        [Some(Choice::Add), Some(Choice::Later), Some(Choice::Never)]
    );
}

#[test]
fn settings_row_is_a_button_only_while_settings_is_open() {
    let mut s = Snapshot::default();
    assert!(s.tree().is_none());
    s.settings_row = Some(None);
    let mut tree = accesskit::TreeUpdate {
        nodes: vec![(accesskit::NodeId(1), accesskit::Node::new(Role::Window))],
        tree: Some(accesskit::Tree::new(accesskit::NodeId(1))),
        tree_id: TreeId::ROOT,
        focus: accesskit::NodeId(1),
    };
    s.add_settings_row(&mut tree);
    let (id, row) = tree.nodes.last().unwrap();
    assert_eq!(
        (row.role(), row.label()),
        (Role::Button, Some("Aggiungi al menu"))
    );
    assert_eq!(tree.nodes[0].1.children(), [*id]);
    assert_eq!(s.action(&click(*id)), Some(Choice::Add));
    s.settings_row = None;
    assert_eq!(s.action(&click(*id)), None);
}
