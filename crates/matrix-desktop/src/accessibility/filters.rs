//! Local, account-scoped room filtering; never changes the selected conversation.
use accesskit::{Action, Node, NodeId, Role, Toggled};
use matrix_domain::AccountId;

pub const UNREAD: NodeId = NodeId(50);
/// Node of the row button at position `i` (the unread one keeps `UNREAD`); ids 51..60 are reserved.
const fn button_node(i: usize) -> NodeId {
    NodeId(51 + i as u64)
}

/// A button of the filter row other than «Non lette»: scopes are radios, «Vuote» a switch.
#[derive(Clone)]
pub struct FilterNode {
    /// Position in `Sidebar::filter_buttons()`.
    pub index: usize,
    pub name: String,
    pub selected: bool,
    pub switch: bool,
}

#[derive(Clone)]
pub struct UnreadFilter {
    pub account: AccountId,
    pub revision: u64,
    pub checked: bool,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
    /// Accessible name of the unread button («Non lette, 12»).
    pub name: String,
    /// All the other buttons of the row.
    pub buttons: Vec<FilterNode>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterAction {
    Focus(AccountId, u64),
    /// Click on a row button other than «Non lette»; `index` is its position in the row.
    Activate {
        account: AccountId,
        revision: u64,
        index: usize,
    },
    SetUnread {
        account: AccountId,
        revision: u64,
        checked: bool,
    },
}

impl UnreadFilter {
    pub fn from_buttons(
        account: AccountId,
        revision: u64,
        focused: bool,
        bounds: Option<accesskit::Rect>,
        row: &[crate::FilterButton],
    ) -> Self {
        let unread = row.iter().find(|b| b.id == crate::FilterId::Unread);
        Self {
            account,
            revision,
            checked: unread.is_some_and(|b| b.selected),
            focused,
            bounds,
            name: unread.map_or_else(|| "Non lette".into(), |b| b.accessible_name()),
            buttons: row
                .iter()
                .enumerate()
                .filter(|(_, b)| b.id != crate::FilterId::Unread)
                .map(|(index, b)| FilterNode {
                    index,
                    name: b.accessible_name(),
                    selected: b.selected,
                    switch: b.id == crate::FilterId::Empty,
                })
                .collect(),
        }
    }

    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<FilterAction> {
        if request.target_tree != accesskit::TreeId::ROOT || request.data.is_some() {
            return None;
        }
        if let Some(button) = self.buttons.iter().find(|b| button_node(b.index) == request.target_node) {
            return match request.action {
                Action::Focus => Some(FilterAction::Focus(self.account, self.revision)),
                Action::Click => Some(FilterAction::Activate {
                    account: self.account,
                    revision: self.revision,
                    index: button.index,
                }),
                _ => None,
            };
        }
        if request.target_node != UNREAD {
            return None;
        }
        match request.action {
            Action::Focus => Some(FilterAction::Focus(self.account, self.revision)),
            Action::Click => Some(FilterAction::SetUnread {
                account: self.account,
                revision: self.revision,
                checked: !self.checked,
            }),
            _ => None,
        }
    }
    pub fn allows(&self, action: &FilterAction) -> bool {
        match action {
            FilterAction::Focus(account, revision) => {
                *account == self.account && *revision == self.revision
            }
            FilterAction::Activate {
                account,
                revision,
                index,
            } => {
                *account == self.account
                    && *revision == self.revision
                    && self.buttons.iter().any(|b| b.index == *index)
            }
            FilterAction::SetUnread {
                account,
                revision,
                checked,
            } => *account == self.account && *revision == self.revision && *checked != self.checked,
        }
    }

    pub fn node(&self) -> (NodeId, Node) {
        let mut node = Node::new(Role::Switch);
        node.set_label(self.name.as_str());
        node.set_toggled(if self.checked {
            Toggled::True
        } else {
            Toggled::False
        });
        if let Some(bounds) = self.bounds {
            node.set_bounds(bounds);
        }
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
        (UNREAD, node)
    }

    /// Every node of the row: «Non lette» plus the other buttons.
    pub fn nodes(&self) -> Vec<(NodeId, Node)> {
        let mut nodes = vec![self.node()];
        for b in &self.buttons {
            let mut node = Node::new(if b.switch { Role::Switch } else { Role::RadioButton });
            node.set_label(b.name.as_str());
            node.set_toggled(if b.selected { Toggled::True } else { Toggled::False });
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
            nodes.push((button_node(b.index), node));
        }
        nodes
    }
}
