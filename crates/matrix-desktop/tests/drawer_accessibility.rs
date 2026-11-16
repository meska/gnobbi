#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::accessibility::drawer::*;
use matrix_domain::AccountId;

fn row(id: u64, name: &str, unread: u64, current: bool, switchable: bool) -> DrawerRow {
    DrawerRow {
        id: AccountId::new(id),
        name: name.into(),
        user_id: format!("@{name}:s.org"),
        unread,
        current,
        switchable,
    }
}
fn click(node: accesskit::NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: node,
        data: None,
    }
}
fn find(tree: &accesskit::TreeUpdate, label: &str) -> accesskit::NodeId {
    tree.nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label))
        .unwrap_or_else(|| panic!("no node {label}"))
        .0
}

#[test]
fn account_rows_are_buttons_with_name_unread_and_click() {
    let snap = DrawerSnapshot {
        rows: vec![
            row(1, "anna", 0, true, false),
            row(2, "bob", 3, false, true),
        ],
        confirm: None,
        connected: true,
        update: None,
    };
    let tree = snap.tree();
    let bob = find(&tree, "bob, @bob:s.org, 3 messaggi non letti");
    let node = &tree.nodes.iter().find(|(id, _)| *id == bob).unwrap().1;
    assert_eq!(node.role(), Role::Button);
    assert!(node.supports_action(Action::Click));
    assert_eq!(
        snap.action(&click(bob)),
        Some(DrawerAction::Switch(AccountId::new(2)))
    );
    // the shown account is labelled and disabled: no switch action
    let anna = find(&tree, "anna, @anna:s.org, account attivo");
    assert_eq!(snap.action(&click(anna)), None);
    let remove = find(&tree, "Rimuovi account bob");
    assert_eq!(
        snap.action(&click(remove)),
        Some(DrawerAction::AskRemove(AccountId::new(2)))
    );
}

#[test]
fn removal_prompt_is_readable_and_hides_the_rows() {
    let mut snap = DrawerSnapshot {
        rows: vec![row(2, "bob", 0, false, true)],
        confirm: Some((AccountId::new(2), "bob".into())),
        connected: true,
        update: None,
    };
    let tree = snap.tree();
    assert!(tree.nodes.iter().any(|(_, n)| {
        n.value()
            .is_some_and(|v| v.contains("Rimuovere l'account bob"))
    }));
    assert_eq!(
        snap.action(&click(find(&tree, "Rimuovi account"))),
        Some(DrawerAction::ConfirmRemove)
    );
    assert_eq!(
        snap.action(&click(find(&tree, "Annulla"))),
        Some(DrawerAction::CancelRemove)
    );
    assert!(!snap.allows(&DrawerAction::Switch(AccountId::new(2))));
    snap.confirm = None;
    assert!(!snap.allows(&DrawerAction::ConfirmRemove));
}

#[test]
fn offline_drawer_offers_only_settings() {
    let snap = DrawerSnapshot::default();
    let tree = snap.tree();
    assert!(snap.allows(&DrawerAction::Item(DrawerItem::Settings)));
    assert!(!snap.allows(&DrawerAction::Item(DrawerItem::NewChat)));
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Nuova chat"))
    );
}

#[test]
fn staged_update_is_labelled_and_actionable_until_dismissed() {
    let mut snap = DrawerSnapshot { update: Some("1.2.3".into()), connected: true, ..Default::default() };
    let tree = snap.tree();
    assert!(tree.nodes.iter().any(|(_, n)| n.value() == Some("Aggiornamento disponibile, versione 1.2.3")));
    let now = find(&tree, "Aggiorna ora");
    let later = find(&tree, "Più tardi");
    assert_eq!(snap.action(&click(now)), Some(DrawerAction::UpdateNow));
    assert_eq!(snap.action(&click(later)), Some(DrawerAction::UpdateLater));
    snap.update = None;
    assert!(!snap.tree().nodes.iter().any(|(_, n)| n.label() == Some("Aggiorna ora")));
    assert_eq!(snap.action(&click(now)), None);
}

#[test]
fn new_space_sits_right_below_new_room_and_opens_its_dialog() {
    let snap = DrawerSnapshot { connected: true, ..Default::default() };
    let tree = snap.tree();
    let labels: Vec<_> = tree.nodes.iter().filter_map(|(_, n)| n.label()).collect();
    let at = labels.iter().position(|l| *l == "Nuova stanza").unwrap();
    assert_eq!(labels[at + 1], "Nuovo spazio");
    let node = find(&tree, "Nuovo spazio");
    assert_eq!(tree.nodes.iter().find(|(id, _)| *id == node).unwrap().1.role(), Role::Button);
    assert_eq!(snap.action(&click(node)), Some(DrawerAction::Item(DrawerItem::NewSpace)));
    // Offline the entry is neither exported nor allowed.
    let off = DrawerSnapshot::default();
    assert!(!off.allows(&DrawerAction::Item(DrawerItem::NewSpace)));
    assert!(!off.tree().nodes.iter().any(|(_, n)| n.label() == Some("Nuovo spazio")));
}
