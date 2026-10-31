//! «Nuova chat» for assistive technology. While the dialog is open it replaces the chat tree
//! (like the notification panel): a dialog with the search field, the result list and one status line.
use super::ROOT;
use crate::new_chat::{Entry, Status};
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
        let selected = s
            .entries
            .get(s.active)
            .map(|_| NodeId(OPTION + s.active as u64));
        if let Some(id) = selected {
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
        // The list exists only with entries, so no empty listbox is announced.
        if !s.entries.is_empty() {
            let mut list = Node::new(Role::ListBox);
            list.set_label("Risultati");
            let size = s.entries.len();
            for (i, entry) in s.entries.iter().enumerate() {
                let id = NodeId(OPTION + i as u64);
                let mut option = Node::new(Role::ListBoxOption);
                option.set_label(entry.label());
                option.set_selected(i == s.active);
                option.set_position_in_set(i + 1);
                option.set_size_of_set(size);
                option.add_action(Action::Click);
                list.push_child(id);
                nodes.push((id, option));
            }
            leaf(&mut root, &mut nodes, LIST, list);
        }
        // A polite live region: the screen reader announces each state change.
        let mut status = Node::new(Role::Status);
        status.set_live(Live::Polite);
        if let Some(status_text) = s.status {
            status.set_value(status_text.text());
        } else if !s.entries.is_empty() {
            status.set_value(format!("{} risultati", s.entries.len()));
        }
        leaf(&mut root, &mut nodes, STATUS, status);
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
            (Action::Click, NodeId(n)) if n >= OPTION => s
                .entries
                .get((n - OPTION) as usize)
                .map(|e| NewChatAction::Choose(e.user_id.clone())),
            _ => None,
        }
    }
}
