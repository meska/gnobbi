#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
use accesskit::{Role, Toggled};
use matrix_desktop::{DemoHistory, accessibility::{HistoryNodes, NavigationSnapshot}};
use matrix_domain::{AccountId, EventId, Poll, PollAnswer, RoomId, RoomScope};

fn snapshot(history: matrix_desktop::accessibility::HistorySnapshot) -> NavigationSnapshot {
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
        details: Default::default(),
        notifications: Default::default(),
        dark: false,
        visible: false,
        query: String::new(),
        search_focused: false,
        search_bounds: None,
        rooms: vec![],
        history: Some(history),
        list_focused: false,
        composer: None,
    }
}

fn poll(disclosed: bool, ended: bool, multiple: bool) -> Poll {
    let a = |id: &str, text: &str, votes, own| PollAnswer { id: id.into(), text: text.into(), votes, own };
    Poll {
        question: "Pizza?".into(),
        answers: vec![a("y", "Si", 5, true), a("n", "No", 1, false)],
        max_selections: if multiple { 2 } else { 1 },
        multiple,
        disclosed,
        ended,
        voters: 6,
        can_end: false,
    }
}

fn nodes(poll: Poll) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    let mut history = DemoHistory::new(RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!p:test") }, 0);
    history.prepend_server(vec![matrix_engine::history::HistoryEvent {
        poll: Some(poll),
        redacted: false,
        event_id: EventId::new("$poll:test"),
        body: "Pizza?".into(),
        outgoing: false,
        timestamp_ms: 0,
        sender: matrix_engine::history::HistorySender::new("@peer:test", None),
        video: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        attachment_name: None,
        attachment_size: None,
    }]);
    snapshot(HistoryNodes::default().project(&history, 0, true)).tree().nodes
}

fn labels(nodes: &[(accesskit::NodeId, accesskit::Node)], role: Role) -> Vec<String> {
    nodes.iter().filter(|(_, n)| n.role() == role).filter_map(|(_, n)| n.label().map(str::to_owned)).collect()
}

#[test]
fn disclosed_poll_has_a_group_radio_options_and_a_footer() {
    let tree = nodes(poll(true, false, false));
    assert_eq!(labels(&tree, Role::Group), ["Sondaggio: Pizza?"]);
    assert_eq!(labels(&tree, Role::RadioButton), ["Si, 5 voti, selezionata", "No, 1 voto"]);
    assert_eq!(labels(&tree, Role::Label), ["Basato su 6 voti"]);
    let toggled: Vec<_> = tree.iter().filter(|(_, n)| n.role() == Role::RadioButton).map(|(_, n)| n.toggled()).collect();
    assert_eq!(toggled, [Some(Toggled::True), Some(Toggled::False)]);
    assert!(tree.iter().filter(|(_, n)| n.role() == Role::RadioButton).all(|(_, n)| !n.is_disabled()));
}

#[test]
fn hidden_results_leave_out_the_counts() {
    let tree = nodes(poll(false, false, false));
    assert_eq!(labels(&tree, Role::RadioButton), ["Si, selezionata", "No"]);
    assert_eq!(labels(&tree, Role::Label), ["6 voti"]);
}

#[test]
fn ended_multiple_poll_uses_checkboxes_disabled_and_a_closed_group() {
    let tree = nodes(poll(false, true, true));
    assert_eq!(labels(&tree, Role::Group), ["Sondaggio: Pizza?, chiuso"]);
    assert_eq!(labels(&tree, Role::CheckBox), ["Si, 5 voti, selezionata", "No, 1 voto"]);
    assert!(tree.iter().filter(|(_, n)| n.role() == Role::CheckBox).all(|(_, n)| n.is_disabled()));
}
