//! Scrollbar values use a normalized loaded extent; no network pagination intent.
use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, Orientation, Rect, Role, TreeId};
use matrix_domain::RoomScope;

#[derive(Clone, Debug, PartialEq)]
pub enum Pane {
    Rooms(Vec<RoomScope>),
    History(RoomScope),
    /// Room list / chat divider; `fraction` of its snapshot is the width in px.
    Divider { min: f64, max: f64 },
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
        let (id, label, role, min, max) = match self.pane {
            Pane::Rooms(_) => (NodeId(80), "Scorrimento conversazioni", Role::ScrollBar, 0., 1.),
            Pane::History(_) => (NodeId(81), "Scorrimento messaggi", Role::ScrollBar, 0., 1.),
            Pane::Divider { min, max } => (NodeId(82), "Divisore", Role::Splitter, min, max),
        };
        let mut node = Node::new(role);
        node.set_label(label);
        node.set_orientation(Orientation::Vertical);
        node.set_bounds(self.bounds);
        node.set_min_numeric_value(min);
        node.set_max_numeric_value(max);
        node.set_numeric_value(self.fraction.clamp(min, max));
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
            fraction: match self.pane {
                Pane::Divider { min, max } => fraction.clamp(min, max),
                _ => fraction.clamp(0., 1.),
            },
        })
    }
}

#[cfg(test)]
mod divider_tests {
    use super::*;

    #[test]
    fn divider_is_a_labelled_splitter_and_set_value_stays_in_limits() {
        let bar = ScrollbarSnapshot { pane: Pane::Divider { min: 304., max: 480. }, fraction: 320., step: 16., bounds: Rect::new(0., 0., 8., 100.) };
        let (_, node) = bar.node();
        assert_eq!((node.role(), node.label()), (Role::Splitter, Some("Divisore")));
        assert_eq!((node.min_numeric_value(), node.max_numeric_value(), node.numeric_value()), (Some(304.), Some(480.), Some(320.)));
        let set = |v| ActionRequest { action: Action::SetValue, target_tree: TreeId::ROOT, target_node: NodeId(82), data: Some(ActionData::NumericValue(v)) };
        assert_eq!(bar.action(&set(900.)).map(|a| a.fraction), Some(480.));
        assert_eq!(bar.action(&set(10.)).map(|a| a.fraction), Some(304.));
    }
}
