#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role, Toggled};
use matrix_desktop::IconChoice;
use matrix_desktop::accessibility::{NavigationAction, NavigationSnapshot};

fn snapshot(icon_choice: Option<IconChoice>) -> NavigationSnapshot {
    NavigationSnapshot {
        title: "Gnobbi".into(),
        #[cfg(all(feature = "live", unix))]
        signin: None,
        openers: vec![],
        back: None,
        scrollbars: vec![],
        operation_panel: None,
        spaces: None,
        navigation_revision: 0,
        icon_choice,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
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
fn settings_icons_are_labelled_radios_with_the_current_one_selected() {
    let snap = snapshot(Some(IconChoice::Gnome));
    let tree = snap.tree();
    let radios: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::RadioButton).collect();
    let got: Vec<_> = radios.iter().map(|(_, n)| (n.label().unwrap(), n.toggled())).collect();
    assert_eq!(
        got,
        [
            ("Segno minimo", Some(Toggled::False)),
            ("Gnomo", Some(Toggled::True)),
            ("Mostriciattolo", Some(Toggled::False))
        ]
    );
    for (id, icon) in radios.iter().map(|(i, _)| *i).zip(IconChoice::ALL) {
        let request = accesskit::ActionRequest {
            action: Action::Click,
            target_node: id,
            target_tree: accesskit::TreeId::ROOT,
            data: None,
        };
        assert!(matches!(snap.action(&request), Some(NavigationAction::ChooseIcon(i)) if i == icon));
    }
    assert!(!snapshot(None).tree().nodes.iter().any(|(_, n)| n.role() == Role::RadioButton));
}
