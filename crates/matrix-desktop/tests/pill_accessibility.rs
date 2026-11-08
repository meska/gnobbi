#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Action, Role};
use matrix_desktop::accessibility::PillLink;
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
                    account_id: AccountId::new(69),
                    room_id: RoomId::new("!pills:test"),
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

fn pill(user: &str, name: &str, start: usize, focused: bool) -> PillLink {
    PillLink { user_id: user.into(), name: name.into(), start, bounds: Some(accesskit::Rect::new(1., 1., 9., 9.)), focused }
}

fn with_pills(pills: Vec<PillLink>) -> NavigationSnapshot {
    let mut snapshot = snapshot();
    snapshot.history.as_mut().unwrap().messages[0].pills = pills;
    snapshot
}

fn request(snapshot: &NavigationSnapshot, id: accesskit::NodeId, action: Action) -> Option<NavigationAction> {
    snapshot.action(&accesskit::ActionRequest { target_tree: accesskit::TreeId::ROOT, target_node: id, action, data: None })
}

#[test]
fn every_pill_is_a_button_named_after_the_person_and_opens_that_person() {
    // Two namesakes (the engine put the id in the name) and a plain name.
    let pills = vec![pill("@alice:hs", "Sam (@alice:hs)", 3, false), pill("@bob:hs", "Sam (@bob:hs)", 20, false), pill("@carla:hs", "Carla", 40, false)];
    let snapshot = with_pills(pills.clone());
    let tree = snapshot.tree();
    let message = &snapshot.history.as_ref().unwrap().messages[0];
    let list_item = tree.nodes.iter().find(|(id, _)| *id == message.id).map(|(_, n)| n).unwrap();
    let mut ids = std::collections::HashSet::new();
    for (i, p) in pills.iter().enumerate() {
        let id = message.pill_id(i);
        assert!(ids.insert(id), "pill node ids are unique");
        assert!(list_item.children().contains(&id), "the pill sits inside its message node");
        let (_, node) = tree.nodes.iter().find(|(n, _)| *n == id).unwrap_or_else(|| panic!("node for {}", p.name));
        assert_eq!((node.role(), node.label()), (Role::Button, Some(p.name.as_str())));
        assert!(node.supports_action(Action::Focus) && node.supports_action(Action::Click));
        assert!(matches!(request(&snapshot, id, Action::Click), Some(NavigationAction::OpenProfile(u)) if u == p.user_id));
        assert!(matches!(request(&snapshot, id, Action::Focus), Some(NavigationAction::FocusPill(e, s)) if e == message.event && s == p.start));
    }
    // The message node still carries the whole text, so the reader reads the words around the pills.
    assert_eq!(list_item.label(), Some(message.text.as_str()));
}

#[test]
fn a_message_without_user_pills_has_no_pill_node() {
    let snapshot = with_pills(vec![]);
    let message = &snapshot.history.as_ref().unwrap().messages[0];
    assert!(snapshot.tree().nodes.iter().all(|(id, _)| (0..16).all(|i| *id != message.pill_id(i))));
}

#[test]
fn tab_walks_the_pills_in_text_order_enter_and_space_open_the_focused_one() {
    let key = |pills: Vec<PillLink>, k: &str, shift: bool| with_pills(pills).history.unwrap().link_key(k, shift);
    let trio = |at: Option<usize>| -> Vec<PillLink> {
        [("@a:hs", "A", 2), ("@b:hs", "B", 9), ("@c:hs", "C", 30)].iter().enumerate().map(|(i, (u, n, s))| pill(u, n, *s, at == Some(i))).collect()
    };
    assert!(matches!(key(trio(None), "f9", false), Some(NavigationAction::FocusPill(_, 2))), "F9 reaches the first stop");
    assert!(matches!(key(trio(Some(0)), "tab", false), Some(NavigationAction::FocusPill(_, 9))));
    assert!(matches!(key(trio(Some(1)), "tab", true), Some(NavigationAction::FocusPill(_, 2))));
    assert!(matches!(key(trio(Some(2)), "tab", false), Some(NavigationAction::FocusComposer)), "past the last stop the ring goes on");
    assert!(matches!(key(trio(Some(0)), "tab", true), Some(NavigationAction::FocusMessage(_))));
    assert!(matches!(key(trio(Some(0)), "escape", false), Some(NavigationAction::FocusMessage(_))));
    for k in ["enter", "space"] {
        assert!(matches!(key(trio(Some(1)), k, false), Some(NavigationAction::OpenProfile(u)) if u == "@b:hs"), "{k}");
    }
    assert!(key(trio(None), "enter", false).is_none(), "nothing focused, nothing opens");
}

#[test]
fn the_focused_pill_holds_the_tree_focus() {
    let snapshot = with_pills(vec![pill("@a:hs", "A", 2, false), pill("@b:hs", "B", 9, true)]);
    let message = &snapshot.history.as_ref().unwrap().messages[0];
    assert_eq!(snapshot.tree().focus, message.pill_id(1));
}

#[test]
fn pill_ids_never_collide_with_the_composer_text_runs() {
    let mut snapshot = with_pills(vec![pill("@a:hs", "A", 2, false)]);
    snapshot.composer = Some(matrix_desktop::accessibility::ComposerSnapshot {
        send_source: Some(1),
        scope: RoomScope { account_id: AccountId::new(69), room_id: RoomId::new("!pills:test") },
        composing: false,
        send_focused: false,
        icon_focused: None,
        live: true,
        send_bounds: None,
        attachment: None,
        mentions: None,
        recording: None,
        emoji_open: false,
        picker_pinned: false,
        picker_selected: None,
        picker_close_focused: false,
        text: "ciao".into(),
        anchor: 0,
        cursor: 0,
        focused: false,
        bounds: Some(accesskit::Rect::new(0., 600., 400., 640.)),
    });
    let ids: Vec<_> = snapshot.tree().nodes.iter().map(|(id, _)| *id).collect();
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len());
}
