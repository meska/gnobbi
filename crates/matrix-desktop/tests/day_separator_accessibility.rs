#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
//! The day separator is a node of its own in the history list, labelled with the day text only, once per day.
use accesskit::Role;
use matrix_desktop::{
    DemoHistory,
    accessibility::{HistoryNodes, NavigationSnapshot, semantic_session::SemanticSession},
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
use matrix_engine::history::{HistoryEvent, HistorySender};

fn row(id: &str, timestamp_ms: u64) -> HistoryEvent {
    HistoryEvent {
        event_id: EventId::new(id),
        body: id.into(),
        outgoing: false,
        redacted: false,
        timestamp_ms,
        sender: HistorySender::new("@anna:test", None),
        video: None,
        attachment_name: None,
        attachment_size: None,
        styles: Vec::new(),
        thread: None,
        thread_root: None,
        status: false,
        utd: false,
        media: None,
        mentions: Vec::new(),
        highlighted: false,
        poll: None,
    }
}

fn snapshot() -> NavigationSnapshot {
    let mut history = DemoHistory::new(RoomScope { account_id: AccountId::new(65), room_id: RoomId::new("!days:test") }, 0);
    // Two messages on one day, a third a week later (far enough to be one day apart in every zone).
    history.prepend_server(vec![row("$a", 86_400_000 * 20), row("$b", 86_400_000 * 20 + 60_000), row("$c", 86_400_000 * 40)]);
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
        history: Some(HistoryNodes::default().project(&history, 0, true)),
        list_focused: false,
        composer: None,
    }
}


#[test]
fn separator_node_carries_only_the_day_text_and_appears_once_per_day() {
    let tree = SemanticSession::new(snapshot()).tree();
    let labels: Vec<&str> = tree.nodes.iter().filter(|(_, n)| n.role() == Role::Label).filter_map(|(_, n)| n.label()).collect();
    assert_eq!(labels, [matrix_desktop::time_fmt::day_label(86_400_000 * 20), matrix_desktop::time_fmt::day_label(86_400_000 * 40)]);
    let mut ids = std::collections::HashSet::new();
    assert!(tree.nodes.iter().all(|(id, _)| ids.insert(*id)), "duplicate node id");
}
