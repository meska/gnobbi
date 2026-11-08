#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::accessibility::about::*;
use matrix_desktop::accessibility::drawer::{DrawerAction, DrawerItem, DrawerSnapshot};

fn req(action: Action, node: accesskit::NodeId) -> ActionRequest {
    ActionRequest { action, target_tree: TreeId::ROOT, target_node: node, data: None }
}
fn find(tree: &accesskit::TreeUpdate, label: &str) -> accesskit::NodeId {
    tree.nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label))
        .unwrap_or_else(|| panic!("no node {label}"))
        .0
}

#[test]
fn drawer_has_a_clickable_info_row_even_offline() {
    let snap = DrawerSnapshot { rows: vec![], confirm: None, connected: false };
    let tree = snap.tree();
    let id = find(&tree, "Informazioni su Gnobbi");
    let node = &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(node.role(), Role::Button);
    assert_eq!(
        snap.action(&req(Action::Click, id)),
        Some(DrawerAction::Item(DrawerItem::About))
    );
}

#[test]
fn info_page_is_a_named_dialog_with_named_rows_and_focus_on_back() {
    let snap = AboutSnapshot { focus: 0, system: "Linux 6.1 (x86_64)".into() };
    let tree = snap.tree();
    let root = &tree.nodes[0].1;
    assert_eq!(root.role(), Role::Dialog);
    assert_eq!(root.label(), Some("Informazioni su Gnobbi"));
    for name in ["Versione", "Build", "Commit", "Data build", "Matrix SDK", "Sito", "Licenza"] {
        assert!(
            tree.nodes.iter().any(|(_, n)| n.label().is_some_and(|l| l.starts_with(&format!("{name}: ")))),
            "{name}"
        );
    }
    find(&tree, "Sistema: Linux 6.1 (x86_64)");
    assert_eq!(tree.focus, find(&tree, "Indietro"));
    for b in BUTTONS {
        find(&tree, b);
    }
}

#[test]
fn copy_information_is_activatable_and_stale_requests_are_ignored() {
    let snap = AboutSnapshot { focus: 0, system: String::new() };
    let tree = snap.tree();
    let copy = find(&tree, "Copia informazioni");
    let node = &tree.nodes.iter().find(|(i, _)| *i == copy).unwrap().1;
    assert!(node.supports_action(Action::Click));
    assert_eq!(snap.action(&req(Action::Click, copy)), Some(AboutAction::Activate(3)));
    assert_eq!(snap.action(&req(Action::Focus, copy)), Some(AboutAction::Focus(3)));
    assert_eq!(snap.action(&req(Action::Click, find(&tree, "Chiudi"))), Some(AboutAction::Activate(4)));
    let info = find(&tree, &format!("Sito: {}", "gnobbi.com · gnobbi.chat"));
    assert_eq!(snap.action(&req(Action::Click, info)), None);
}
