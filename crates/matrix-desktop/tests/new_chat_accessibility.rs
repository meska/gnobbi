#![cfg(all(feature = "linux-a11y", target_os = "linux"))]

use accesskit::{Action, ActionRequest, Live, Role, TreeId};
use matrix_desktop::accessibility::new_chat::*;
use matrix_desktop::new_chat::{Entry, Status};

fn entry(id: &str, name: &str) -> Entry {
    Entry {
        user_id: id.into(),
        name: Some(name.into()),
        avatar: None,
        direct: false,
    }
}
fn open(entries: Vec<Entry>, status: Option<Status>, all: Option<bool>) -> NewChatView {
    NewChatView::Open(Box::new(Snapshot {
        text: "ma".into(),
        entries,
        active: 1,
        status,
        all,
    }))
}
fn click(id: accesskit::NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: id,
        data: None,
    }
}

#[test]
fn every_result_is_a_named_option_in_a_selected_listbox() {
    let view = open(
        vec![entry("@ma:s", "Mara"), entry("@mo:s", "Moro")],
        None,
        None,
    );
    let tree = view.tree().unwrap();
    let node = |id| &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(
        node(matrix_desktop::accessibility::ROOT).role(),
        Role::Dialog
    );
    assert_eq!(node(LIST).role(), Role::ListBox);
    assert_eq!(node(LIST).children().len(), 2);
    let first = node(accesskit::NodeId(OPTION));
    assert_eq!(first.role(), Role::ListBoxOption);
    assert_eq!(first.label(), Some("Mara, @ma:s"));
    assert_eq!(first.is_selected(), Some(false));
    assert_eq!(
        node(accesskit::NodeId(OPTION + 1)).is_selected(),
        Some(true)
    );
    assert_eq!(
        node(INPUT).active_descendant(),
        Some(accesskit::NodeId(OPTION + 1))
    );
    // Every id a parent names exists: no missing node.
    for (_, n) in &tree.nodes {
        for child in n.children() {
            assert!(tree.nodes.iter().any(|(i, _)| i == child));
        }
    }
    assert!(tree.nodes.iter().all(|(id, _)| *id != SWITCH));
    assert_eq!(tree.focus, INPUT);
}

#[test]
fn status_is_a_polite_live_line_and_empty_list_has_no_listbox() {
    for (status, text) in [
        (Status::Searching, "Ricerca in corso"),
        (Status::NoResults, "Nessun risultato"),
        (Status::Failed, "Ricerca non riuscita"),
        (Status::Partial, "Risultati parziali"),
        (
            Status::Error(matrix_engine::search::SearchError::Network),
            "Connessione non riuscita",
        ),
    ] {
        let tree = open(vec![], Some(status), None).tree().unwrap();
        let line = &tree.nodes.iter().find(|(i, _)| *i == STATUS).unwrap().1;
        assert_eq!(line.live(), Some(Live::Polite));
        assert_eq!(line.value(), Some(text));
        assert!(tree.nodes.iter().all(|(i, _)| *i != LIST));
    }
}

#[test]
fn clicking_a_result_chooses_that_user_and_stale_ids_are_ignored() {
    let view = open(
        vec![entry("@ma:s", "Mara"), entry("@mo:s", "Moro")],
        None,
        None,
    );
    assert_eq!(
        view.action(&click(accesskit::NodeId(OPTION))),
        Some(NewChatAction::Choose("@ma:s".into()))
    );
    assert!(view.action(&click(accesskit::NodeId(OPTION + 2))).is_none());
    assert_eq!(
        view.action(&click(START)),
        Some(NewChatAction::Choose("@mo:s".into()))
    );
    assert_eq!(view.action(&click(CANCEL)), Some(NewChatAction::Cancel));
    assert!(NewChatView::Closed.action(&click(CANCEL)).is_none());
}

#[test]
fn the_scope_switch_exists_only_when_the_server_allows_it() {
    let hidden = open(vec![], None, None);
    assert!(hidden.action(&click(SWITCH)).is_none());
    let shown = open(vec![], None, Some(true));
    let tree = shown.tree().unwrap();
    let switch = &tree.nodes.iter().find(|(i, _)| *i == SWITCH).unwrap().1;
    assert_eq!(switch.role(), Role::Switch);
    assert_eq!(shown.action(&click(SWITCH)), Some(NewChatAction::ToggleAll));
}
