//! «Informazioni su Gnobbi»: build facts and five actions (Indietro, licenze, terze parti, Copia, Chiudi).
//! Only `BUILD_INFO` and the system string; no account data.
use super::ROOT;
use crate::build_info::BUILD_INFO;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Tree, TreeId, TreeUpdate};

/// Button labels in the page's row order (index = `about_row`).
pub const BUTTONS: [&str; 5] = [
    "Indietro",
    "Apri licenze",
    "Licenze di terze parti",
    "Copia informazioni",
    "Chiudi",
];
const BUTTON_BASE: u64 = 100;
const INFO_BASE: u64 = 120;

fn button_id(row: usize) -> NodeId {
    NodeId(BUTTON_BASE + row as u64)
}

#[derive(Clone, Debug, PartialEq)]
pub struct AboutSnapshot {
    /// The focused button; 0 (Indietro) when the page opens.
    pub focus: usize,
    pub system: String,
}
#[derive(Debug, PartialEq)]
pub enum AboutAction {
    Focus(usize),
    /// Same row index as the keyboard path.
    Activate(usize),
}
impl AboutSnapshot {
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Dialog);
        root.set_label("Informazioni su Gnobbi");
        let mut nodes = vec![];
        let infos = [
            ("Versione", BUILD_INFO.version),
            ("Build", BUILD_INFO.build),
            ("Commit", BUILD_INFO.git_sha),
            ("Data build", BUILD_INFO.build_date),
            ("Matrix SDK", BUILD_INFO.matrix_sdk),
            ("Sistema", self.system.as_str()),
            ("Sito", "gnobbi.com · gnobbi.chat"),
            ("Licenza", "MIT OR Apache-2.0"),
        ];
        for (i, (name, value)) in infos.into_iter().enumerate() {
            let id = NodeId(INFO_BASE + i as u64);
            let mut node = Node::new(Role::Label);
            node.set_label(format!("{name}: {value}"));
            root.push_child(id);
            nodes.push((id, node));
        }
        for (row, label) in BUTTONS.into_iter().enumerate() {
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            root.push_child(button_id(row));
            nodes.push((button_id(row), node));
        }
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: button_id(self.focus.min(BUTTONS.len() - 1)),
        }
    }
    pub fn action(&self, request: &ActionRequest) -> Option<AboutAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        let row = (0..BUTTONS.len()).find(|r| button_id(*r) == request.target_node)?;
        match request.action {
            Action::Focus => Some(AboutAction::Focus(row)),
            Action::Click => Some(AboutAction::Activate(row)),
            _ => None,
        }
    }
}
