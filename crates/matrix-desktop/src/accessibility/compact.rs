//! One compact chat visit, not permission to leave a replacement conversation.
use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use matrix_domain::RoomScope;

#[derive(Clone)]
pub struct BackSnapshot {
    pub id: NodeId,
    pub scope: RoomScope,
    pub focused: bool,
    pub bounds: Option<accesskit::Rect>,
}

#[derive(Clone)]
pub struct BackIntent {
    pub id: NodeId,
    pub scope: RoomScope,
    pub focus_only: bool,
}

pub struct BackNodes {
    current: Option<BackSnapshot>,
    next: u64,
}
impl Default for BackNodes {
    fn default() -> Self {
        Self {
            current: None,
            next: 1 << 58,
        }
    }
}
impl BackNodes {
    pub fn project(&mut self, scope: Option<RoomScope>) -> Option<BackSnapshot> {
        let Some(scope) = scope else {
            self.current = None;
            return None;
        };
        if !self
            .current
            .as_ref()
            .is_some_and(|view| view.scope == scope)
        {
            assert!(self.next < 1 << 59, "compact Back IDs exhausted");
            self.current = Some(BackSnapshot {
                id: NodeId(self.next),
                scope,
                focused: false,
                bounds: None,
            });
            self.next += 1;
        }
        self.current.clone()
    }
}
pub enum BackKey {
    Focus,
    Activate,
    Composer,
    List,
}

impl BackSnapshot {
    pub fn key(&self, key: &str, shift: bool, composer: bool, list: bool) -> Option<BackKey> {
        if !self.focused {
            return (key == "tab" && ((shift && composer) || (!shift && list)))
                .then_some(BackKey::Focus);
        }
        match (key, shift) {
            ("space" | "enter", false) => Some(BackKey::Activate),
            ("tab", false) => Some(BackKey::Composer),
            ("tab", true) | ("escape", false) => Some(BackKey::List),
            _ => None,
        }
    }

    pub fn allows(&self, intent: &BackIntent) -> bool {
        self.id == intent.id && self.scope == intent.scope
    }
    pub fn node(&self) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label("Indietro alle conversazioni");
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        if let Some(bounds) = self.bounds {
            node.set_bounds(bounds);
        }
        node
    }
    pub fn action(&self, request: &ActionRequest) -> Option<BackIntent> {
        (request.target_tree == TreeId::ROOT
            && request.target_node == self.id
            && request.data.is_none()
            && matches!(request.action, Action::Focus | Action::Click))
        .then(|| BackIntent {
            id: self.id,
            scope: self.scope.clone(),
            focus_only: request.action == Action::Focus,
        })
    }
}
