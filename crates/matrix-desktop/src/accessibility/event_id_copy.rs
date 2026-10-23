//! The semantic action shares the painted stamp's captured source.
use crate::EventIdCopyTarget;
use accesskit::{Action, Node, NodeId, Role};

#[derive(Clone)]
pub struct Snapshot {
    pub target: EventIdCopyTarget,
    pub bounds: Option<accesskit::Rect>,
    pub focused: bool,
}
#[derive(Clone)]
pub struct Intent {
    pub target: EventIdCopyTarget,
    pub focus_only: bool,
}
impl Snapshot {
    pub fn id(&self) -> NodeId {
        NodeId(
            (1_u64 << 58)
                .checked_add(self.target.instance())
                .expect("event ID nodes exhausted"),
        )
    }
    pub fn node(&self) -> Node {
        let mut n = Node::new(Role::Button);
        n.set_label("Copia ID evento server");
        if let matrix_domain::TimelineId::Server(id) = &self.target.event().id {
            n.set_description(id.as_str());
        }
        n.add_action(Action::Focus);
        n.add_action(Action::Click);
        if let Some(bounds) = self.bounds {
            n.set_bounds(bounds);
        }
        n
    }
}

impl super::HistorySnapshot {
    pub fn event_id_key(&self, key: &str, shift: bool) -> Option<super::NavigationAction> {
        use super::NavigationAction;
        if !self.focused {
            return None;
        }
        let focused = self
            .messages
            .iter()
            .filter_map(|m| m.event_id_copy.as_ref())
            .find(|c| c.focused);
        let (copy, focus_only) = match (key, shift, focused) {
            ("f10", false, _) => (self.messages.first()?.event_id_copy.as_ref()?, true),
            ("space" | "enter", false, Some(c)) => (c, false),
            ("escape", false, Some(c)) | ("tab", true, Some(c)) => {
                return Some(NavigationAction::FocusMessage(c.target.event().clone()));
            }
            ("tab", false, Some(_)) => return Some(NavigationAction::FocusComposer),
            _ => return None,
        };
        Some(NavigationAction::CopyEventId(Intent {
            target: copy.target.clone(),
            focus_only,
        }))
    }
}
