#![cfg(feature = "linux-a11y")]
use accesskit::{Action, ActionRequest, Role, TreeId};
use matrix_desktop::{
    LiveOperations,
    accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    },
};
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};

fn selection(event: &str) -> ScopedTimelineId {
    ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!media:local"),
        },
        id: TimelineId::Server(EventId::new(event)),
    }
}

#[test]
fn png_preview_is_reachable_from_the_message_opener_by_tab() {
    let mut operations = LiveOperations::default();
    operations.select(selection("$png"));
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(matches!(
        panel.key("tab", false, true).unwrap().intent,
        PanelIntent::FocusPreview
    ));
}

#[test]
fn png_preview_keyboard_activation_uses_the_focused_control() {
    let mut operations = LiveOperations::default();
    operations.select(selection("$png"));
    let mut panel = PanelSnapshot::project(&operations, false, None).unwrap();
    // The focus request must be routed to a control that has keyboard activation.
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.preview_id(),
        data: None,
    };
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusPreview
    ));
    panel.preview_focused = true;
    panel.preview_bounds = Some(accesskit::Rect::new(10., 20., 150., 48.));
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.preview_id());
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(id, _)| *id == panel.preview_id())
            .unwrap()
            .1
            .bounds(),
        panel.preview_bounds
    );
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::Preview
        ));
    }
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Message
    ));
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::Close
    ));
    panel.preview_focused = false;
    // Until focus is mirrored back, no activation may be inferred.
    assert!(panel.key("space", false, false).is_none());
    panel.focused = true;
    // Reverse traversal from Close must reach Preview, not skip the new action.
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::FocusPreview
    ));
}

#[test]
fn png_preview_exports_a_scoped_action_and_retires_the_previous_opening() {
    let mut operations = LiveOperations::default();
    operations.select(selection("$png"));
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let (id, node) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Anteprima PNG"))
        .expect("existing PNG preview must be an assistive action");
    assert_eq!(node.role(), Role::Button);
    assert!(node.supports_action(Action::Click));
    assert!(node.supports_action(Action::Focus));
    assert!(node.description().unwrap().contains("4 MiB"));
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: *id,
        data: None,
    };
    let action = panel.action(&request).expect("preview action is routed");
    assert!(panel.allows(&action));
    let mut confirming = panel.clone();
    confirming.confirming = true;
    assert!(confirming.action(&request).is_none());
    assert!(!confirming.allows(&action));
    let mut pending = selection("$png");
    pending.id = TimelineId::Pending(matrix_domain::TransactionId::new("pending"));
    operations.clear_selection();
    assert!(!operations.select(pending));
    assert!(PanelSnapshot::project(&operations, false, None).is_none());
    // Reopening even the exact same target must retire the previous handles.
    operations.select(selection("$png"));
    let reopened = PanelSnapshot::project(&operations, false, None).unwrap();
    assert_ne!(reopened.preview_id(), panel.preview_id());
    assert!(reopened.action(&request).is_none());
    operations.select(selection("$other"));
    let replacement = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(replacement.action(&request).is_none());
    assert!(!replacement.allows(&action));
    operations.clear_selection();
    assert!(PanelSnapshot::project(&operations, false, None).is_none());
    assert!(matches!(action.intent, PanelIntent::Preview));
}
