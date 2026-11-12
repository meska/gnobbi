#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Live, Role};
use matrix_desktop::accessibility::NavigationSnapshot;

fn snapshot(visible: bool) -> NavigationSnapshot {
    NavigationSnapshot {
        new_chat: Default::default(),
        room_search: Default::default(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        drawer: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        disconnected: None,
        devices: Default::default(),
        settings: None,
        about: None,
        icon_choice: None,
        new_room: None,
        title: String::new(),
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

fn statuses(s: &NavigationSnapshot) -> Vec<accesskit::Node> {
    s.tree().nodes.into_iter().map(|(_, n)| n).filter(|n| n.role() == Role::Status).collect()
}

#[test]
fn disconnected_node_exists_only_while_disconnected() {
    let mut s = snapshot(true);
    assert!(statuses(&s).is_empty(), "connected: no node, so nothing in the focus chain");
    s.disconnected = Some("Sync interrotta");
    let nodes = statuses(&s);
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].label(), Some("Sync interrotta"));
    assert_eq!(nodes[0].live(), Live::Polite);
    s.disconnected = None;
    assert!(statuses(&s).is_empty());
}
