#![cfg(feature = "linux-a11y")]
use accesskit::{Action, ActionRequest, TreeId};
use matrix_desktop::accessibility::{operation_panel::{PanelIntent, PanelSnapshot}, theme_tree};
use matrix_desktop::LiveOperations;
use matrix_domain::{TransactionId, AccountId, EventId, Poll, RoomId, RoomScope, ScopedTimelineId, TimelineId};
use matrix_engine::operations::{MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult};

fn poll(can_end: bool) -> Poll {
    Poll { question: "Q".into(), answers: vec![], max_selections: 1, multiple: false, disclosed: true, ended: !can_end, voters: 0, can_end }
}
fn inspected() -> (LiveOperations, ScopedTimelineId) {
    let id = ScopedTimelineId {
        scope: RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:local") },
        id: TimelineId::Server(EventId::new("$poll")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else { panic!("inspect") };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails { body: "q".into(), permissions: MessagePermissions { redact: true, ..Default::default() } }),
    });
    (operations, id)
}

#[test]
fn entry_only_for_own_open_poll() {
    let (operations, _) = inspected();
    assert!(operations.can_end_poll(Some(&poll(true))));
    assert!(!operations.can_end_poll(Some(&poll(false)))); // other's or already closed
    assert!(!operations.can_end_poll(None)); // not a poll
}

#[test]
fn confirm_sends_end_poll_on_target_and_cancel_sends_nothing() {
    let (mut operations, id) = inspected();
    let mut panel = PanelSnapshot::project(&operations, false, None).unwrap();
    panel.end_available = true;
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let node = tree.nodes.iter().find(|(n, _)| *n == panel.end_id()).expect("end node");
    assert_eq!(node.1.label(), Some("Chiudi sondaggio"));
    let click = |target_node| ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None };
    let begin = panel.action(&click(panel.end_id())).unwrap();
    assert!(matches!(begin.intent, PanelIntent::BeginEnd) && panel.allows(&begin));

    // Confirmation: the confirm button sits in the redact slot, labels name the poll.
    let mut confirming = panel.clone();
    confirming.confirming = true;
    confirming.ending = true;
    confirming.redact_available = true;
    assert!(!confirming.allows(&begin));
    let mut tree = theme_tree(false);
    confirming.append_to(&mut tree);
    let label = |tree: &accesskit::TreeUpdate, n| tree.nodes.iter().find(|(i, _)| *i == n).and_then(|(_, node)| node.label().map(str::to_owned));
    assert_eq!(label(&tree, confirming.close).as_deref(), Some("Annulla chiusura sondaggio"));
    assert_eq!(label(&tree, confirming.confirm_redact_id()).as_deref(), Some("Conferma chiusura sondaggio"));
    let cancel = confirming.action(&click(confirming.close)).unwrap();
    assert!(matches!(cancel.intent, PanelIntent::CancelRedact));
    assert!(operations.take_command().is_none()); // Annulla: nothing queued
    let confirm = confirming.action(&click(confirming.confirm_redact_id())).unwrap();
    assert!(matches!(confirm.intent, PanelIntent::ConfirmRedact) && confirming.allows(&confirm));

    assert!(operations.submit(Operation::EndPoll, TransactionId::new("tx")));
    let Some(OperationCommand::Apply(request)) = operations.take_command() else { panic!("apply") };
    assert!(matches!(request.operation, Operation::EndPoll));
    assert_eq!((request.scope, TimelineId::Server(request.target)), (id.scope, id.id));
}

#[test]
fn tab_reaches_end_after_delete_and_back() {
    let (operations, _) = inspected();
    let mut panel = PanelSnapshot::project(&operations, false, None).unwrap();
    panel.end_available = true;
    panel.redact_available = true;
    panel.redact_focused = true;
    assert!(matches!(panel.key("tab", false, false).unwrap().intent, PanelIntent::FocusEnd));
    panel.redact_focused = false;
    panel.end_focused = true;
    assert!(matches!(panel.key("tab", true, false).unwrap().intent, PanelIntent::FocusRedact));
    assert!(matches!(panel.key("enter", false, false).unwrap().intent, PanelIntent::BeginEnd));
    assert!(matches!(panel.key("tab", false, false).unwrap().intent, PanelIntent::Composer));
}
