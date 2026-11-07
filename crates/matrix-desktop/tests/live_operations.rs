use matrix_desktop::LiveOperations;
use matrix_domain::TransactionId;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};
use matrix_engine::operations::{
    MessageDetails, MessagePermissions, OperationCommand, OperationResult,
};
use matrix_engine::operations::{Operation, OperationError};

#[test]
fn accepted_write_survives_selection_clear_and_retries_same_transaction() {
    let mut actions = LiveOperations::default();
    let selected = id("!room:test", "$first");
    actions.select(selected.clone());
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                edit: true,
                ..Default::default()
            },
        }),
    });
    assert!(actions.submit(
        Operation::Edit {
            body: "edited".into()
        },
        TransactionId::new("tx")
    ));
    actions.clear_selection();
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("accepted write must not be cancelled");
    };
    assert!(request.scope == selected.scope);
    assert!(actions.apply(OperationResult::Applied {
        request: request.clone(),
        result: Err(OperationError::Transport)
    }));
    assert!(actions.retry());
    let Some(OperationCommand::Apply(retry)) = actions.take_command() else {
        panic!("retry");
    };
    assert!(retry.transaction_id == request.transaction_id && retry.operation == request.operation);
    assert!(actions.apply(OperationResult::Applied {
        request: retry,
        result: Ok(EventId::new("$edited"))
    }));
    assert_eq!(actions.confirmed().len(), 1);
    assert!(!actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$edited"))
    }));
    assert_eq!(actions.confirmed().len(), 1);
}

#[test]
fn forbidden_draft_action_preserves_editor_and_success_consumes_it() {
    let mut actions = LiveOperations::default();
    let mut draft = matrix_desktop::Composer::default();
    draft.replace(None, "edited Unicode è\nlinea");
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            },
        }),
    });
    assert!(!actions.submit_draft(true, TransactionId::new("denied"), &mut draft));
    assert_eq!(draft.text(), "edited Unicode è\nlinea");
    assert!(actions.submit_draft(false, TransactionId::new("reply"), &mut draft));
    assert!(draft.text().is_empty());
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("reply");
    };
    assert!(
        request.operation
            == Operation::Reply {
                body: "edited Unicode è\nlinea".into()
            }
    );
}

#[test]
fn forbidden_result_releases_write_slot_for_another_action() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        reply: true,
        ..Default::default()
    });
    actions.submit(
        Operation::Edit {
            body: "edit".into(),
        },
        TransactionId::new("tx"),
    );
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("write");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Forbidden),
    });
    assert!(!actions.retry());
    assert!(actions.submit(
        Operation::Reply {
            body: "reply".into()
        },
        TransactionId::new("reply")
    ));
}

#[test]
fn failed_write_remains_retryable_after_inspecting_another_room() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        ..Default::default()
    });
    actions.submit(
        Operation::Edit {
            body: "edit".into(),
        },
        TransactionId::new("tx"),
    );
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("write");
    };
    actions.apply(OperationResult::Applied {
        request,
        result: Err(OperationError::Transport),
    });
    actions.select(id("!other:test", "$second"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Err(OperationError::Unavailable),
    });
    assert!(actions.retry());
}

#[test]
fn toggling_own_reaction_redacts_its_server_id() {
    let mut actions = inspected(MessagePermissions {
        react: true,
        ..Default::default()
    });
    assert!(actions.toggle_reaction("+1", TransactionId::new("react")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("react");
    };
    assert!(request.operation == Operation::React { key: "+1".into() });
    actions.apply(OperationResult::Applied {
        request,
        result: Ok(EventId::new("$reaction")),
    });
    assert!(actions.toggle_reaction("+1", TransactionId::new("remove")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else {
        panic!("remove");
    };
    assert!(request.operation == Operation::Redact);
    assert_eq!(request.target, EventId::new("$reaction"));
}

#[test]
fn empty_draft_is_not_consumed_or_queued() {
    let mut actions = inspected(MessagePermissions {
        edit: true,
        ..Default::default()
    });
    let mut draft = matrix_desktop::Composer::default();
    draft.replace(None, "  \n");
    assert!(!actions.submit_draft(true, TransactionId::new("empty"), &mut draft));
    assert_eq!(draft.text(), "  \n");
    assert!(actions.take_command().is_none());
}

fn inspected(permissions: MessagePermissions) -> LiveOperations {
    let mut actions = LiveOperations::default();
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspect");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions,
        }),
    });
    actions
}

fn id(room: &str, event: &str) -> ScopedTimelineId {
    ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(21),
            room_id: RoomId::new(room),
        },
        id: TimelineId::Server(EventId::new(event)),
    }
}
#[test]
fn inspection_rejects_stale_selection_even_with_same_scope() {
    let mut actions = LiveOperations::default();
    assert!(actions.select(id("!room:test", "$first")));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    assert!(actions.select(id("!room:test", "$second")));
    assert!(!actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "stale".into(),
            permissions: MessagePermissions::default()
        })
    }));
    assert!(actions.details().is_none());
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    assert!(actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "current".into(),
            permissions: MessagePermissions {
                reply: true,
                ..Default::default()
            }
        })
    }));
    assert!(actions.details().unwrap().body == "current");
    actions.clear_selection();
    assert!(actions.selection().is_none());
}

#[test]
fn selecting_another_message_preserves_queued_write() {
    let mut actions = LiveOperations::default();
    actions.select(id("!room:test", "$first"));
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else {
        panic!("inspection");
    };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails {
            body: "old".into(),
            permissions: MessagePermissions {
                react: true,
                ..Default::default()
            },
        }),
    });
    assert!(actions.submit(
        Operation::React { key: "+1".into() },
        TransactionId::new("tx")
    ));
    actions.select(id("!other:test", "$second"));
    assert!(matches!(
        actions.take_command(),
        Some(OperationCommand::Apply(_))
    ));
}

fn poll(multiple: bool, ended: bool, own: &[&str]) -> matrix_domain::Poll {
    let a = |id: &str| matrix_domain::PollAnswer { id: id.into(), text: id.into(), votes: 1, own: own.contains(&id) };
    matrix_domain::Poll {
        question: "q".into(),
        answers: vec![a("a"), a("b"), a("c")],
        max_selections: if multiple { 2 } else { 1 },
        multiple,
        disclosed: true,
        ended,
        voters: 1,
        can_end: false,
    }
}

/// Answers of the vote sent by clicking option `ix`, None when nothing is sent.
fn clicked(poll: &matrix_domain::Poll, ix: usize) -> Option<Vec<String>> {
    let mut actions = LiveOperations::default();
    let scope = id("!room:test", "$poll").scope;
    if !actions.vote(scope, EventId::new("$poll"), poll, ix, TransactionId::new("tx")) {
        assert!(actions.take_command().is_none());
        return None;
    }
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("apply") };
    assert_eq!(request.target, EventId::new("$poll"));
    let Operation::Vote { answers } = request.operation else { panic!("vote") };
    Some(answers)
}

#[test]
fn voting_sends_the_answers_the_poll_expects() {
    // single choice replaces; the voted option again sends nothing
    assert_eq!(clicked(&poll(false, false, &["a"]), 1), Some(vec!["b".into()]));
    assert_eq!(clicked(&poll(false, false, &["a"]), 0), None);
    // multiple choice adds, removes, and may end empty
    assert_eq!(clicked(&poll(true, false, &["a"]), 1), Some(vec!["a".into(), "b".into()]));
    assert_eq!(clicked(&poll(true, false, &["a", "b"]), 0), Some(vec!["b".into()]));
    assert_eq!(clicked(&poll(true, false, &["a"]), 0), Some(vec![]));
    // cap reached: adding is ignored; closed: nothing at all; unknown option: nothing
    assert_eq!(clicked(&poll(true, false, &["a", "b"]), 2), None);
    assert_eq!(clicked(&poll(false, true, &["a"]), 1), None);
    assert_eq!(clicked(&poll(false, false, &[]), 9), None);
}

#[test]
fn refused_vote_is_flagged_until_the_next_vote() {
    let mut actions = LiveOperations::default();
    let scope = id("!room:test", "$poll").scope;
    let target = EventId::new("$poll");
    let p = poll(false, false, &[]);
    assert!(actions.vote(scope.clone(), target.clone(), &p, 0, TransactionId::new("t1")));
    assert!(!actions.vote(scope.clone(), target.clone(), &p, 1, TransactionId::new("t2")), "one vote in flight");
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("apply") };
    assert!(actions.apply(OperationResult::Applied { request, result: Err(OperationError::Transport) }));
    assert!(actions.vote_failed(&target));
    assert!(actions.vote(scope, target.clone(), &p, 1, TransactionId::new("t3")), "a transport error does not block the next vote");
    assert!(!actions.vote_failed(&target));
}

/// The operations panel (the message menu) is shown while a message is selected, an operation is pending
/// or an error is held. A poll vote must trigger none of them: the click only votes.
fn panel_shown(actions: &LiveOperations) -> bool {
    actions.selection().is_some() || actions.is_pending() || actions.error().is_some()
}

#[test]
fn left_click_on_a_poll_option_never_shows_the_message_menu() {
    let mut actions = LiveOperations::default();
    let scope = id("!room:test", "$poll").scope;
    let p = poll(false, false, &[]);
    assert!(!panel_shown(&actions));
    assert!(actions.vote(scope, EventId::new("$poll"), &p, 1, TransactionId::new("t1")));
    // right after the event, while the vote is in flight (the frame that used to flash the menu)
    assert!(actions.take_command().is_some(), "the vote goes out");
    assert!(!panel_shown(&actions), "menu visible while the vote is in flight");
}

#[test]
fn accepted_and_refused_votes_leave_no_menu_behind() {
    for result in [Ok(EventId::new("$v")), Err(OperationError::Transport)] {
        let mut actions = LiveOperations::default();
        let scope = id("!room:test", "$poll").scope;
        assert!(actions.vote(scope, EventId::new("$poll"), &poll(false, false, &[]), 0, TransactionId::new("t1")));
        let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("apply") };
        actions.apply(OperationResult::Applied { request, result: result.clone() });
        assert!(!panel_shown(&actions), "menu left after a vote that ended with {result:?}");
    }
}

#[test]
fn clicks_that_send_no_vote_show_no_menu() {
    let scope = id("!room:test", "$poll").scope;
    // own vote clicked again (single choice), cap reached, closed poll
    for (p, ix) in [(poll(false, false, &["a"]), 0), (poll(true, false, &["a", "b"]), 2), (poll(false, true, &["a"]), 1)] {
        let mut actions = LiveOperations::default();
        assert!(!actions.vote(scope.clone(), EventId::new("$poll"), &p, ix, TransactionId::new("t")));
        assert!(actions.take_command().is_none());
        assert!(!panel_shown(&actions));
    }
}

#[test]
fn right_click_on_a_poll_bubble_opens_the_menu_like_any_message() {
    // the bubble's right-button handler is `select`, whatever the poll state
    let mut actions = LiveOperations::default();
    assert!(actions.select(id("!room:test", "$poll")));
    assert!(panel_shown(&actions));
    assert_eq!(actions.selection(), Some(&id("!room:test", "$poll")));
    assert!(matches!(actions.take_command(), Some(OperationCommand::Inspect { .. })));
}
