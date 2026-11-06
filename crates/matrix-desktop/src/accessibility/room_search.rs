//! Room message search («Cerca nei messaggi») for assistive technology. Unlike «Nuova chat» it is not
//! a dialog: the nodes are added to the chat tree while the header field is open: a text input
//! (with the active result as `active_descendant`), a close button, the results list and one status
//! line. Labels only; nothing explains the search.
use super::ROOT;
use crate::new_chat::Status;
use accesskit::{Action, ActionRequest, Live, Node, NodeId, Role, TreeId, TreeUpdate};

/// Far from the room list (from 100 up) and from the history (from 2^62).
const BASE: u64 = 1 << 60;
pub const INPUT: NodeId = NodeId(BASE);
pub const CLOSE: NodeId = NodeId(BASE + 1);
pub const LIST: NodeId = NodeId(BASE + 2);
pub const STATUS: NodeId = NodeId(BASE + 3);
pub const MORE: NodeId = NodeId(BASE + 4);
/// Result `i` is node `OPTION + i`.
pub const OPTION: u64 = BASE + 100;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub text: String,
    /// «mittente, data, estratto», one per result.
    pub labels: Vec<String>,
    pub active: usize,
    /// Whether the list is shown (Esc closes it without closing the search).
    pub list_open: bool,
    pub status: Option<Status>,
    /// «Altri risultati» is offered.
    pub more: bool,
    pub bounds: Option<accesskit::Rect>,
    /// The field holds the keyboard focus.
    pub focused: bool,
}

#[derive(Clone, Debug, Default)]
pub enum RoomSearchView {
    #[default]
    Closed,
    Open(Box<Snapshot>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RoomSearchAction {
    FocusInput,
    Close,
    /// Jump to result `i`.
    Choose(usize),
    More,
}

impl RoomSearchView {
    /// Adds the search nodes to the chat tree (and takes the focus when the field has it).
    pub fn append(&self, tree: &mut TreeUpdate) {
        let Self::Open(s) = self else {
            return;
        };
        let list_shown = s.list_open && !s.labels.is_empty();
        let mut input = Node::new(Role::TextInput);
        input.set_label("Cerca nei messaggi");
        input.set_value(s.text.clone());
        input.add_action(Action::Focus);
        if let Some(bounds) = s.bounds {
            input.set_bounds(bounds);
        }
        if list_shown {
            input.set_controls(vec![LIST]);
            input.set_has_popup(accesskit::HasPopup::Listbox);
            input.set_expanded(true);
            input.set_active_descendant(NodeId(OPTION + s.active as u64));
        }
        let mut close = Node::new(Role::Button);
        close.set_label("Chiudi ricerca");
        close.add_action(Action::Click);
        close.add_action(Action::Focus);
        let mut status = Node::new(Role::Status);
        status.set_live(Live::Polite);
        match (s.status, s.labels.len()) {
            (Some(status_text), _) if s.list_open => status.set_value(status_text.text()),
            (None, n) if n > 0 && s.list_open => status.set_value(if n == 1 {
                "1 risultato".to_owned()
            } else {
                format!("{n} risultati")
            }),
            _ => {}
        }
        let mut ids = vec![INPUT, CLOSE];
        tree.nodes.push((INPUT, input));
        tree.nodes.push((CLOSE, close));
        if list_shown {
            let mut list = Node::new(Role::ListBox);
            list.set_label("Risultati");
            let size = s.labels.len();
            for (i, label) in s.labels.iter().enumerate() {
                let id = NodeId(OPTION + i as u64);
                let mut option = Node::new(Role::ListBoxOption);
                option.set_label(label.clone());
                option.set_selected(i == s.active);
                option.set_position_in_set(i + 1);
                option.set_size_of_set(size);
                option.add_action(Action::Click);
                list.push_child(id);
                tree.nodes.push((id, option));
            }
            if s.more {
                let mut more = Node::new(Role::Button);
                more.set_label("Altri risultati");
                more.add_action(Action::Click);
                list.push_child(MORE);
                tree.nodes.push((MORE, more));
            }
            tree.nodes.push((LIST, list));
            ids.push(LIST);
        }
        tree.nodes.push((STATUS, status));
        ids.push(STATUS);
        for id in ids {
            tree.nodes[0].1.push_child(id);
        }
        debug_assert_eq!(tree.nodes[0].0, ROOT);
        if s.focused {
            tree.focus = INPUT;
        }
    }

    pub fn action(&self, request: &ActionRequest) -> Option<RoomSearchAction> {
        let Self::Open(s) = self else {
            return None;
        };
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        match (request.action, request.target_node) {
            (Action::Focus, INPUT) => Some(RoomSearchAction::FocusInput),
            (Action::Click | Action::Focus, CLOSE) => Some(RoomSearchAction::Close),
            (Action::Click, MORE) if s.more => Some(RoomSearchAction::More),
            (Action::Click, NodeId(n)) if n >= OPTION && ((n - OPTION) as usize) < s.labels.len() => {
                Some(RoomSearchAction::Choose((n - OPTION) as usize))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Gnobbi");
        TreeUpdate { nodes: vec![(ROOT, root)], tree: None, tree_id: TreeId::ROOT, focus: ROOT }
    }

    fn open(labels: &[&str], list_open: bool, status: Option<Status>) -> RoomSearchView {
        RoomSearchView::Open(Box::new(Snapshot {
            text: "ciao".into(),
            labels: labels.iter().map(|l| (*l).to_owned()).collect(),
            active: 1,
            list_open,
            status,
            more: true,
            bounds: None,
            focused: true,
        }))
    }

    fn node(tree: &TreeUpdate, id: NodeId) -> &Node {
        &tree.nodes.iter().find(|(n, _)| *n == id).unwrap_or_else(|| panic!("node {id:?}")).1
    }

    #[test]
    fn field_list_and_results_have_the_expected_roles_and_labels() {
        let mut t = tree();
        open(&["Anna, ieri, ciao a tutti", "Bo, oggi, ciao Anna"], true, None).append(&mut t);
        assert_eq!(t.focus, INPUT);
        let input = node(&t, INPUT);
        assert_eq!((input.role(), input.label(), input.value()), (Role::TextInput, Some("Cerca nei messaggi"), Some("ciao")));
        assert_eq!(input.active_descendant(), Some(NodeId(OPTION + 1)));
        assert_eq!(input.controls(), [LIST]);
        assert_eq!(node(&t, CLOSE).label(), Some("Chiudi ricerca"));
        let list = node(&t, LIST);
        assert_eq!((list.role(), list.label()), (Role::ListBox, Some("Risultati")));
        assert_eq!(list.children(), [NodeId(OPTION), NodeId(OPTION + 1), MORE]);
        let second = node(&t, NodeId(OPTION + 1));
        assert_eq!((second.role(), second.is_selected(), second.position_in_set(), second.size_of_set()), (Role::ListBoxOption, Some(true), Some(2), Some(2)));
        assert_eq!(node(&t, STATUS).value(), Some("2 risultati"));
        // Every node hangs off the root exactly once.
        let kids = t.nodes[0].1.children().to_vec();
        for id in [INPUT, CLOSE, LIST, STATUS] {
            assert_eq!(kids.iter().filter(|k| **k == id).count(), 1);
        }
    }

    #[test]
    fn closed_list_and_empty_state_announce_only_the_label() {
        let mut t = tree();
        open(&["a"], false, None).append(&mut t);
        assert!(t.nodes.iter().all(|(id, _)| *id != LIST));
        assert_eq!(node(&t, INPUT).controls(), []);
        assert_eq!(t.focus, INPUT, "the field keeps the focus when only the list closes");
        let mut t = tree();
        open(&[], true, Some(Status::NoResults)).append(&mut t);
        assert!(t.nodes.iter().all(|(id, _)| *id != LIST));
        assert_eq!(node(&t, STATUS).value(), Some("Nessun risultato"));
    }

    #[test]
    fn actions_map_to_intents() {
        let view = open(&["a", "b"], true, None);
        let req = |action, node| ActionRequest { action, target_tree: TreeId::ROOT, target_node: node, data: None };
        assert_eq!(view.action(&req(Action::Focus, INPUT)), Some(RoomSearchAction::FocusInput));
        assert_eq!(view.action(&req(Action::Click, CLOSE)), Some(RoomSearchAction::Close));
        assert_eq!(view.action(&req(Action::Click, NodeId(OPTION + 1))), Some(RoomSearchAction::Choose(1)));
        assert_eq!(view.action(&req(Action::Click, NodeId(OPTION + 2))), None);
        assert_eq!(view.action(&req(Action::Click, MORE)), Some(RoomSearchAction::More));
        assert_eq!(RoomSearchView::Closed.action(&req(Action::Click, CLOSE)), None);
    }
}
