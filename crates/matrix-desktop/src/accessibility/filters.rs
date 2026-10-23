//! Local, account-scoped room filtering; never changes the selected conversation.
use accesskit::{Action, Node, NodeId, Role, Toggled};
use matrix_domain::AccountId;

pub const UNREAD: NodeId = NodeId(50);

#[derive(Clone)]
pub struct UnreadFilter {
    pub account: AccountId,
    pub revision: u64,
    pub checked: bool,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterAction {
    Focus(AccountId, u64),
    SetUnread {
        account: AccountId,
        revision: u64,
        checked: bool,
    },
}

impl UnreadFilter {
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<FilterAction> {
        if request.target_tree != accesskit::TreeId::ROOT
            || request.target_node != UNREAD
            || request.data.is_some()
        {
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
            FilterAction::SetUnread {
                account,
                revision,
                checked,
            } => *account == self.account && *revision == self.revision && *checked != self.checked,
        }
    }

    pub fn node(&self) -> (NodeId, Node) {
        let mut node = Node::new(Role::CheckBox);
        node.set_label("Solo stanze non lette");
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
}
