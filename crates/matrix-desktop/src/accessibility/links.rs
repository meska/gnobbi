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

/// A Tab stop inside a bubble: a link or a mention pill.
#[derive(Clone, Copy)]
pub enum Stop<'a> {
    Link(&'a Snapshot),
    Pill(usize, &'a super::PillLink),
}

impl Stop<'_> {
    pub fn focused(&self) -> bool {
        match self {
            Stop::Link(l) => l.focused,
            Stop::Pill(_, p) => p.focused,
        }
    }

    pub fn node(&self, message: &super::SemanticMessage) -> (NodeId, Node) {
        match self {
            Stop::Link(l) => (l.id(), l.node()),
            Stop::Pill(i, p) => {
                let mut node = Node::new(Role::Button);
                node.set_label(p.name.clone());
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
                if let Some(bounds) = p.bounds {
                    node.set_bounds(bounds);
                }
                (message.pill_id(*i), node)
            }
        }
    }

    fn go(&self, message: &super::SemanticMessage, focus_only: bool) -> super::NavigationAction {
        use super::NavigationAction;
        match self {
            Stop::Link(l) => NavigationAction::MessageLink(Intent { target: l.target.clone(), focus_only }),
            Stop::Pill(_, p) if focus_only => NavigationAction::FocusPill(message.event.clone(), p.start),
            Stop::Pill(_, p) => NavigationAction::OpenProfile(p.user_id.clone()),
        }
    }
}

impl super::SemanticMessage {
    /// Links and pills in text order.
    pub fn stops(&self) -> Vec<Stop<'_>> {
        let mut stops: Vec<(usize, Stop)> = self
            .links
            .iter()
            .map(|l| (l.target.range().start, Stop::Link(l)))
            .chain(self.pills.iter().take(super::PILLS).enumerate().map(|(i, p)| (p.start, Stop::Pill(i, p))))
            .collect();
        stops.sort_by_key(|(at, _)| *at);
        stops.into_iter().map(|(_, s)| s).collect()
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
            .find_map(|m| m.stops().into_iter().position(|s| s.focused()).map(|i| (m, i)));
        let (message, stop, focus_only) = match (key, shift, focused) {
            ("f9", false, _) => {
                let m = self.messages.first()?;
                (m, *m.stops().first()?, true)
            }
            ("enter" | "space", false, Some((m, i))) => (m, m.stops()[i], false),
            ("escape", false, Some((m, _))) => {
                return Some(NavigationAction::FocusMessage(m.event.clone()));
            }
            ("tab", shift, Some((m, i))) => {
                if shift && i == 0 {
                    return Some(NavigationAction::FocusMessage(m.event.clone()));
                }
                let next = if shift { i - 1 } else { i + 1 };
                let Some(next) = m.stops().get(next).copied() else {
                    return Some(NavigationAction::FocusComposer);
                };
                (m, next, true)
            }
            _ => return None,
        };
        Some(stop.go(message, focus_only))
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
