//! Screen-reader nodes of the «Nuova stanza» / «Unisciti» dialog. The keyboard ring (Tab, Space,
//! Enter, Esc) operates it; only «Unisciti» advertises clicks (server menu and result options).
use super::directory_servers::{ServerAction, ServerMenu};
use super::search_list::{self, Ids};
use crate::search_list::{Hit, Row, Status};
use accesskit::{
    ActionRequest, Action, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub role: Role,
    pub label: String,
    /// Only for the «Pubblica» switch.
    pub checked: Option<bool>,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
}

/// Visible dialog controls in Tab order, then the error line when there is one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub items: Vec<Item>,
    /// «Unisciti»: server menu and results, shown after `items[..at]` (the first item is the search field).
    pub join: Option<Join>,
}

/// The shared projections of the directory search: [`ServerMenu`] (1200..) and [`search_list::project`].
#[derive(Clone, Debug, PartialEq)]
pub struct Join {
    pub at: usize,
    pub menu: ServerMenu,
    /// The server button or one of its menu items holds the focus.
    pub menu_focused: bool,
    list: Vec<(NodeId, Node)>,
    status: (NodeId, Node),
    active: Option<NodeId>,
    rows: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NewRoomAction {
    Server(ServerAction),
    /// Highlight result `i` and join it, as Enter does.
    Join(usize),
}

pub const LIST: NodeId = NodeId(1300);
pub const STATUS: NodeId = NodeId(1301);
/// Result `i` is node `OPTION + i`.
pub const OPTION: u64 = 2000;
const IDS: Ids = Ids { list: LIST, status: STATUS, option: OPTION };

impl Join {
    pub fn new<H: Hit>(
        at: usize,
        menu: ServerMenu,
        menu_focused: bool,
        rows: &[Row<'_, H>],
        active: usize,
        status: Option<Status>,
    ) -> Self {
        let p = search_list::project(IDS, rows, active, status);
        Self { at, menu, menu_focused, list: p.list, status: p.status, active: p.active, rows: rows.len() }
    }
}

const FIRST: u64 = 90;

impl Snapshot {
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Dialog);
        root.set_label("Gnobbi");
        let mut focus = super::ROOT;
        let mut nodes = Vec::new();
        let mut ids = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            let id = NodeId(FIRST + i as u64);
            let mut node = Node::new(item.role);
            node.set_label(item.label.as_str());
            if let Some(checked) = item.checked {
                node.set_toggled(if checked {
                    Toggled::True
                } else {
                    Toggled::False
                });
            }
            if let Some(bounds) = item.bounds {
                node.set_bounds(bounds);
            }
            if item.focused {
                focus = id;
            }
            ids.push(id);
            nodes.push((id, node));
        }
        if let Some(join) = &self.join {
            let (top, menu, menu_focus) = join.menu.nodes();
            let mut shown = top;
            if !join.list.is_empty() {
                shown.push(LIST);
            }
            shown.push(STATUS);
            ids.splice(join.at..join.at, shown);
            nodes.extend(menu);
            nodes.extend(join.list.iter().cloned());
            nodes.push(join.status.clone());
            if join.menu_focused {
                focus = menu_focus;
            }
            if let Some((_, input)) = nodes.first_mut().filter(|_| join.at > 0) {
                if !join.list.is_empty() {
                    input.set_controls(vec![LIST]);
                    input.set_has_popup(accesskit::HasPopup::Listbox);
                }
                if let Some(active) = join.active {
                    input.set_active_descendant(active);
                }
            }
        }
        root.set_children(ids);
        nodes.insert(0, (super::ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(super::ROOT)),
            tree_id: TreeId::ROOT,
            focus,
        }
    }

    pub fn action(&self, request: &ActionRequest) -> Option<NewRoomAction> {
        let join = self.join.as_ref()?;
        if let Some(action) = join.menu.action(request) {
            return Some(NewRoomAction::Server(action));
        }
        if request.target_tree != TreeId::ROOT || request.data.is_some() || request.action != Action::Click {
            return None;
        }
        search_list::index_of(IDS, request.target_node, join.rows).map(NewRoomAction::Join)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(role: Role, label: &str, checked: Option<bool>, focused: bool) -> Item {
        Item {
            role,
            label: label.into(),
            checked,
            focused,
            bounds: None,
        }
    }

    #[test]
    fn room_dialog_exposes_fields_switch_buttons_error_and_focus() {
        let snapshot = Snapshot {
            items: vec![
                item(Role::TextInput, "Nome", None, false),
                item(Role::TextInput, "Argomento", None, false),
                item(Role::Switch, "Pubblica", Some(true), true),
                item(Role::Button, "Annulla", None, false),
                item(Role::Button, "Crea", None, false),
                item(Role::Label, "Nome mancante", None, false),
            ],
            join: None,
        };
        let tree = snapshot.tree();
        let got: Vec<_> = tree.nodes[1..]
            .iter()
            .map(|(_, n)| (n.role(), n.label().unwrap().to_owned(), n.toggled()))
            .collect();
        assert_eq!(
            got,
            [
                (Role::TextInput, "Nome".into(), None),
                (Role::TextInput, "Argomento".into(), None),
                (Role::Switch, "Pubblica".into(), Some(Toggled::True)),
                (Role::Button, "Annulla".into(), None),
                (Role::Button, "Crea".into(), None),
                (Role::Label, "Nome mancante".into(), None),
            ]
        );
        assert_eq!(tree.focus, tree.nodes[3].0, "focus follows the Tab ring");
        assert_eq!(tree.nodes[0].1.children().len(), 6);
    }
}
