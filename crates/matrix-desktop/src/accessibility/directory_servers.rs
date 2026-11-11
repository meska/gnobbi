//! Screen-reader nodes of the directory server picker: a button with the chosen server, and while
//! open a menu of check items (servers) and plain items («Aggiungi nuovo server…», «Rimuovi …»).
use crate::directory_servers::{Row, ServerPicker};
use accesskit::{Action, ActionRequest, HasPopup, Live, Node, NodeId, Role, Toggled, TreeId};

pub const BUTTON: NodeId = NodeId(1200);
pub const MENU: NodeId = NodeId(1201);
pub const ERROR: NodeId = NodeId(1202);
/// Row `i` of the menu is node `ITEM + i`.
pub const ITEM: u64 = 1210;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub label: String,
    /// `Some` only for server rows.
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerMenu {
    pub shown: String,
    pub open: bool,
    pub items: Vec<MenuItem>,
    pub highlighted: usize,
    /// The one error line of «Aggiungi nuovo server…».
    pub error: Option<&'static str>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerAction {
    Toggle,
    Choose(usize),
}

impl ServerMenu {
    pub fn of(p: &ServerPicker) -> Self {
        let items = p
            .rows()
            .iter()
            .map(|r| MenuItem {
                label: p.label(r),
                checked: matches!(r, Row::Server(_)).then(|| p.is_selected(r)),
            })
            .collect();
        Self {
            shown: p.shown().to_owned(),
            open: p.is_open(),
            items,
            highlighted: p.highlighted(),
            error: p.add_error(),
        }
    }

    /// Top-level node ids to hang under the dialog, all nodes, and the node holding the focus.
    pub fn nodes(&self) -> (Vec<NodeId>, Vec<(NodeId, Node)>, NodeId) {
        let mut button = Node::new(Role::Button);
        button.set_label(format!("Server: {}", self.shown));
        button.set_has_popup(HasPopup::Menu);
        button.set_expanded(self.open);
        button.add_action(Action::Click);
        let mut top = vec![BUTTON];
        let mut nodes = vec![(BUTTON, button)];
        let mut focus = BUTTON;
        if self.open {
            let mut menu = Node::new(Role::Menu);
            menu.set_label("Server");
            for (i, item) in self.items.iter().enumerate() {
                let id = NodeId(ITEM + i as u64);
                let mut node = Node::new(if item.checked.is_some() {
                    Role::MenuItemCheckBox
                } else {
                    Role::MenuItem
                });
                node.set_label(item.label.as_str());
                if let Some(c) = item.checked {
                    node.set_toggled(if c { Toggled::True } else { Toggled::False });
                }
                node.add_action(Action::Click);
                menu.push_child(id);
                nodes.push((id, node));
                if i == self.highlighted {
                    focus = id;
                }
            }
            nodes.push((MENU, menu));
            top.push(MENU);
        }
        if let Some(text) = self.error {
            let mut e = Node::new(Role::Label);
            e.set_value(text);
            e.set_live(Live::Polite);
            nodes.push((ERROR, e));
            top.push(ERROR);
        }
        (top, nodes, focus)
    }

    pub fn action(&self, request: &ActionRequest) -> Option<ServerAction> {
        if request.target_tree != TreeId::ROOT
            || request.data.is_some()
            || request.action != Action::Click
        {
            return None;
        }
        match request.target_node {
            BUTTON => Some(ServerAction::Toggle),
            NodeId(n) if self.open && n >= ITEM && ((n - ITEM) as usize) < self.items.len() => {
                Some(ServerAction::Choose((n - ITEM) as usize))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_servers::SavedServers;
    use matrix_engine::search::SearchError;

    fn picker() -> ServerPicker {
        let mut store = SavedServers::default();
        store.add(1, "z.org");
        ServerPicker::new(1, "a.org", vec!["b.org".into()], store)
    }
    fn click(id: NodeId) -> ActionRequest {
        ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: id,
            data: None,
        }
    }
    fn node(nodes: &[(NodeId, Node)], id: NodeId) -> &Node {
        &nodes.iter().find(|(i, _)| *i == id).unwrap().1
    }

    #[test]
    fn closed_menu_is_one_collapsed_button() {
        let (top, nodes, focus) = ServerMenu::of(&picker()).nodes();
        assert_eq!((top, focus), (vec![BUTTON], BUTTON));
        let b = node(&nodes, BUTTON);
        assert_eq!(b.role(), Role::Button);
        assert_eq!(b.label(), Some("Server: a.org"));
        assert_eq!(b.is_expanded(), Some(false));
        assert_eq!(b.has_popup(), Some(HasPopup::Menu));
    }

    #[test]
    fn open_menu_lists_checked_servers_then_fixed_items_and_follows_the_keyboard() {
        let mut p = picker();
        p.key("down");
        p.key("down");
        let view = ServerMenu::of(&p);
        let (top, nodes, focus) = view.nodes();
        assert_eq!(top, vec![BUTTON, MENU]);
        assert_eq!(node(&nodes, BUTTON).is_expanded(), Some(true));
        let menu = node(&nodes, MENU);
        assert_eq!(menu.role(), Role::Menu);
        let got: Vec<_> = menu
            .children()
            .iter()
            .map(|id| {
                let n = node(&nodes, *id);
                (n.role(), n.label().unwrap().to_owned(), n.toggled())
            })
            .collect();
        assert_eq!(
            got,
            [
                (Role::MenuItemCheckBox, "a.org — Il tuo server".into(), Some(Toggled::True)),
                (Role::MenuItemCheckBox, "b.org".into(), Some(Toggled::False)),
                (Role::MenuItemCheckBox, "z.org".into(), Some(Toggled::False)),
                (Role::MenuItemCheckBox, "matrix.org".into(), Some(Toggled::False)),
                (Role::MenuItem, "Aggiungi nuovo server…".into(), None),
                (Role::MenuItem, "Rimuovi z.org".into(), None),
            ]
        );
        assert_eq!(focus, NodeId(ITEM + 1), "focus on the highlighted row");
        // every id a parent names exists
        assert!(menu.children().iter().all(|id| nodes.iter().any(|(i, _)| i == id)));
    }

    #[test]
    fn clicks_toggle_or_choose_and_error_is_a_live_line() {
        let mut p = picker();
        assert_eq!(ServerMenu::of(&p).action(&click(BUTTON)), Some(ServerAction::Toggle));
        assert_eq!(ServerMenu::of(&p).action(&click(NodeId(ITEM + 1))), None, "closed");
        p.open();
        let v = ServerMenu::of(&p);
        assert_eq!(v.action(&click(NodeId(ITEM + 5))), Some(ServerAction::Choose(5)));
        assert_eq!(v.action(&click(NodeId(ITEM + 6))), None);
        assert_eq!(v.action(&click(ERROR)), None);
        p.choose(4);
        p.confirm_add("x.org");
        p.probed("x.org", Err(SearchError::Unreachable));
        let (top, nodes, _) = ServerMenu::of(&p).nodes();
        assert_eq!(top.last(), Some(&ERROR));
        let e = node(&nodes, ERROR);
        assert_eq!((e.value(), e.live()), (Some("Il server non risponde"), Some(Live::Polite)));
    }
}
