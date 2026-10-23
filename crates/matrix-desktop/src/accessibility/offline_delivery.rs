//! Local demo controls are never translated into a Matrix retry.
use crate::{DemoSendState, OfflineDeliveryTarget};
use accesskit::{Action, Node, NodeId, Role};
#[derive(Clone)]
pub struct Snapshot {
    pub target: OfflineDeliveryTarget,
    pub bounds: Option<accesskit::Rect>,
    pub focused: bool,
}
#[derive(Clone)]
pub struct Intent {
    pub target: OfflineDeliveryTarget,
    pub focus_only: bool,
}
impl super::HistorySnapshot {
    pub fn offline_delivery_key(&self, key: &str, shift: bool) -> Option<super::NavigationAction> {
        use super::NavigationAction;
        if !self.focused {
            return None;
        }
        let focused = self
            .messages
            .iter()
            .filter_map(|m| m.offline_delivery.as_ref())
            .find(|s| s.focused);
        let (control, focus_only) = match (key, shift, focused) {
            ("f9", false, _) => (self.messages.first()?.offline_delivery.as_ref()?, true),
            ("space" | "enter", false, Some(s)) => (s, false),
            ("escape", false, Some(s)) | ("tab", true, Some(s)) => {
                return Some(NavigationAction::FocusMessage(s.target.event().clone()));
            }
            ("tab", false, Some(_)) => return Some(NavigationAction::FocusComposer),
            _ => return None,
        };
        Some(NavigationAction::OfflineDelivery(Intent {
            target: control.target.clone(),
            focus_only,
        }))
    }
}

impl Snapshot {
    pub fn id(&self) -> NodeId {
        NodeId(
            (1_u64 << 57)
                .checked_add(self.target.instance())
                .expect("offline nodes exhausted"),
        )
    }
    pub fn node(&self) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(match self.target.state() {
            DemoSendState::Pending => "Simula errore · demo offline",
            DemoSendState::Failed => "Riprova · demo offline",
        });
        node.set_description("Simulazione locale · Nessuna consegna Matrix");
        node.add_action(Action::Focus);
        node.add_action(Action::Click);
        if let Some(bounds) = self.bounds {
            node.set_bounds(bounds);
        }
        node
    }
}
