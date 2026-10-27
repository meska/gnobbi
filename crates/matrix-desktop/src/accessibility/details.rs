//! Read-only room sheet. Semantic actions never mutate homeserver state.
use super::ROOT;
use accesskit::{Action, Node, NodeId, Role, Tree, TreeId, TreeUpdate};
use matrix_domain::RoomScope;

pub const OPEN: NodeId = NodeId(30);
#[derive(Clone, Default)]
pub enum DetailsView {
    #[default]
    Unavailable,
    Closed(RoomScope),
    Open(Box<DetailsSnapshot>),
}
#[derive(Debug, PartialEq)]
pub enum DetailsAction {
    Open(RoomScope),
    Close(RoomScope),
    Copy(RoomScope),
    Focus {
        scope: RoomScope,
        control: DetailsControl,
    },
}
impl DetailsView {
    pub fn action(&self, request: &accesskit::ActionRequest) -> Option<DetailsAction> {
        if request.target_tree != TreeId::ROOT || request.data.is_some() {
            return None;
        }
        match self {
            Self::Closed(scope)
                if request.target_node == OPEN && request.action == Action::Click =>
            {
                Some(DetailsAction::Open(scope.clone()))
            }
            Self::Open(panel) => {
                let control = [DetailsControl::Close, DetailsControl::Copy]
                    .into_iter()
                    .find(|c| c.id() == request.target_node)?;
                match request.action {
                    Action::Focus => Some(DetailsAction::Focus {
                        scope: panel.scope.clone(),
                        control,
                    }),
                    Action::Click => Some(match control {
                        DetailsControl::Close => DetailsAction::Close(panel.scope.clone()),
                        DetailsControl::Copy => DetailsAction::Copy(panel.scope.clone()),
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    pub fn allows(&self, action: &DetailsAction) -> bool {
        match (self, action) {
            (Self::Closed(current), DetailsAction::Open(scope)) => current == scope,
            (
                Self::Open(panel),
                DetailsAction::Close(scope)
                | DetailsAction::Copy(scope)
                | DetailsAction::Focus { scope, .. },
            ) => panel.scope == *scope,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetailsControl {
    #[default]
    Close,
    Copy,
}
impl DetailsControl {
    pub fn id(self) -> NodeId {
        match self {
            Self::Close => NodeId(31),
            Self::Copy => NodeId(32),
        }
    }
}
#[derive(Clone)]
pub struct DetailsSnapshot {
    pub scope: RoomScope,
    pub name: String,
    pub info: matrix_engine::RoomInfo,
    pub focus: DetailsControl,
    pub bounds: [Option<accesskit::Rect>; 2],
}
impl DetailsSnapshot {
    pub fn next_focus(&self) -> DetailsControl {
        match self.focus {
            DetailsControl::Close => DetailsControl::Copy,
            DetailsControl::Copy => DetailsControl::Close,
        }
    }
    pub fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label("Informazioni stanza");
        let mut nodes = vec![];
        for (index, label) in [
            self.name.clone(),
            if self.info.is_space {
                "Spazio Matrix".into()
            } else {
                "Stanza Matrix".into()
            },
            format!(
                "{} {}",
                self.info.joined_members,
                if self.info.joined_members == 1 {
                    "membro"
                } else {
                    "membri"
                }
            ),
            format!(
                "Argomento: {}",
                self.info.topic.as_deref().unwrap_or("Nessun argomento")
            ),
            match self.info.encrypted {
                Some(true) => "Cifratura E2EE attiva",
                Some(false) => "Cifratura E2EE non attiva",
                None => "Cifratura non confermata dallo stato SDK",
            }
            .into(),
            self.scope.room_id.as_str().to_owned(),
            "Stato dell'ultima sync. Nessuna modifica, invito o adesione automatica.".into(),
        ]
        .into_iter()
        .enumerate()
        {
            let id = NodeId(33 + index as u64);
            let mut node = Node::new(Role::Label);
            node.set_value(label);
            root.push_child(id);
            nodes.push((id, node));
        }
        for (index, (control, label)) in [
            (DetailsControl::Close, "Chiudi informazioni stanza"),
            (DetailsControl::Copy, "Copia ID stanza"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut node = Node::new(Role::Button);
            node.set_label(label);
            node.add_action(Action::Click);
            node.add_action(Action::Focus);
            if let Some(bounds) = self.bounds[index] {
                node.set_bounds(bounds);
            }
            root.push_child(control.id());
            nodes.push((control.id(), node));
        }
        nodes.insert(0, (ROOT, root));
        TreeUpdate {
            nodes,
            tree: Some(Tree::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: self.focus.id(),
        }
    }
}
