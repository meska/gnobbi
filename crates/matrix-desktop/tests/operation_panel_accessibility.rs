use matrix_desktop::LiveOperations;

#[cfg(feature = "linux-a11y")]
#[test]
fn redaction_keyboard_focus_defaults_to_cancel_and_uses_observed_geometry() {
    use accesskit::{Action, ActionRequest, Rect, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    let mut operations = LiveOperations::default();
    operations.select(ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    });
    let mut panel = PanelSnapshot::project(&operations, true, None).unwrap();
    panel.redact_available = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusRedact
    ));
    panel.focused = false;
    panel.redact_focused = true;
    panel.redact_bounds = Some(Rect::new(1., 2., 50., 30.));
    let mut request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.redact_id(),
        data: None,
    };
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusRedact
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::BeginRedact
        ));
    }
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.redact_id());
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(id, _)| *id == panel.redact_id())
            .unwrap()
            .1
            .bounds(),
        panel.redact_bounds
    );
    panel.confirming = true;
    request.target_node = panel.confirm_redact_id();
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusConfirmRedact
    ));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::ConfirmRedact
        ));
    }
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::CancelRedact
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.confirm_redact_id());
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(id, _)| *id == panel.confirm_redact_id())
            .unwrap()
            .1
            .bounds(),
        panel.redact_bounds
    );
    panel.redact_focused = false;
    panel.focused = true;
    for key in ["space", "enter", "escape"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::CancelRedact
        ));
    }
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusConfirmRedact
    ));
    assert!(panel.key("enter", true, false).is_none());
    panel.confirming = false;
    panel.focused = false;
    panel.react_available = true;
    panel.react_focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusRedact
    ));
    panel.react_focused = false;
    panel.edit_available = true;
    panel.edit_focused = true;
    panel.react_available = false;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusRedact
    ));
    panel.edit_focused = false;
    panel.edit_available = false;
    panel.reply_available = true;
    panel.reply_focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusRedact
    ));
    operations.clear_selection();
    assert!(panel.key("enter", false, false).is_none());
}

#[cfg(feature = "linux-a11y")]
#[test]
fn redaction_requires_a_distinct_confirmation_and_revalidates_permission_and_opening() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let before = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut click = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: before.redact_id(),
        data: None,
    };
    assert!(before.action(&click).is_none());
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                redact: true,
                ..Default::default()
            },
        }),
    });
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let button = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == panel.redact_id())
        .expect("delete missing");
    assert_eq!(button.1.label(), Some("Elimina il messaggio"));
    let begin = panel.action(&click).unwrap();
    assert!(matches!(begin.intent, PanelIntent::BeginRedact));
    assert!(panel.allows(&begin));
    let mut confirmation = panel.clone();
    confirmation.confirming = true;
    assert!(!confirmation.allows(&begin));
    assert!(confirmation.action(&click).is_none());
    click.target_node = confirmation.confirm_redact_id();
    assert!(panel.action(&click).is_none());
    let confirm = confirmation.action(&click).unwrap();
    assert!(matches!(confirm.intent, PanelIntent::ConfirmRedact));
    assert!(confirmation.allows(&confirm));
    let mut foreign = matrix_desktop::accessibility::operation_panel::PanelAction {
        target: confirm.target.clone(),
        tag: confirm.tag.clone(),
        intent: confirm.intent,
        retry_request: confirm.retry_request.clone(),
    };
    foreign.target.scope.account_id = AccountId::new(2);
    assert!(!confirmation.allows(&foreign));
    foreign.target = confirm.target.clone();
    foreign.target.scope.room_id = RoomId::new("!other:local");
    assert!(!confirmation.allows(&foreign));
    foreign.target = confirm.target.clone();
    foreign.target.id = TimelineId::Server(EventId::new("$other"));
    assert!(!confirmation.allows(&foreign));
    assert!(!panel.allows(&confirm));
    let mut tree = theme_tree(false);
    confirmation.append_to(&mut tree);
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(node, _)| *node == confirmation.confirm_redact_id())
            .unwrap()
            .1
            .label(),
        Some("Conferma eliminazione messaggio")
    );
    assert_eq!(
        tree.nodes
            .iter()
            .find(|(node, _)| *node == confirmation.close)
            .unwrap()
            .1
            .label(),
        Some("Annulla eliminazione messaggio")
    );
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(node, _)| *node == confirmation.redact_id())
    );
    click.target_node = confirmation.close;
    assert!(matches!(
        confirmation.action(&click).unwrap().intent,
        PanelIntent::CancelRedact
    ));
    let mut denied = confirmation.clone();
    denied.redact_available = false;
    assert!(!denied.allows(&confirm));
    click.target_node = confirmation.confirm_redact_id();
    assert!(denied.action(&click).is_none());
    click.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(confirmation.action(&click).is_none());
    click.data = None;
    click.target_tree = TreeId("01010101-0101-0101-0101-010101010101".parse().unwrap());
    assert!(confirmation.action(&click).is_none());
    operations.submit(
        Operation::Redact,
        matrix_domain::TransactionId::new("delete-once"),
    );
    let mut pending = PanelSnapshot::project(&operations, false, None).unwrap();
    pending.confirming = true;
    assert!(!pending.allows(&confirm));
    operations.select(id);
    assert!(
        !PanelSnapshot::project(&operations, false, None)
            .unwrap()
            .allows(&confirm)
    );
    assert!(!confirmation.allows(&confirm));
    assert_ne!(panel.redact_id(), panel.confirm_redact_id());
    assert!(panel.confirm_redact_id().0 < panel.close.0 + 8);
}

#[cfg(feature = "linux-a11y")]
#[test]
fn operation_retry_focus_has_geometry_keyboard_exit_and_request_revalidation() {
    use accesskit::{Action, ActionRequest, Rect, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{Operation, OperationRequest};
    let scope = RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!fixture:local"),
    };
    let mut operations = LiveOperations::default();
    operations.select(ScopedTimelineId {
        scope: scope.clone(),
        id: TimelineId::Server(EventId::new("$fixture")),
    });
    let mut panel = PanelSnapshot::project(&operations, true, None).unwrap();
    panel.retry_request = Some(OperationRequest {
        scope,
        target: EventId::new("$fixture"),
        transaction_id: matrix_domain::TransactionId::new("original-txn"),
        operation: Operation::Reply {
            body: "saved body".into(),
        },
    });
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusRetry
    ));
    panel.focused = false;
    panel.retry_focused = true;
    panel.retry_bounds = Some(Rect::new(1., 2., 60., 30.));
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.retry_id(),
        data: None,
    };
    let action = panel.action(&request).unwrap();
    assert!(matches!(action.intent, PanelIntent::FocusRetry));
    assert!(panel.allows(&action));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::Retry
        ));
    }
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::Close
    ));
    assert!(panel.key("space", true, false).is_none());
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.retry_id());
    let button = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == panel.retry_id())
        .unwrap()
        .1;
    assert!(button.supports_action(Action::Focus));
    assert_eq!(button.bounds(), panel.retry_bounds);
    for change in 0..5 {
        let mut newer = panel.clone();
        let saved = newer.retry_request.as_mut().unwrap();
        match change {
            0 => saved.transaction_id = matrix_domain::TransactionId::new("different-txn"),
            1 => saved.scope.room_id = RoomId::new("!other:local"),
            2 => saved.target = EventId::new("$other"),
            3 => saved.scope.account_id = AccountId::new(2),
            _ => saved.operation = Operation::Redact,
        }
        assert!(!newer.allows(&action));
    }
    panel.retry_request = None;
    assert!(panel.action(&request).is_none());
    assert!(!panel.allows(&action));
    assert!(panel.key("enter", false, false).is_none());
    operations.clear_selection();
    assert!(panel.key("tab", false, true).is_none());
}

#[cfg(feature = "linux-a11y")]
#[test]
fn operation_retry_exports_only_a_transport_failed_exact_request() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationError,
        OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            },
        }),
    });
    assert!(operations.retry_request().is_none());
    assert!(operations.submit(
        Operation::Reply {
            body: "reply Unicode è 🦀".into()
        },
        matrix_domain::TransactionId::new("retry-operation")
    ));
    let Some(OperationCommand::Apply(request)) = operations.take_command() else {
        panic!("write missing")
    };
    assert!(operations.retry_request().is_none());
    operations.apply(OperationResult::Applied {
        request: request.clone(),
        result: Err(OperationError::Transport),
    });
    let saved = operations.retry_request().unwrap();
    assert_eq!(saved.transaction_id, request.transaction_id);
    assert_eq!(saved.scope, request.scope);
    assert_eq!(saved.target, request.target);
    assert!(saved.operation == request.operation);
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let button = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == panel.retry_id())
        .expect("operation retry missing");
    assert_eq!(button.1.label(), Some("Riprova operazione messaggio"));
    assert_eq!(button.1.role(), accesskit::Role::Button);
    let click = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: panel.retry_id(),
        data: None,
    };
    let action = panel.action(&click).unwrap();
    assert!(matches!(action.intent, PanelIntent::Retry));
    assert!(panel.allows(&action));
    let mut malformed = click.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(panel.action(&malformed).is_none());
    malformed = click.clone();
    malformed.target_tree = TreeId("01010101-0101-0101-0101-010101010101".parse().unwrap());
    assert!(panel.action(&malformed).is_none());
    // Switching selection cannot retarget the accepted write. A fresh action
    // is required; its retained request still points at the original room.
    operations.select(ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!other:local"),
        },
        id: TimelineId::Server(EventId::new("$other")),
    });
    let switched = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!switched.allows(&action));
    let mut switched_click = click.clone();
    switched_click.target_node = switched.retry_id();
    let switched_action = switched.action(&switched_click).unwrap();
    assert!(switched.allows(&switched_action));
    assert_eq!(
        switched_action.retry_request.as_ref().unwrap().scope,
        request.scope
    );
    assert_eq!(
        switched_action.retry_request.as_ref().unwrap().target,
        request.target
    );
    assert!(operations.retry());
    assert!(!operations.retry());
    let pending = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(pending.action(&click).is_none());
    assert!(!pending.allows(&action));
    let Some(OperationCommand::Apply(retried)) = operations.take_command() else {
        panic!("retry missing")
    };
    assert_eq!(retried.transaction_id, request.transaction_id);
    assert_eq!(retried.scope, request.scope);
    assert_eq!(retried.target, request.target);
    assert!(retried.operation == request.operation);
    operations.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Forbidden),
    });
    assert!(operations.retry_request().is_none());
    let denied = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(denied.action(&click).is_none());
    operations.select(id);
    assert!(
        !PanelSnapshot::project(&operations, false, None)
            .unwrap()
            .allows(&action)
    );
}

#[cfg(feature = "linux-a11y")]
#[test]
fn reaction_action_is_permission_scoped_and_never_aliases_another_panel() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let before = PanelSnapshot::project(&operations, false, None).unwrap();
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: before.react_id(),
        data: None,
    };
    assert!(before.action(&request).is_none());
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                react: true,
                ..Default::default()
            },
        }),
    });
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let button = tree
        .nodes
        .iter()
        .find(|(node, _)| *node == panel.react_id())
        .expect("reaction missing");
    assert_eq!(button.1.role(), accesskit::Role::Button);
    assert_eq!(button.1.label(), Some("Aggiungi o togli reazione +1"));
    let action = panel.action(&request).unwrap();
    assert!(matches!(action.intent, PanelIntent::React));
    assert!(panel.allows(&action));
    assert_eq!(action.target, id);
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(panel.action(&malformed).is_none());
    malformed = request.clone();
    malformed.target_tree = TreeId("01010101-0101-0101-0101-010101010101".parse().unwrap());
    assert!(panel.action(&malformed).is_none());
    let mut foreign = matrix_desktop::accessibility::operation_panel::PanelAction {
        retry_request: action.retry_request.clone(),
        target: action.target.clone(),
        tag: action.tag.clone(),
        intent: action.intent,
    };
    foreign.target.scope.account_id = AccountId::new(2);
    assert!(!panel.allows(&foreign));
    foreign.target = action.target.clone();
    foreign.target.scope.room_id = RoomId::new("!other:local");
    assert!(!panel.allows(&foreign));
    let mut denied = panel.clone();
    denied.react_available = false;
    assert!(!denied.allows(&action));
    assert!(denied.action(&request).is_none());
    let mut denied_tree = theme_tree(false);
    denied.append_to(&mut denied_tree);
    assert!(
        !denied_tree
            .nodes
            .iter()
            .any(|(id, _)| *id == denied.react_id())
    );
    assert!(operations.toggle_reaction("+1", matrix_domain::TransactionId::new("fixture-react")));
    let pending = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(pending.action(&request).is_none());
    assert!(!pending.allows(&action));
    operations.select(id);
    let reopened = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!reopened.allows(&action));
    assert!(!panel.allows(&action));
    let old_ids = [
        panel.close,
        accesskit::NodeId(panel.close.0 + 1),
        panel.reply_id(),
        panel.edit_id(),
        panel.react_id(),
        panel.retry_id(),
    ];
    let new_ids = [
        reopened.close,
        accesskit::NodeId(reopened.close.0 + 1),
        reopened.reply_id(),
        reopened.edit_id(),
        reopened.react_id(),
        reopened.retry_id(),
    ];
    assert!(old_ids.iter().all(|id| !new_ids.contains(id)));
    assert!(
        old_ids
            .iter()
            .all(|id| (1_u64 << 54..1_u64 << 55).contains(&id.0))
    );
}

#[cfg(feature = "linux-a11y")]
#[test]
fn reaction_focus_links_edit_to_composer_with_observed_bounds() {
    use accesskit::{Action, ActionRequest, Rect, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    let mut operations = LiveOperations::default();
    operations.select(ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    });
    let mut panel = PanelSnapshot::project(&operations, true, None).unwrap();
    panel.react_available = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusReact
    ));
    panel.focused = false;
    panel.react_focused = true;
    panel.react_bounds = Some(Rect::new(1., 2., 30., 28.));
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.react_id(),
        data: None,
    };
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusReact
    ));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::React
        ));
    }
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::Close
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.react_id());
    let button = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == panel.react_id())
        .unwrap()
        .1;
    assert!(button.supports_action(Action::Focus));
    assert_eq!(button.bounds(), panel.react_bounds);
    panel.reply_available = true;
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::FocusReply
    ));
    panel.edit_available = true;
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::FocusEdit
    ));
    panel.react_focused = false;
    panel.edit_focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusReact
    ));
    panel.edit_focused = false;
    panel.edit_available = false;
    panel.reply_focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusReact
    ));
    operations.clear_selection();
    assert!(panel.key("enter", false, false).is_none());
}

#[cfg(feature = "linux-a11y")]
#[test]
fn reply_action_exists_only_after_permission_and_rejects_pending_or_stale_selection() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let before = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    before.append_to(&mut tree);
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Rispondi al messaggio"))
    );
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions::default(),
        }),
    });
    let denied = PanelSnapshot::project(&operations, false, None).unwrap();
    let denied_request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: denied.reply_id(),
        data: None,
    };
    assert!(denied.action(&denied_request).is_none());
    let mut tree = theme_tree(false);
    denied.append_to(&mut tree);
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Rispondi al messaggio"))
    );
    operations.apply(OperationResult::Inspected {
        tag: operations.selection_tag().unwrap().clone(),
        target: EventId::new("$fixture"),
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            },
        }),
    });
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let reply = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Rispondi al messaggio"))
        .expect("permitted reply control missing");
    assert_eq!(reply.1.role(), accesskit::Role::Button);
    assert!(reply.1.supports_action(Action::Click));
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: reply.0,
        data: None,
    };
    let action = panel.action(&request).unwrap();
    assert!(matches!(action.intent, PanelIntent::Reply));
    assert_eq!(action.target, id);
    assert!(panel.allows(&action));
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(panel.action(&malformed).is_none());
    malformed = request.clone();
    malformed.target_tree = TreeId("01010101-0101-0101-0101-010101010101".parse().unwrap());
    assert!(panel.action(&malformed).is_none());
    let mut foreign = matrix_desktop::accessibility::operation_panel::PanelAction {
        retry_request: action.retry_request.clone(),
        target: action.target.clone(),
        tag: action.tag.clone(),
        intent: action.intent,
    };
    foreign.target.scope.account_id = AccountId::new(2);
    assert!(!panel.allows(&foreign));
    foreign.target = action.target.clone();
    foreign.target.scope.room_id = RoomId::new("!other:local");
    assert!(!panel.allows(&foreign));
    operations.submit(
        Operation::Reply {
            body: "reply".into(),
        },
        matrix_domain::TransactionId::new("reply-txn"),
    );
    let pending = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!pending.allows(&action));
    assert!(pending.action(&request).is_none());
    operations.select(id);
    let reopened = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!reopened.allows(&action));
    assert!(!panel.allows(&action));
}

#[cfg(feature = "linux-a11y")]
#[test]
fn edit_action_exists_only_after_permission_and_rejects_pending_or_stale_selection() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let before = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    before.append_to(&mut tree);
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Modifica il messaggio"))
    );
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions::default(),
        }),
    });
    let denied = PanelSnapshot::project(&operations, false, None).unwrap();
    let denied_request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: denied.edit_id(),
        data: None,
    };
    assert!(denied.action(&denied_request).is_none());
    let mut tree = theme_tree(false);
    denied.append_to(&mut tree);
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Modifica il messaggio"))
    );
    operations.apply(OperationResult::Inspected {
        tag: operations.selection_tag().unwrap().clone(),
        target: EventId::new("$fixture"),
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                edit: true,
                ..Default::default()
            },
        }),
    });
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    let edit = tree
        .nodes
        .iter()
        .find(|(_, node)| node.label() == Some("Modifica il messaggio"))
        .expect("permitted edit control missing");
    assert_eq!(edit.1.role(), accesskit::Role::Button);
    assert!(edit.1.supports_action(Action::Click));
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: edit.0,
        data: None,
    };
    let action = panel.action(&request).unwrap();
    assert!(matches!(action.intent, PanelIntent::Edit));
    assert_eq!(action.target, id);
    assert!(panel.allows(&action));
    let mut malformed = request.clone();
    malformed.data = Some(accesskit::ActionData::Value("unexpected".into()));
    assert!(panel.action(&malformed).is_none());
    malformed = request.clone();
    malformed.target_tree = TreeId("01010101-0101-0101-0101-010101010101".parse().unwrap());
    assert!(panel.action(&malformed).is_none());
    let mut foreign = matrix_desktop::accessibility::operation_panel::PanelAction {
        retry_request: action.retry_request.clone(),
        target: action.target.clone(),
        tag: action.tag.clone(),
        intent: action.intent,
    };
    foreign.target.scope.account_id = AccountId::new(2);
    assert!(!panel.allows(&foreign));
    foreign.target = action.target.clone();
    foreign.target.scope.room_id = RoomId::new("!other:local");
    assert!(!panel.allows(&foreign));
    operations.submit(
        Operation::Edit {
            body: "edit".into(),
        },
        matrix_domain::TransactionId::new("edit-txn"),
    );
    let pending = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!pending.allows(&action));
    assert!(pending.action(&request).is_none());
    operations.select(id);
    let reopened = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!reopened.allows(&action));
    assert!(!panel.allows(&action));
}

#[cfg(feature = "linux-a11y")]
#[test]
fn reply_focus_has_observed_geometry_and_keyboard_links_close_to_composer() {
    use accesskit::{Action, ActionRequest, Rect, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id);
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            },
        }),
    });
    let mut panel = PanelSnapshot::project(&operations, true, None).unwrap();
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusReply
    ));
    panel.focused = false;
    panel.reply_focused = true;
    panel.reply_bounds = Some(Rect::new(10., 20., 50., 48.));
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.reply_id(),
        data: None,
    };
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusReply
    ));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::Reply
        ));
    }
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::Close
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.reply_id());
    let reply = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == panel.reply_id())
        .unwrap()
        .1;
    assert!(reply.supports_action(Action::Focus));
    assert_eq!(reply.bounds(), panel.reply_bounds);
    operations.clear_selection();
    assert!(panel.key("enter", false, false).is_none());
}

#[cfg(feature = "linux-a11y")]
#[test]
fn edit_focus_links_close_to_composer_with_observed_geometry() {
    use accesskit::{Action, ActionRequest, Rect, TreeId};
    use matrix_desktop::accessibility::{
        operation_panel::{PanelIntent, PanelSnapshot},
        theme_tree,
    };
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, OperationCommand, OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id);
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                edit: true,
                ..Default::default()
            },
        }),
    });
    let mut panel = PanelSnapshot::project(&operations, true, None).unwrap();
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusEdit
    ));
    panel.focused = false;
    panel.edit_focused = true;
    panel.edit_bounds = Some(Rect::new(10., 20., 50., 48.));
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: panel.edit_id(),
        data: None,
    };
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::FocusEdit
    ));
    for key in ["space", "enter"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::Edit
        ));
    }
    assert!(matches!(
        panel.key("escape", false, false).unwrap().intent,
        PanelIntent::Close
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::Focus
    ));
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    let mut tree = theme_tree(false);
    panel.append_to(&mut tree);
    assert_eq!(tree.focus, panel.edit_id());
    let edit = &tree
        .nodes
        .iter()
        .find(|(id, _)| *id == panel.edit_id())
        .unwrap()
        .1;
    assert!(edit.supports_action(Action::Focus));
    assert_eq!(edit.bounds(), panel.edit_bounds);
    // Both controls available: close -> reply -> edit -> composer, and reverse.
    panel.reply_available = true;
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::FocusReply
    ));
    panel.edit_focused = false;
    panel.reply_focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::FocusEdit
    ));
    assert_ne!(panel.reply_id(), panel.edit_id());
    assert_ne!(panel.close, panel.edit_id());
    assert!(!(1_u64 << 55..1_u64 << 57).contains(&panel.edit_id().0));
    operations.clear_selection();
    assert!(panel.key("enter", false, false).is_none());
}

#[test]
fn panel_notice_tracks_real_inspection_write_failure_and_confirmation_states() {
    use matrix_engine::operations::{
        MessageDetails, MessagePermissions, Operation, OperationCommand, OperationError,
        OperationResult,
    };
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    assert_eq!(operations.notice(false), "Verifica permessi…");
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Err(OperationError::Unavailable),
    });
    assert_eq!(
        operations.notice(false),
        "Azioni non disponibili per questo evento"
    );
    operations.select(id);
    let Some(OperationCommand::Inspect { tag, target }) = operations.take_command() else {
        panic!("inspection missing")
    };
    operations.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "fixture".into(),
            permissions: MessagePermissions {
                reply: true,
                edit: true,
                redact: true,
                react: true,
            },
        }),
    });
    assert_eq!(
        operations.notice(false),
        "Risposta/modifica: scrivi nel draft, poi usa il pulsante. Enter invia normalmente."
    );
    assert_eq!(
        operations.notice(true),
        "Eliminare il messaggio selezionato? Questa azione non si annulla."
    );
    operations.submit(
        Operation::Reply {
            body: "reply fixture".into(),
        },
        matrix_domain::TransactionId::new("fixture-txn"),
    );
    let Some(OperationCommand::Apply(request)) = operations.take_command() else {
        panic!("write missing")
    };
    assert_eq!(
        operations.notice(false),
        "Operazione in attesa · nessuna conferma server"
    );
    operations.apply(OperationResult::Applied {
        request: request.clone(),
        result: Err(OperationError::Transport),
    });
    assert_eq!(
        operations.notice(true),
        "Operazione non confermata · Riprova usa lo stesso ID (anche cambiando stanza)"
    );
    operations.retry();
    assert_eq!(
        operations.notice(false),
        "Operazione in attesa · nessuna conferma server"
    );
    operations.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Forbidden),
    });
    assert_eq!(
        operations.notice(false),
        "Operazione rifiutata o non disponibile"
    );
}

#[cfg(feature = "linux-a11y")]
#[test]
fn panel_handles_never_alias_device_sas_flow_handles() {
    use matrix_desktop::accessibility::{
        devices::{DeviceControl, DeviceNodes, DeviceOverview},
        operation_panel::PanelSnapshot,
    };
    let account = AccountId::new(1);
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: account,
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id);
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let devices = DeviceNodes::default().scope_panel(DeviceOverview::project(
        account,
        &matrix_desktop::DeviceVerification::new(account),
        DeviceControl::Close,
        [None; 6],
    ));
    for control in [
        DeviceControl::Confirm,
        DeviceControl::Mismatch,
        DeviceControl::Cancel,
        DeviceControl::Comparison,
    ] {
        assert_ne!(panel.close, devices.control_id(control));
        assert_ne!(
            accesskit::NodeId(panel.close.0 + 1),
            devices.control_id(control)
        );
    }
    assert!(!(1_u64 << 55..1_u64 << 57).contains(&panel.close.0));
}

#[cfg(feature = "linux-a11y")]
#[test]
fn panel_close_keyboard_links_opener_to_composer_without_a_focus_trap() {
    use matrix_desktop::accessibility::operation_panel::{PanelIntent, PanelSnapshot};
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id);
    let mut panel = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(matches!(
        panel.key("tab", false, true).unwrap().intent,
        PanelIntent::FocusPreview
    ));
    assert!(panel.key("tab", false, false).is_none());
    assert!(panel.key("enter", false, true).is_none());
    panel.focused = true;
    assert!(matches!(
        panel.key("tab", false, false).unwrap().intent,
        PanelIntent::Composer
    ));
    assert!(matches!(
        panel.key("tab", true, false).unwrap().intent,
        PanelIntent::FocusPreview
    ));
    for key in ["space", "enter", "escape"] {
        assert!(matches!(
            panel.key(key, false, false).unwrap().intent,
            PanelIntent::Close
        ));
    }
    assert!(panel.key("space", true, false).is_none());
    assert!(panel.key("x", false, false).is_none());
    operations.clear_selection();
    assert!(panel.key("enter", false, false).is_none());
}

#[cfg(feature = "linux-a11y")]
#[test]
fn panel_close_intent_is_revalidated_against_the_exact_open_instance() {
    use accesskit::{Action, ActionRequest, TreeId};
    use matrix_desktop::accessibility::operation_panel::{PanelIntent, PanelSnapshot};
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    operations.select(id.clone());
    let panel = PanelSnapshot::project(&operations, false, None).unwrap();
    let mut request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: panel.close,
        data: None,
    };
    let close = panel.action(&request).unwrap();
    assert!(matches!(close.intent, PanelIntent::Close));
    assert_eq!(close.target, id);
    assert!(panel.allows(&close));
    request.action = Action::Focus;
    assert!(matches!(
        panel.action(&request).unwrap().intent,
        PanelIntent::Focus
    ));
    request.data = Some(accesskit::ActionData::Value("malformed".into()));
    assert!(panel.action(&request).is_none());
    request.data = None;
    request.target_node = accesskit::NodeId(panel.close.0 + 1);
    assert!(panel.action(&request).is_none());
    operations.select(id);
    let reopened = PanelSnapshot::project(&operations, false, None).unwrap();
    assert!(!panel.allows(&close));
    assert!(!reopened.allows(&close));
    assert_ne!(panel.close, reopened.close);
    operations.clear_selection();
    assert!(!reopened.allows(&close));
}

#[cfg(feature = "linux-a11y")]
#[test]
fn selected_message_panel_exports_readable_status_and_close_without_hiding_chat() {
    use accesskit::{Action, Role};
    use matrix_desktop::accessibility::{ROOT, operation_panel::PanelSnapshot, theme_tree};
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    assert!(PanelSnapshot::project(&operations, false, None).is_none());
    operations.select(id.clone());
    let bounds = accesskit::Rect::new(1., 2., 31., 30.);
    let panel = PanelSnapshot::project(&operations, true, Some(bounds)).unwrap();
    assert_eq!(panel.target, id);
    let mut tree = theme_tree(false);
    let old_children = tree.nodes[0].1.children().to_vec();
    panel.append_to(&mut tree);
    let close = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Chiudi azioni messaggio"))
        .unwrap();
    assert_eq!(close.1.role(), Role::Button);
    assert!(close.1.supports_action(Action::Click));
    assert!(close.1.supports_action(Action::Focus));
    assert_eq!(close.1.bounds(), Some(bounds));
    assert_eq!(close.1.description(), Some("Verifica permessi…"));
    assert_eq!(tree.focus, close.0);
    let root = &tree.nodes.iter().find(|(id, _)| *id == ROOT).unwrap().1;
    assert!(old_children.iter().all(|id| root.children().contains(id)));
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.value() == Some("Verifica permessi…"))
    );
    operations.clear_selection();
    assert!(PanelSnapshot::project(&operations, true, Some(bounds)).is_none());
}
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};

#[test]
fn selection_tag_retires_each_panel_open_even_for_the_same_message() {
    let id = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!fixture:local"),
        },
        id: TimelineId::Server(EventId::new("$fixture")),
    };
    let mut operations = LiveOperations::default();
    assert!(operations.selection_tag().is_none());
    assert!(operations.select(id.clone()));
    let first = operations.selection_tag().unwrap().clone();
    assert_eq!(first.scope(), &id.scope);
    operations.take_command();
    assert_eq!(
        operations.selection_tag().unwrap().generation(),
        first.generation()
    );
    assert!(operations.select(id));
    let second = operations.selection_tag().unwrap().clone();
    assert!(first.is_cancelled());
    assert_ne!(first.generation(), second.generation());
    operations.clear_selection();
    assert!(second.is_cancelled());
    assert!(operations.selection_tag().is_none());
}
