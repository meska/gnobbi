use matrix_domain::{
    AccountId, RoomId, RoomScope, SendStatus, Timeline, TimelineId, TimelineUpdate, TransactionId,
};

#[test]
fn acknowledgement_promotes_pending_identity_to_sent() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
    assert_eq!(timeline.items()[0].status, SendStatus::Sent);
    assert_eq!(
        timeline.items()[0].identity.id,
        TimelineId::Server(matrix_domain::EventId::new("$e1"))
    );
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("t1"))
    );
}
#[test]
fn echo_before_ack_reconciles_pending_without_duplicate() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Echo {
                transaction_id: Some(TransactionId::new("t1")),
                event_id: matrix_domain::EventId::new("$e1"),
                body: "server body".into(),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
    assert_eq!(timeline.items()[0].status, SendStatus::Sent);
    assert_eq!(timeline.items()[0].body, "server body");
    assert_eq!(
        timeline.items()[0].identity.id,
        TimelineId::Server(matrix_domain::EventId::new("$e1"))
    );
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
}
#[test]
fn acknowledgement_merges_uncorrelated_echo_into_local_position() {
    let mut timeline = pending();
    for (event, body) in [("$other", "other"), ("$e1", "canonical")] {
        timeline
            .apply(
                &scope(),
                TimelineUpdate::Echo {
                    transaction_id: None,
                    event_id: matrix_domain::EventId::new(event),
                    body: body.into(),
                },
            )
            .unwrap();
    }
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 2);
    assert_eq!(timeline.items()[0].body, "canonical");
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("t1"))
    );
    assert_eq!(timeline.items()[1].body, "other");
}
#[test]
fn duplicate_local_send_is_idempotent_but_cannot_replace_body() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::LocalSend {
                transaction_id: TransactionId::new("t1"),
                body: "fixture".into(),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
    let before = timeline.items().to_vec();
    assert_eq!(
        timeline.apply(
            &scope(),
            TimelineUpdate::LocalSend {
                transaction_id: TransactionId::new("t1"),
                body: "different".into()
            }
        ),
        Err(matrix_domain::TimelineError::IdentityConflict)
    );
    assert_eq!(timeline.items(), before);
}
#[test]
fn invalid_retry_does_not_change_pending_or_sent() {
    let mut timeline = pending();
    let before = timeline.items().to_vec();
    assert_eq!(
        timeline.apply(
            &scope(),
            TimelineUpdate::Retry {
                transaction_id: TransactionId::new("t1")
            }
        ),
        Err(matrix_domain::TimelineError::InvalidTransition)
    );
    assert_eq!(timeline.items(), before);
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    let before = timeline.items().to_vec();
    for update in [
        TimelineUpdate::Retry {
            transaction_id: TransactionId::new("t1"),
        },
        TimelineUpdate::Failed {
            transaction_id: TransactionId::new("t1"),
        },
    ] {
        assert_eq!(
            timeline.apply(&scope(), update),
            Err(matrix_domain::TimelineError::InvalidTransition)
        );
        assert_eq!(timeline.items(), before);
    }
}
#[test]
fn unknown_transaction_is_rejected_without_mutation() {
    let mut timeline = pending();
    let before = timeline.items().to_vec();
    for update in [
        TimelineUpdate::Retry {
            transaction_id: TransactionId::new("missing"),
        },
        TimelineUpdate::Failed {
            transaction_id: TransactionId::new("missing"),
        },
        TimelineUpdate::Sent {
            transaction_id: TransactionId::new("missing"),
            event_id: matrix_domain::EventId::new("$e1"),
        },
    ] {
        assert_eq!(
            timeline.apply(&scope(), update),
            Err(matrix_domain::TimelineError::UnknownTransaction)
        );
        assert_eq!(timeline.items(), before);
    }
}
#[test]
fn echo_with_transaction_merges_already_seen_server_row() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Echo {
                transaction_id: None,
                event_id: matrix_domain::EventId::new("$e1"),
                body: "first".into(),
            },
        )
        .unwrap();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Echo {
                transaction_id: Some(TransactionId::new("t1")),
                event_id: matrix_domain::EventId::new("$e1"),
                body: "canonical".into(),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
    assert_eq!(timeline.items()[0].body, "canonical");
    assert_eq!(timeline.items()[0].status, SendStatus::Sent);
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("t1"))
    );
}
#[test]
fn conflicting_event_mapping_is_rejected_atomically() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    let before = timeline.items().to_vec();
    for update in [
        TimelineUpdate::Sent {
            transaction_id: TransactionId::new("t1"),
            event_id: matrix_domain::EventId::new("$different"),
        },
        TimelineUpdate::Echo {
            transaction_id: Some(TransactionId::new("t1")),
            event_id: matrix_domain::EventId::new("$different"),
            body: "bad".into(),
        },
        TimelineUpdate::Echo {
            transaction_id: Some(TransactionId::new("different")),
            event_id: matrix_domain::EventId::new("$e1"),
            body: "bad".into(),
        },
    ] {
        assert_eq!(
            timeline.apply(&scope(), update),
            Err(matrix_domain::TimelineError::IdentityConflict)
        );
        assert_eq!(timeline.items(), before);
    }
}
#[test]
fn late_ack_of_failed_send_is_authoritative() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Failed {
                transaction_id: TransactionId::new("t1"),
            },
        )
        .unwrap();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items()[0].status, SendStatus::Sent);
}

#[test]
fn repeated_echo_and_ack_are_idempotent() {
    let mut timeline = pending();
    for _ in 0..3 {
        timeline
            .apply(
                &scope(),
                TimelineUpdate::Sent {
                    transaction_id: TransactionId::new("t1"),
                    event_id: matrix_domain::EventId::new("$e1"),
                },
            )
            .unwrap();
        timeline
            .apply(
                &scope(),
                TimelineUpdate::Echo {
                    transaction_id: None,
                    event_id: matrix_domain::EventId::new("$e1"),
                    body: "canonical".into(),
                },
            )
            .unwrap();
    }
    assert_eq!(timeline.items().len(), 1);
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("t1"))
    );
    assert_eq!(timeline.items()[0].body, "canonical");
}

#[test]
fn identical_text_without_transaction_never_matches_local_send() {
    let mut timeline = pending();
    for _ in 0..2 {
        timeline
            .apply(
                &scope(),
                TimelineUpdate::Echo {
                    transaction_id: None,
                    event_id: matrix_domain::EventId::new("$e1"),
                    body: "fixture".into(),
                },
            )
            .unwrap();
    }
    assert_eq!(timeline.items().len(), 2);
    assert_eq!(timeline.items()[0].status, SendStatus::Pending);
}

#[test]
fn updates_from_other_account_or_room_are_rejected() {
    let mut timeline = pending();
    let before = timeline.items().to_vec();
    for foreign in [
        RoomScope {
            account_id: AccountId::new(2),
            room_id: scope().room_id,
        },
        RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!other:local"),
        },
    ] {
        for update in [
            TimelineUpdate::LocalSend {
                transaction_id: TransactionId::new("t1"),
                body: "bad".into(),
            },
            TimelineUpdate::Echo {
                transaction_id: Some(TransactionId::new("t1")),
                event_id: matrix_domain::EventId::new("$e1"),
                body: "bad".into(),
            },
            TimelineUpdate::Sent {
                transaction_id: TransactionId::new("t1"),
                event_id: matrix_domain::EventId::new("$e1"),
            },
            TimelineUpdate::Failed {
                transaction_id: TransactionId::new("t1"),
            },
            TimelineUpdate::Retry {
                transaction_id: TransactionId::new("t1"),
            },
        ] {
            assert_eq!(
                timeline.apply(&foreign, update),
                Err(matrix_domain::TimelineError::ScopeMismatch)
            );
            assert_eq!(timeline.items(), before);
        }
    }
}
#[test]
fn delivery_order_permutations_converge_to_one_server_item() {
    // All 6 orders of ACK, uncorrelated echo, and correlated echo.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut timeline = pending();
        for step in order {
            let update = match step {
                0 => TimelineUpdate::Sent {
                    transaction_id: TransactionId::new("t1"),
                    event_id: matrix_domain::EventId::new("$e1"),
                },
                1 => TimelineUpdate::Echo {
                    transaction_id: None,
                    event_id: matrix_domain::EventId::new("$e1"),
                    body: "canonical".into(),
                },
                _ => TimelineUpdate::Echo {
                    transaction_id: Some(TransactionId::new("t1")),
                    event_id: matrix_domain::EventId::new("$e1"),
                    body: "canonical".into(),
                },
            };
            timeline.apply(&scope(), update).unwrap();
        }
        assert_eq!(timeline.items().len(), 1, "order {order:?}");
        assert_eq!(timeline.items()[0].body, "canonical");
        assert_eq!(timeline.items()[0].status, SendStatus::Sent);
        assert_eq!(
            timeline.items()[0].transaction_id,
            Some(TransactionId::new("t1"))
        );
    }
}

#[test]
fn separate_scopes_can_reuse_identifiers_without_reconciliation() {
    let scopes = [
        scope(),
        RoomScope {
            account_id: AccountId::new(2),
            room_id: scope().room_id,
        },
        RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!other:local"),
        },
    ];
    let mut timelines: Vec<_> = scopes.iter().cloned().map(Timeline::new).collect();
    for (i, timeline) in timelines.iter_mut().enumerate() {
        timeline
            .apply(
                &scopes[i],
                TimelineUpdate::LocalSend {
                    transaction_id: TransactionId::new("t1"),
                    body: format!("fixture {i}"),
                },
            )
            .unwrap();
    }
    timelines[0]
        .apply(
            &scopes[0],
            TimelineUpdate::Echo {
                transaction_id: Some(TransactionId::new("t1")),
                event_id: matrix_domain::EventId::new("$e1"),
                body: "canonical".into(),
            },
        )
        .unwrap();
    assert_eq!(timelines[0].items()[0].status, SendStatus::Sent);
    for i in [1, 2] {
        assert_eq!(timelines[i].items()[0].status, SendStatus::Pending);
        assert_eq!(timelines[i].items()[0].body, format!("fixture {i}"));
    }
}
fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!fixture:local"),
    }
}

fn pending() -> Timeline {
    let mut timeline = Timeline::new(scope());
    timeline
        .apply(
            &scope(),
            TimelineUpdate::LocalSend {
                transaction_id: TransactionId::new("t1"),
                body: "fixture".into(),
            },
        )
        .unwrap();
    timeline
}

#[test]
fn failed_send_can_retry_with_same_transaction_and_body() {
    let mut timeline = pending();
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Failed {
                transaction_id: TransactionId::new("t1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items()[0].status, SendStatus::Failed);
    timeline
        .apply(
            &scope(),
            TimelineUpdate::Retry {
                transaction_id: TransactionId::new("t1"),
            },
        )
        .unwrap();
    assert_eq!(timeline.items()[0].status, SendStatus::Pending);
    assert_eq!(
        timeline.items()[0].transaction_id,
        Some(TransactionId::new("t1"))
    );
    assert_eq!(timeline.items()[0].body, "fixture");
    assert_eq!(timeline.items().len(), 1);
}

#[test]
fn local_send_creates_one_pending_item() {
    let mut timeline = Timeline::new(scope());
    timeline
        .apply(
            &scope(),
            TimelineUpdate::LocalSend {
                transaction_id: TransactionId::new("t1"),
                body: "ciao 🌍\nfixture".into(),
            },
        )
        .unwrap();
    assert_eq!(timeline.items().len(), 1);
    let item = &timeline.items()[0];
    assert_eq!(item.identity.scope, scope());
    assert_eq!(
        item.identity.id,
        TimelineId::Pending(TransactionId::new("t1"))
    );
    assert_eq!(item.body, "ciao 🌍\nfixture");
    assert_eq!(item.status, SendStatus::Pending);
    assert_eq!(item.transaction_id, Some(TransactionId::new("t1")));
}
