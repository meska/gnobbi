//! Drawer settings: four on/off switches (debug, read receipts, hide deleted messages, close to tray).
use super::ROOT;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};

pub const LABELS: [&str; 4] = [
    "Modalità debug",
    "Conferme di lettura",
    "Nascondi messaggi eliminati",
    "Riduci nel tray alla chiusura",
];
const BASE: u64 = 70;

fn id(row: usize) -> NodeId {
    NodeId(BASE + row as u64)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSnapshot {
    pub focus: usize,
    pub on: [bool; 4],
}
#[derive(Debug, PartialEq)]
pub enum SettingsAction {
    Focus(usize),
    /// Explicit target state: a stale or duplicated request cannot toggle back.
    Set(usize, bool),
}
impl SettingsSnapshot {
    // shortcut: no bounds on the nodes, so pointer-based AT hit testing finds nothing; add canvas observers like notification_view when needed.
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Impostazioni");
        let mut nodes = vec![];
        for (row, label) in LABELS.into_iter().enumerate() {
            let mut node = Node::new(Role::CheckBox);
            node.set_label(label);
            node.set_toggled(if self.on[row] {
                Toggled::True
            } else {
                Toggled::False
            });
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            root.push_child(id(row));
            nodes.push((id(row), node));
        }
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: id(self.focus.min(3)),
        }
    }
    pub fn action(&self, request: &ActionRequest) -> Option<SettingsAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        let row = (0..4).find(|r| id(*r) == request.target_node)?;
        match request.action {
            Action::Focus => Some(SettingsAction::Focus(row)),
            Action::Click => Some(SettingsAction::Set(row, !self.on[row])),
            _ => None,
        }
    }
}
