//! Accessible tree of the launcher-entry proposal and of its Settings row. Ids sit above the
//! other ranges and carry a generation, so a reopened control is a new AT object.
use crate::install_offer::Choice;
use accesskit::{Action, ActionRequest, Node, NodeId, Rect, Role, Tree, TreeId, TreeUpdate};

const BASE: u64 = 1 << 52;
const LABELS: [(Choice, &str); 3] = [
    (Choice::Add, "Aggiungi al menu"),
    (Choice::Later, "Non ora"),
    (Choice::Never, "Non chiedere più"),
];

#[derive(Clone, Default)]
pub struct Snapshot {
    pub generation: u64,
    /// Button bounds (Add, Later, Never) while the proposal is open.
    pub dialog: Option<[Option<Rect>; 3]>,
    /// Row bounds while Settings is open.
    pub settings_row: Option<Option<Rect>>,
    pub error: Option<String>,
}

impl Snapshot {
    fn id(&self, slot: u64) -> NodeId {
        NodeId(BASE + self.generation * 8 + slot)
    }

    /// Slots: 0 window, 1 dialog, 2..=4 buttons, 5 Settings row, 6 error.
    pub fn tree(&self) -> Option<TreeUpdate> {
        let bounds = self.dialog?;
        let mut root = Node::new(Role::Window);
        root.set_label("Aggiungi al menu");
        let mut dialog = Node::new(Role::Dialog);
        dialog.set_label("Aggiungi al menu");
        let mut nodes = vec![];
        for (i, (_, label)) in LABELS.iter().enumerate() {
            let mut b = Node::new(Role::Button);
            b.set_label(*label);
            if let Some(r) = bounds[i] {
                b.set_bounds(r);
            }
            b.add_action(Action::Click);
            b.add_action(Action::Focus);
            dialog.push_child(self.id(2 + i as u64));
            nodes.push((self.id(2 + i as u64), b));
        }
        if let Some(error) = &self.error {
            let mut e = Node::new(Role::Label);
            e.set_value(error.as_str());
            dialog.push_child(self.id(6));
            nodes.push((self.id(6), e));
        }
        root.push_child(self.id(1));
        nodes.insert(0, (self.id(0), root));
        nodes.insert(1, (self.id(1), dialog));
        Some(TreeUpdate {
            nodes,
            tree: Some(Tree::new(self.id(0))),
            tree_id: TreeId::ROOT,
            focus: self.id(2),
        })
    }

    /// Adds the Settings row to the regular tree, under its root.
    pub fn add_settings_row(&self, tree: &mut TreeUpdate) {
        let Some(bounds) = self.settings_row else {
            return;
        };
        let mut row = Node::new(Role::Button);
        row.set_label("Aggiungi al menu");
        if let Some(r) = bounds {
            row.set_bounds(r);
        }
        row.add_action(Action::Click);
        let Some(root) = tree.tree.as_ref().map(|t| t.root) else {
            return;
        };
        if let Some((_, node)) = tree.nodes.iter_mut().find(|(id, _)| *id == root) {
            node.push_child(self.id(5));
        }
        tree.nodes.push((self.id(5), row));
    }

    pub fn action(&self, request: &ActionRequest) -> Option<Choice> {
        if request.target_tree != TreeId::ROOT
            || request.data.is_some()
            || request.action != Action::Click
        {
            return None;
        }
        if self.settings_row.is_some() && request.target_node == self.id(5) {
            return Some(Choice::Add);
        }
        self.dialog?;
        LABELS
            .iter()
            .enumerate()
            .find(|(i, _)| self.id(2 + *i as u64) == request.target_node)
            .map(|(_, (c, _))| *c)
    }
}
