//! Screen-reader nodes of the «Nuova chat» / «Nuova stanza» dialog. Read-only: the keyboard ring
//! (Tab, Space, Enter, Esc) is the way to operate it, so no actions are advertised.
use accesskit::{Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};

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
}

const FIRST: u64 = 90;

impl Snapshot {
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Dialog);
        root.set_label("Gnobbi");
        let mut focus = super::ROOT;
        let mut nodes = Vec::new();
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
            root.push_child(id);
            nodes.push((id, node));
        }
        nodes.insert(0, (super::ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(super::ROOT)),
            tree_id: TreeId::ROOT,
            focus,
        }
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
                item(Role::CheckBox, "Pubblica", Some(true), true),
                item(Role::Button, "Annulla", None, false),
                item(Role::Button, "Crea", None, false),
                item(Role::Label, "Nome mancante", None, false),
            ],
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
                (Role::CheckBox, "Pubblica".into(), Some(Toggled::True)),
                (Role::Button, "Annulla".into(), None),
                (Role::Button, "Crea".into(), None),
                (Role::Label, "Nome mancante".into(), None),
            ]
        );
        assert_eq!(tree.focus, tree.nodes[3].0, "focus follows the Tab ring");
        assert_eq!(tree.nodes[0].1.children().len(), 6);
    }
}
