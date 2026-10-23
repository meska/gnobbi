//! Link activation is a request, never permission to launch a browser.
use crate::MessageLinkTarget;
use accesskit::{Action, Node, NodeId, Role};

#[derive(Clone)]
pub struct Snapshot {
    pub target: MessageLinkTarget,
    pub bounds: Option<accesskit::Rect>,
    pub focused: bool,
    pub requested: bool,
}

impl Snapshot {
    pub fn id(&self) -> NodeId {
        NodeId(
            (1_u64 << 60)
                .checked_add(self.target.instance())
                .expect("link node IDs exhausted"),
        )
    }
    pub fn node(&self) -> Node {
        let mut node = Node::new(Role::Link);
        node.set_label(format!("Apri link: {}", self.target.url().as_str()));
        node.set_url(self.target.url().as_str());
        node.set_description(if self.requested {
            "Conferma apertura link richiesta"
        } else {
            "Richiede conferma prima di aprire nel browser esterno"
        });
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
        if let Some(bounds) = self.bounds {
            node.set_bounds(bounds);
        }
        node
    }
}

impl super::HistorySnapshot {
    pub fn link_key(&self, key: &str, shift: bool) -> Option<super::NavigationAction> {
        use super::NavigationAction;
        if !self.focused {
            return None;
        }
        let focused = self
            .messages
            .iter()
            .flat_map(|m| &m.links)
            .find(|l| l.focused);
        let (link, focus_only) = match (key, shift, focused) {
            ("f9", false, _) => (self.messages.first()?.links.first()?, true),
            ("enter" | "space", false, Some(link)) => (link, false),
            ("escape", false, Some(link)) => {
                return Some(NavigationAction::FocusMessage(link.target.event().clone()));
            }
            ("tab", shift, Some(link)) => {
                let links = &self
                    .messages
                    .iter()
                    .find(|m| m.event == *link.target.event())?
                    .links;
                let index = links.iter().position(|l| l.id() == link.id())?;
                if shift && index == 0 {
                    return Some(NavigationAction::FocusMessage(link.target.event().clone()));
                }
                let next = if shift { index - 1 } else { index + 1 };
                let Some(next) = links.get(next) else {
                    return Some(NavigationAction::FocusComposer);
                };
                (next, true)
            }
            _ => return None,
        };
        Some(NavigationAction::MessageLink(Intent {
            target: link.target.clone(),
            focus_only,
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationControl {
    Cancel,
    Open,
}

#[derive(Clone)]
pub struct ConfirmationSnapshot {
    pub request: crate::LinkConfirmationRequest,
    pub focused: Option<ConfirmationControl>,
    /// Address, Cancel, Open; observed physical bounds.
    pub bounds: [Option<accesskit::Rect>; 3],
}

#[derive(Clone)]
pub struct ConfirmationIntent {
    pub request: crate::LinkConfirmationRequest,
    pub control: ConfirmationControl,
    pub focus_only: bool,
}

impl ConfirmationSnapshot {
    pub fn key(&self, key: &str, shift: bool) -> Option<ConfirmationIntent> {
        let focused = self.focused?;
        let (control, focus_only) = match (key, shift) {
            ("tab", _) => (
                if focused == ConfirmationControl::Cancel {
                    ConfirmationControl::Open
                } else {
                    ConfirmationControl::Cancel
                },
                true,
            ),
            ("escape", false) => (ConfirmationControl::Cancel, false),
            ("space" | "enter", false) => (focused, false),
            _ => return None,
        };
        Some(ConfirmationIntent {
            request: self.request.clone(),
            control,
            focus_only,
        })
    }

    fn id(&self, offset: u64) -> NodeId {
        NodeId(
            (1_u64 << 59)
                .checked_add(
                    self.request
                        .generation()
                        .checked_mul(4)
                        .expect("confirmation IDs exhausted"),
                )
                .and_then(|n| n.checked_add(offset))
                .expect("confirmation IDs exhausted"),
        )
    }

    pub fn append_to(&self, tree: &mut accesskit::TreeUpdate) {
        let mut address = Node::new(Role::Label);
        address.set_value(self.request.url().as_str());
        address.set_description("Conferma apertura link richiesta");
        if let Some(bounds) = self.bounds[0] {
            address.set_bounds(bounds);
        }
        tree.nodes[0].1.push_child(self.id(0));
        tree.nodes.push((self.id(0), address));
        for (i, control, label) in [
            (1, ConfirmationControl::Cancel, "Annulla apertura link"),
            (
                2,
                ConfirmationControl::Open,
                "Apri link nel browser esterno",
            ),
        ] {
            let id = self.id(i);
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.set_description(format!(
                "Conferma apertura link richiesta: {}",
                self.request.url().as_str()
            ));
            node.add_action(Action::Focus);
            node.add_action(Action::Click);
            if let Some(bounds) = self.bounds[i as usize] {
                node.set_bounds(bounds);
            }
            tree.nodes[0].1.push_child(id);
            tree.nodes.push((id, node));
            if self.focused == Some(control) {
                tree.focus = id;
            }
        }
    }

    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<ConfirmationIntent> {
        if request.target_tree != accesskit::TreeId::ROOT
            || request.data.is_some()
            || !matches!(request.action, Action::Focus | Action::Click)
        {
            return None;
        }
        let control = if request.target_node == self.id(1) {
            ConfirmationControl::Cancel
        } else if request.target_node == self.id(2) {
            ConfirmationControl::Open
        } else {
            return None;
        };
        Some(ConfirmationIntent {
            request: self.request.clone(),
            control,
            focus_only: request.action == Action::Focus,
        })
    }
}

#[derive(Clone)]
pub struct Intent {
    pub target: MessageLinkTarget,
    pub focus_only: bool,
}
