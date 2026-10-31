//! «Nuova chat» for assistive technology. While the dialog is open it replaces the chat tree
//! (like the notification panel): a dialog with the search field, the result list and one status line.
use super::ROOT;
use super::search_list::{self, Ids};
use crate::new_chat::{Entry, Status};
use crate::search_list::{Hit, Row};
use accesskit::{
    Action, ActionRequest, Live, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate,
};

pub const INPUT: NodeId = NodeId(100);
pub const LIST: NodeId = NodeId(101);
pub const STATUS: NodeId = NodeId(102);
pub const SWITCH: NodeId = NodeId(103);
pub const CANCEL: NodeId = NodeId(104);
pub const START: NodeId = NodeId(105);
/// Result `i` is node `OPTION + i`.
pub const OPTION: u64 = 1000;
const IDS: Ids = Ids {
    list: LIST,
    status: STATUS,
    option: OPTION,
};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub text: String,
    pub entries: Vec<Entry>,
    pub active: usize,
    pub status: Option<Status>,
    /// `None`: the server does not allow searching beyond the own instance, no switch.
    pub all: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub enum NewChatView {
    #[default]
    Closed,
    Open(Box<Snapshot>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum NewChatAction {
    FocusInput,
    /// Start a chat with this user ID.
    Choose(String),
    ToggleAll,
    Cancel,
}

impl NewChatView {
    pub fn tree(&self) -> Option<TreeUpdate> {
        let Self::Open(s) = self else {
            return None;
        };
        let mut root = Node::new(Role::Dialog);
        root.set_label("Nuova chat");
        let mut nodes = Vec::new();
        fn leaf(root: &mut Node, nodes: &mut Vec<(NodeId, Node)>, id: NodeId, node: Node) {
            root.push_child(id);
            nodes.push((id, node));
        }
        let mut input = Node::new(Role::TextInput);
        input.set_label("ID utente");
        input.set_value(s.text.clone());
        input.add_action(Action::Focus);
        input.set_controls(vec![LIST]);
        input.set_has_popup(accesskit::HasPopup::Listbox);
        let rows: Vec<_> = s.entries.iter().map(Row::Hit).collect();
        let list = search_list::project(IDS, &rows, s.active, s.status);
        if let Some(id) = list.active {
            input.set_active_descendant(id);
        }
        leaf(&mut root, &mut nodes, INPUT, input);
        if let Some(all) = s.all {
            let mut switch = Node::new(Role::Switch);
            switch.set_label("Cerca anche su altre istanze");
            switch.set_toggled(if all { Toggled::True } else { Toggled::False });
            switch.add_action(Action::Click);
            switch.add_action(Action::Focus);
            leaf(&mut root, &mut nodes, SWITCH, switch);
        }
        if !list.list.is_empty() {
            root.push_child(LIST);
            nodes.extend(list.list);
        }
        leaf(&mut root, &mut nodes, list.status.0, list.status.1);
        for (id, label) in [(CANCEL, "Annulla"), (START, "Avvia")] {
            let mut button = Node::new(Role::Button);
            button.set_label(label);
            button.add_action(Action::Click);
            button.add_action(Action::Focus);
            leaf(&mut root, &mut nodes, id, button);
        }
        nodes.insert(0, (ROOT, root));
        Some(TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: INPUT,
        })
    }

    pub fn action(&self, request: &ActionRequest) -> Option<NewChatAction> {
        let Self::Open(s) = self else {
            return None;
        };
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        let id = request.target_node;
        match (request.action, id) {
            (Action::Focus, INPUT) => Some(NewChatAction::FocusInput),
            (Action::Click, SWITCH) if s.all.is_some() => Some(NewChatAction::ToggleAll),
            (Action::Click, CANCEL) => Some(NewChatAction::Cancel),
            (Action::Click, START) => s
                .entries
                .get(s.active)
                .map(|e| e.user_id.clone())
                .or_else(|| Some(s.text.trim().to_owned()).filter(|t| !t.is_empty()))
                .map(NewChatAction::Choose),
            (Action::Click, node) => search_list::index_of(IDS, node, s.entries.len())
                .map(|i| NewChatAction::Choose(s.entries[i].user_id.clone())),
            _ => None,
        }
    }
}
