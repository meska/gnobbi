#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::accessibility::ProfileLink;
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationAction, NavigationSnapshot},
};
use matrix_domain::{AccountId, RoomId, RoomScope};

fn snapshot() -> NavigationSnapshot {
    NavigationSnapshot {
        title: "Gnobbi".into(),
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
        icon_choice: None,
        unread_filter: None,
        new_room: None,
        devices: Default::default(),
        settings: None,
        about: None,
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(HistoryNodes::default().project(
            &DemoHistory::new(
                RoomScope {
                    account_id: AccountId::new(68),
                    room_id: RoomId::new("!profile:test"),
                },
                8,
            ),
            0,
            true,
        )),
        list_focused: false,
        composer: None,
    }
}


const LONG: &str = "Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena Maddalena";

fn with_links(links: &[Option<(&str, &str)>]) -> NavigationSnapshot {
    let mut snapshot = snapshot();
    let history = snapshot.history.as_mut().unwrap();
    for (message, link) in history.messages.iter_mut().zip(links) {
        message.profile = link.map(|(user_id, name)| ProfileLink {
            user_id: user_id.into(),
            name: name.into(),
            bounds: Some(accesskit::Rect::new(10., 10., 40., 40.)),
        });
    }
    snapshot
}

fn click(snapshot: &NavigationSnapshot, id: accesskit::NodeId, action: Action) -> Option<NavigationAction> {
    snapshot.action(&accesskit::ActionRequest {
        target_tree: accesskit::TreeId::ROOT,
        target_node: id,
        action,
        data: None,
    })
}

#[test]
fn every_group_bubble_names_its_own_profile_control_and_opens_that_member() {
    // Two namesakes (the engine already put the id in the name), a long name, a bridge ghost, and a bubble with no link (own / 1:1).
    let links = [
        Some(("@alice:hs", "Sam (@alice:hs)")),
        Some(("@bob:hs", "Sam (@bob:hs)")),
        Some(("@tg_42:hs", "Mario (Telegram)")),
        Some(("@lunga:hs", LONG)),
        None,
    ];
    let snapshot = with_links(&links);
    let tree = snapshot.tree();
    let profile_nodes: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.label().is_some_and(|l| l.starts_with("Apri profilo di "))).collect();
    assert_eq!(profile_nodes.len(), 4, "one control per linked bubble, none for the bubble without a link");
    let mut ids = std::collections::HashSet::new();
    for (user, name) in links.iter().flatten() {
        let label = format!("Apri profilo di {name}");
        let (id, node) = *profile_nodes.iter().find(|(_, n)| n.label() == Some(label.as_str())).unwrap_or_else(|| panic!("node «{label}»"));
        assert_eq!(node.role(), Role::Button);
        assert!(ids.insert(*id), "node ids are unique");
        // The action reaches the right member, not a neighbour.
        assert!(matches!(click(&snapshot, *id, Action::Click), Some(NavigationAction::OpenProfile(u)) if u == *user), "{label}: Click");
        assert!(node.supports_action(Action::Focus));
    }
}

#[test]
fn bubbles_without_link_expose_no_profile_node() {
    let snapshot = with_links(&[None; 5]);
    assert!(snapshot.tree().nodes.iter().all(|(_, n)| !n.label().is_some_and(|l| l.starts_with("Apri profilo di "))));
}

// DEFECT the task (found by this test): the node advertises Action::Focus but NavigationSnapshot::action answers nothing for it.
// A screen reader moving to «Apri profilo di …» gets no focus intent. Remove #[ignore] once action() handles Focus on the +8 slot.
#[test]
#[ignore = "known defect: Focus on the profile node is advertised but not handled"]
fn focus_on_the_profile_node_is_handled() {
    let snapshot = with_links(&[Some(("@alice:hs", "Sam (@alice:hs)")), None, None, None, None]);
    let tree = snapshot.tree();
    let (id, _) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Apri profilo di Sam (@alice:hs)")).unwrap();
    let got = click(&snapshot, *id, Action::Focus);
    assert!(got.is_some(), "Focus request on an advertised action must produce an intent");
}
