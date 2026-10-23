//! Server pagination is distinct from navigating already loaded semantic rows.
use crate::HistoryStatus;
use accesskit::{Action, Node, NodeId, Role};

#[derive(Clone)]
pub struct ServerPagination {
    pub tag: matrix_engine::RequestTag,
    pub status: HistoryStatus,
    pub bounds: Option<accesskit::Rect>,
    pub focused: bool,
}

impl ServerPagination {
    pub fn node(&self) -> Option<(NodeId, Node)> {
        // Separate from fixed controls, device handles, message IDs and text runs.
        // Fail closed rather than wrap into another semantic identity range.
        let generation = self.tag.generation();
        if generation >= 1_u64 << 59 {
            return None;
        }
        let id = NodeId((1_u64 << 59) + generation);
        let mut node = Node::new(Role::Button);
        if !self.tag.is_cancelled() {
            node.add_action(Action::Focus);
        }
        node.set_label(match self.status {
            HistoryStatus::Empty => return None,
            HistoryStatus::Loading => "Caricamento storico…",
            HistoryStatus::More => "Carica messaggi precedenti dal server",
            HistoryStatus::Start => "Inizio dello storico raggiunto",
            HistoryStatus::Failed => "Storico non disponibile · riprova",
        });
        if matches!(self.status, HistoryStatus::More | HistoryStatus::Failed)
            && !self.tag.is_cancelled()
        {
            node.add_action(Action::Click);
        } else {
            node.set_disabled();
        }
        if let Some(bounds) = self.bounds {
            node.set_bounds(bounds);
        }
        Some((id, node))
    }
}
