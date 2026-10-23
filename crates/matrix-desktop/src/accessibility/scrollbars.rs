//! Scrollbar values use a normalized loaded extent; no network pagination intent.
use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Orientation, Rect, Role, TreeId};
use matrix_domain::RoomScope;

#[derive(Clone, Debug, PartialEq)]
pub enum Pane {
    Rooms(Vec<RoomScope>),
    History(RoomScope),
}

#[derive(Clone)]
pub struct ScrollbarSnapshot {
    pub pane: Pane,
    pub fraction: f64,
    pub step: f64,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbarAction {
    pub pane: Pane,
    pub fraction: f64,
}

impl ScrollbarSnapshot {
    pub fn node(&self) -> (NodeId, Node) {
        let (id, label) = match self.pane {
            Pane::Rooms(_) => (NodeId(80), "Scorrimento conversazioni"),
            Pane::History(_) => (NodeId(81), "Scorrimento messaggi"),
        };
        let mut node = Node::new(Role::ScrollBar);
        node.set_label(label);
        node.set_orientation(Orientation::Vertical);
        node.set_bounds(self.bounds);
        node.set_min_numeric_value(0.);
        node.set_max_numeric_value(1.);
        node.set_numeric_value(self.fraction.clamp(0., 1.));
        node.set_numeric_value_step(self.step);
        node.add_action(Action::SetValue);
        (id, node)
    }

    pub fn action(&self, request: &ActionRequest) -> Option<ScrollbarAction> {
        if request.target_tree != TreeId::ROOT || request.target_node != self.node().0 {
            return None;
        }
        let fraction = match (&request.action, &request.data) {
            (Action::SetValue, Some(ActionData::NumericValue(value))) if value.is_finite() => {
                *value
            }
            _ => return None,
        };
        Some(ScrollbarAction {
            pane: self.pane.clone(),
            fraction: fraction.clamp(0., 1.),
        })
    }
}
