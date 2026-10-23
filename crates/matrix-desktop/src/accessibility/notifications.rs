//! Session-only notification preferences; no message content or network actions.
use super::ROOT;
use accesskit::{Action, ActionRequest, Node, NodeId, Role, Toggled, Tree, TreeId, TreeUpdate};
use matrix_domain::RoomScope;

pub const ENABLED: NodeId = NodeId(21);
pub const MUTE: NodeId = NodeId(22);
pub const PREVIEW: NodeId = NodeId(23);
pub const CLOSE: NodeId = NodeId(24);
pub const OPEN: NodeId = NodeId(27);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationControl {
    #[default]
    Enabled,
    Mute,
    Preview,
    Close,
}
impl NotificationControl {
    pub fn id(self) -> NodeId {
        match self {
            Self::Enabled => ENABLED,
            Self::Mute => MUTE,
            Self::Preview => PREVIEW,
            Self::Close => CLOSE,
        }
    }
}

#[derive(Clone)]
pub struct NotificationSnapshot {
    pub bounds: [Option<accesskit::Rect>; 4],
    pub focus: NotificationControl,
    pub enabled: bool,
    pub preview: bool,
    pub selected: Option<(RoomScope, bool)>,
    pub service_failed: bool,
}
impl NotificationSnapshot {
    pub fn next_focus(&self, backwards: bool) -> NotificationControl {
        use NotificationControl::*;
        let controls = if self.selected.is_some() {
            vec![Enabled, Mute, Preview, Close]
        } else {
            vec![Enabled, Preview, Close]
        };
        let index = controls.iter().position(|c| *c == self.focus).unwrap_or(0);
        controls[(index + if backwards { controls.len() - 1 } else { 1 }) % controls.len()]
    }
}
#[derive(Clone, Default)]
pub enum NotificationView {
    #[default]
    Unavailable,
    Closed,
    Open(Box<NotificationSnapshot>),
}
#[derive(Debug, PartialEq)]
pub enum NotificationAction {
    Focus(NotificationControl),
    Open,
    Close,
    SetEnabled(bool),
    SetPreview(bool),
    SetMuted { scope: RoomScope, muted: bool },
}
impl NotificationView {
    pub fn tree(&self) -> Option<TreeUpdate> {
        let Self::Open(panel) = self else {
            return None;
        };
        let mut root = Node::new(Role::Window);
        root.set_label("Notifiche locali");
        let mut nodes = vec![];
        for (index, (id, label, checked)) in [
            (ENABLED, "Notifiche locali attive", panel.enabled),
            (
                MUTE,
                "Stanza selezionata silenziata",
                panel.selected.as_ref().is_some_and(|(_, muted)| *muted),
            ),
            (PREVIEW, "Anteprima del solo conteggio", panel.preview),
        ]
        .into_iter()
        .enumerate()
        {
            let mut node = Node::new(Role::CheckBox);
            node.set_label(label);
            if let Some(bounds) = panel.bounds[index] {
                node.set_bounds(bounds);
            }
            node.set_toggled(if checked {
                Toggled::True
            } else {
                Toggled::False
            });
            if id == MUTE && panel.selected.is_none() {
                node.set_disabled();
            } else {
                node.add_action(Action::Click);
                node.add_action(Action::Focus);
            }
            root.push_child(id);
            nodes.push((id, node));
        }
        let mut close = Node::new(Role::Button);
        close.set_label("Chiudi notifiche");
        if let Some(bounds) = panel.bounds[3] {
            close.set_bounds(bounds);
        }
        close.add_action(Action::Click);
        close.add_action(Action::Focus);
        root.push_child(CLOSE);
        nodes.push((CLOSE, close));
        let mut privacy = Node::new(Role::Label);
        privacy.set_value("Solo questa sessione. Anteprima solo del conteggio. Mai testo, mittente, stanza, media o segreti nelle notifiche OS. Nessun push a client chiuso.");
        root.push_child(NodeId(25));
        nodes.push((NodeId(25), privacy));
        let mut service = Node::new(Role::Label);
        service.set_value(if panel.service_failed { "Servizio OS non disponibile o richiesta scaduta. Gli avvisi sono best effort." } else { "Linux: richiede notify-send e un servizio notifiche della sessione. Windows/macOS non implementati." });
        root.push_child(NodeId(26));
        nodes.push((NodeId(26), service));
        nodes.insert(0, (ROOT, root));
        Some(TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: if panel.focus == NotificationControl::Mute && panel.selected.is_none() {
                ENABLED
            } else {
                panel.focus.id()
            },
        })
    }
    pub fn action(&self, request: &ActionRequest) -> Option<NotificationAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        if request.action == Action::Focus {
            let Self::Open(panel) = self else {
                return None;
            };
            return [
                NotificationControl::Enabled,
                NotificationControl::Mute,
                NotificationControl::Preview,
                NotificationControl::Close,
            ]
            .into_iter()
            .find(|control| {
                control.id() == request.target_node
                    && (*control != NotificationControl::Mute || panel.selected.is_some())
            })
            .map(NotificationAction::Focus);
        }
        if request.action != Action::Click {
            return None;
        }
        match (self, request.target_node) {
            (Self::Closed, OPEN) => Some(NotificationAction::Open),
            (Self::Open(_), CLOSE) => Some(NotificationAction::Close),
            (Self::Open(panel), ENABLED) => Some(NotificationAction::SetEnabled(!panel.enabled)),
            (Self::Open(panel), PREVIEW) => Some(NotificationAction::SetPreview(!panel.preview)),
            (Self::Open(panel), MUTE) => {
                panel
                    .selected
                    .as_ref()
                    .map(|(scope, muted)| NotificationAction::SetMuted {
                        scope: scope.clone(),
                        muted: !muted,
                    })
            }
            _ => None,
        }
    }
    /// Revalidate queue commands against current UI state, especially selected scope.
    pub fn allows(&self, action: &NotificationAction) -> bool {
        match (self, action) {
            (Self::Open(panel), NotificationAction::Focus(control)) => {
                *control != NotificationControl::Mute || panel.selected.is_some()
            }
            (Self::Closed, NotificationAction::Open)
            | (Self::Open(_), NotificationAction::Close) => true,
            (Self::Open(panel), NotificationAction::SetEnabled(value)) => panel.enabled != *value,
            (Self::Open(panel), NotificationAction::SetPreview(value)) => panel.preview != *value,
            (Self::Open(panel), NotificationAction::SetMuted { scope, muted }) => panel
                .selected
                .as_ref()
                .is_some_and(|(current, state)| current == scope && state != muted),
            _ => false,
        }
    }
}
