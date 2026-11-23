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
fn the_menu_toggle_reacts_or_takes_back_by_what_the_row_shows() {
    let mut actions = inspected(MessagePermissions { react: true, ..Default::default() });
    assert!(actions.toggle_reaction("+1", false, TransactionId::new("react")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("react") };
    assert!(request.operation == Operation::React { key: "+1".into() });
    actions.apply(OperationResult::Applied { request, result: Ok(EventId::new("$reaction")) });
    // The row now says the reaction is ours (even one made in an earlier session): the same emoji takes it back.
    assert!(actions.toggle_reaction("+1", true, TransactionId::new("remove")));
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("remove") };
    assert!(request.operation == Operation::Unreact { key: "+1".into() });
}

fn reaction(key: &str, count: usize, mine: bool) -> matrix_engine::history::Reaction {
    matrix_engine::history::Reaction { key: key.into(), count, senders: Vec::new(), mine, pending: false }
}

#[test]
fn a_pill_tap_shows_at_once_and_rolls_back_when_the_send_fails() {
    use matrix_desktop::overlay_reactions as overlay;
    let mut actions = LiveOperations::default();
    let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") };
    let target = EventId::new("$m");
    let server = vec![reaction("👍", 2, false)];
    assert!(actions.react_direct(scope.clone(), target.clone(), "👍", false, TransactionId::new("t1")));
    assert!(!actions.react_direct(scope.clone(), target.clone(), "👍", false, TransactionId::new("t2")), "one wish per pill");
    let shown = overlay(actions.wishes(), &target, &server);
    assert_eq!((shown[0].count, shown[0].mine, shown[0].pending), (3, true, true));
    // A new emoji appears after the others; another message is untouched.
    assert!(actions.react_direct(scope.clone(), target.clone(), "🎉", false, TransactionId::new("t3")) == false, "the command slot is busy");
    assert_eq!(overlay(actions.wishes(), &EventId::new("$other"), &server), server);
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("apply") };
    assert!(request.operation == Operation::React { key: "👍".into() });
    actions.apply(OperationResult::Applied { request, result: Err(OperationError::Transport) });
    assert_eq!(overlay(actions.wishes(), &target, &server), server, "rolled back");
}

#[test]
fn taking_a_pill_back_drops_it_at_zero_and_a_wish_the_server_met_changes_nothing() {
    use matrix_desktop::overlay_reactions as overlay;
    let mut actions = LiveOperations::default();
    let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") };
    let target = EventId::new("$m");
    assert!(actions.react_direct(scope, target.clone(), "👍", true, TransactionId::new("t1")));
    let alone = vec![reaction("👍", 1, true), reaction("🎉", 1, false)];
    let shown = overlay(actions.wishes(), &target, &alone);
    assert_eq!(shown.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["🎉"]);
    let shared = vec![reaction("👍", 3, true)];
    let shown = overlay(actions.wishes(), &target, &shared);
    assert_eq!((shown[0].count, shown[0].mine, shown[0].pending), (2, false, true));
    // The server already shows it gone: nothing changes.
    let gone = vec![reaction("👍", 2, false)];
    assert_eq!(overlay(actions.wishes(), &target, &gone), gone);
    let Some(OperationCommand::Apply(request)) = actions.take_command() else { panic!("apply") };
    assert!(request.operation == Operation::Unreact { key: "👍".into() });
    actions.apply(OperationResult::Applied { request, result: Ok(EventId::new("$removed")) });
    actions.tick(std::time::Instant::now());
    assert_eq!(actions.wishes().len(), 1, "kept a few seconds for the timeline to catch up");
    actions.tick(std::time::Instant::now() + std::time::Duration::from_secs(9));
    assert!(actions.wishes().is_empty());
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

#[test]
fn a_vote_and_another_operation_in_the_same_frame_never_overwrite_each_other() {
    let mut actions = LiveOperations::default();
    let selected = id("!room:test", "$msg");
    actions.select(selected.clone());
    let Some(OperationCommand::Inspect { tag, target }) = actions.take_command() else { panic!("inspect") };
    actions.apply(OperationResult::Inspected {
        tag,
        target,
        details: Ok(MessageDetails { body: "hi".into(), permissions: MessagePermissions { react: true, ..Default::default() } }),
    });
    let p = poll(false, false, &[]);
    assert!(actions.submit(Operation::React { key: "+1".into() }, TransactionId::new("r")));
    assert!(!actions.vote(selected.scope.clone(), EventId::new("$poll"), &p, 0, TransactionId::new("v")), "the unsent reaction is kept");
    let Some(OperationCommand::Apply(react)) = actions.take_command() else { panic!("react") };
    assert!(matches!(react.operation, Operation::React { .. }));
    actions.apply(OperationResult::Applied { request: react, result: Ok(EventId::new("$r")) });
    assert!(actions.vote(selected.scope, EventId::new("$poll"), &p, 0, TransactionId::new("v")));
    assert!(!actions.submit(Operation::React { key: "+1".into() }, TransactionId::new("r2")), "the unsent vote is kept");
    assert!(matches!(actions.take_command(), Some(OperationCommand::Apply(r)) if matches!(r.operation, Operation::Vote { .. })));
}

#[test]
fn a_new_pill_of_ours_replaces_the_old_one_and_taking_it_back_keeps_the_rest() {
    use matrix_desktop::overlay_reactions as overlay;
    let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:test") };
    let target = EventId::new("$m");
    let key = |shown: &[matrix_engine::history::Reaction]| shown.iter().map(|r| (r.key.clone(), r.count, r.mine)).collect::<Vec<_>>();
    // None of ours before: nothing to replace.
    let mut actions = LiveOperations::default();
    assert!(actions.react_direct(scope.clone(), target.clone(), "😮", false, TransactionId::new("t0")));
    assert_eq!(key(&overlay(actions.wishes(), &target, &[reaction("👍", 1, false)])), [("👍".into(), 1, false), ("😮".into(), 1, true)]);
    // Ours 😂 (alone) and 👍 (shared): 😮 replaces both, the others' count stays.
    let server = vec![reaction("😂", 1, true), reaction("👍", 3, true)];
    let mut actions = LiveOperations::default();
    assert!(actions.react_direct(scope.clone(), target.clone(), "😮", false, TransactionId::new("t1")));
    assert_eq!(key(&overlay(actions.wishes(), &target, &server)), [("👍".into(), 2, false), ("😮".into(), 1, true)]);
    // Same emoji again is the toggle: only that one goes.
    let mut actions = LiveOperations::default();
    assert!(actions.react_direct(scope, target.clone(), "😂", true, TransactionId::new("t2")));
    assert_eq!(key(&overlay(actions.wishes(), &target, &server)), [("👍".into(), 3, true)]);
}
