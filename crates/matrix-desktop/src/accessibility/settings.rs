//! Drawer settings: on/off switches (debug, show deleted messages, close to tray; read receipts live on the privacy page).
use super::ROOT;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};

pub const LABELS: [&str; 4] = [
    "Modalità debug",
    "Conferme di lettura",
    "Messaggi eliminati",
    "Riduci nel tray alla chiusura",
];
/// Row of the «Privacy e sicurezza» button (not a switch).
pub const PRIVACY: usize = crate::SETTINGS_PRIVACY_ROW;
const BASE: u64 = 70;

fn id(row: usize) -> NodeId {
    NodeId(BASE + row as u64)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SettingsSnapshot {
    pub focus: usize,
    pub on: [bool; 4],
    pub update_check: Option<bool>,
    /// «Privacy e sicurezza» page: only «Conferme di lettura» is exposed; Settings no longer lists it.
    pub privacy_page: bool,
}
#[derive(Debug, PartialEq)]
pub enum SettingsAction {
    Focus(usize),
    OpenPrivacy,
    /// Explicit target state: a stale or duplicated request cannot toggle back.
    Set(usize, bool),
}
impl SettingsSnapshot {
    pub fn state(&self, row: usize) -> Option<bool> {
        self.on.get(row).copied().or_else(|| (row == 4).then_some(self.update_check).flatten())
    }

    // shortcut: no bounds on the nodes, so pointer-based AT hit testing finds nothing; add canvas observers like notification_view when needed.
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Impostazioni");
        let mut nodes = vec![];
        for (row, label) in LABELS.into_iter().enumerate() {
            if self.privacy_page != (row == 1) {
                continue;
            }
            let mut node = Node::new(Role::Switch);
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
            if row == 0 {
                let mut node = Node::new(Role::Button);
                node.set_label("Privacy e sicurezza");
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                root.push_child(id(PRIVACY));
                nodes.push((id(PRIVACY), node));
            }
        }
        if let Some(on) = self.update_check.filter(|_| !self.privacy_page) {
            let mut node = Node::new(Role::Switch);
            node.set_label(crate::update::label::update_check_label());
            node.set_toggled(if on { Toggled::True } else { Toggled::False });
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            root.push_child(id(4));
            nodes.push((id(4), node));
        }
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: if self.privacy_page {
                if self.focus == 1 { id(1) } else { ROOT } // «Dispositivi» has no node yet
            } else {
                id(if self.focus == PRIVACY { PRIVACY } else { self.focus.min(if self.update_check.is_some() { 4 } else { 3 }) })
            },
        }
    }
    pub fn action(&self, request: &ActionRequest) -> Option<SettingsAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        if request.target_node == id(PRIVACY) && !self.privacy_page {
            return match request.action {
                Action::Focus => Some(SettingsAction::Focus(PRIVACY)),
                Action::Click => Some(SettingsAction::OpenPrivacy),
                _ => None,
            };
        }
        let row = (0..5).find(|r| id(*r) == request.target_node && self.state(*r).is_some() && self.privacy_page == (*r == 1))?;
        match request.action {
            Action::Focus => Some(SettingsAction::Focus(row)),
            Action::Click => Some(SettingsAction::Set(row, !self.state(row)?)),
            _ => None,
        }
    }
}
