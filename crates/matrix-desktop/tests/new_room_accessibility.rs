#![cfg(all(feature = "linux-a11y", target_os = "linux"))]
//! «Unisciti» uses the shared projections: the server menu and the search list.

use accesskit::{Action, ActionRequest, Live, NodeId, Role, TreeId};
use matrix_desktop::accessibility::directory_servers::{self, ServerAction, ServerMenu};
use matrix_desktop::accessibility::new_room::*;
use matrix_desktop::directory_servers::{SavedServers, ServerPicker};
use matrix_desktop::search_list::{Hit, Row, Status};

struct Room;
impl Hit for Room {
    fn key(&self) -> &str {
        "!c:s"
    }
    fn title(&self) -> &str {
        "Cucina"
    }
    fn detail(&self) -> String {
        "#cucina:s · 12 membri".into()
    }
    fn avatar(&self) -> Option<&str> {
        None
    }
}

fn click(id: NodeId) -> ActionRequest {
    ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: id, data: None }
}
fn item(role: Role, label: &str, focused: bool) -> Item {
    Item { role, label: label.into(), checked: None, focused, bounds: None }
}
fn snapshot(open: bool, rows: &[Row<'_, Room>], status: Option<Status>) -> Snapshot {
    let mut picker = ServerPicker::new(1, "a.org", vec!["b.org".into()], SavedServers::default());
    if open {
        picker.open();
        picker.key("down");
    }
    Snapshot {
        items: vec![
            item(Role::TextInput, "Cerca", !open),
            item(Role::Button, "Annulla", false),
            item(Role::Button, "Unisciti", false),
        ],
        join: Some(Join::new(1, ServerMenu::of(&picker), open, rows, 0, status)),
    }
}

#[test]
fn open_menu_has_rows_and_the_highlighted_one_holds_the_focus() {
    let tree = snapshot(true, &[], None).tree();
    let node = |id| &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(node(directory_servers::MENU).children().len(), 4);
    assert_eq!(node(directory_servers::BUTTON).is_expanded(), Some(true));
    assert_eq!(tree.focus, NodeId(directory_servers::ITEM + 1));
    assert_eq!(node(NodeId(directory_servers::ITEM + 1)).label(), Some("b.org"));
}

#[test]
fn active_option_is_selected_and_the_search_field_points_at_it() {
    let room = Room;
    let tree = snapshot(false, &[Row::Hit(&room)], None).tree();
    let node = |id| &tree.nodes.iter().find(|(i, _)| *i == id).unwrap().1;
    assert_eq!(node(LIST).role(), Role::ListBox);
    let option = node(NodeId(OPTION));
    assert_eq!(option.label(), Some("Cucina, #cucina:s · 12 membri"));
    assert_eq!(option.is_selected(), Some(true));
    let input = node(NodeId(90));
    assert_eq!(input.active_descendant(), Some(NodeId(OPTION)));
    assert_eq!(input.controls(), [LIST]);
    // The server button sits right after the search field; no flat result node remains.
    let root = node(matrix_desktop::accessibility::ROOT).children();
    assert_eq!(root[1], directory_servers::BUTTON);
    assert!(tree.nodes.iter().filter(|(_, n)| n.role() == Role::Button).all(|(id, _)| {
        *id == directory_servers::BUTTON || id.0 >= 90 && id.0 < 100
    }));
    for (_, n) in &tree.nodes {
        assert!(n.children().iter().all(|c| tree.nodes.iter().any(|(i, _)| i == c)));
    }
    assert_eq!(tree.focus, NodeId(90));
}

#[test]
fn status_is_live_and_clicks_become_actions() {
    let room = Room;
    let empty = snapshot(false, &[], Some(Status::NoResults));
    let tree = empty.tree();
    let status = &tree.nodes.iter().find(|(i, _)| *i == STATUS).unwrap().1;
    assert_eq!((status.value(), status.live()), (Some("Nessun risultato"), Some(Live::Polite)));
    assert!(tree.nodes.iter().all(|(i, _)| *i != LIST), "no empty listbox");

    let s = snapshot(false, &[Row::Hit(&room)], None);
    assert_eq!(s.action(&click(NodeId(OPTION))), Some(NewRoomAction::Join(0)));
    assert_eq!(s.action(&click(NodeId(OPTION + 1))), None);
    assert_eq!(
        s.action(&click(directory_servers::BUTTON)),
        Some(NewRoomAction::Server(ServerAction::Toggle))
    );
}
