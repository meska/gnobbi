use matrix_domain::{
    AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId, TransactionId,
};

#[test]
fn server_identity_is_distinct_from_pending_with_same_text() {
    let server = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!one:example.invalid"),
        },
        id: TimelineId::Server(EventId::new("same-text")),
    };
    assert_eq!(server.id, TimelineId::Server(EventId::new("same-text")));
    let pending = ScopedTimelineId {
        scope: server.scope.clone(),
        id: TimelineId::Pending(TransactionId::new("same-text")),
    };
    assert_ne!(server, pending);
    let other_account = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(2),
            room_id: server.scope.room_id.clone(),
        },
        id: server.id.clone(),
    };
    let other_room = ScopedTimelineId {
        scope: RoomScope {
            account_id: server.scope.account_id,
            room_id: RoomId::new("!two:example.invalid"),
        },
        id: server.id.clone(),
    };
    assert_eq!(
        std::collections::HashSet::from([
            server.clone(),
            server,
            pending,
            other_account,
            other_room
        ])
        .len(),
        4
    );
}

#[test]
fn pending_identity_retains_explicit_scope() {
    let pending = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(1),
            room_id: RoomId::new("!one:example.invalid"),
        },
        id: TimelineId::Pending(TransactionId::new("txn-1")),
    };
    assert_eq!(pending.scope.account_id.as_u64(), 1);
    assert_eq!(pending.scope.room_id.as_str(), "!one:example.invalid");
    assert_eq!(pending.id, TimelineId::Pending(TransactionId::new("txn-1")));
    assert_eq!(pending, pending.clone());
    let other_account = ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(2),
            room_id: pending.scope.room_id.clone(),
        },
        id: pending.id.clone(),
    };
    let other_room = ScopedTimelineId {
        scope: RoomScope {
            account_id: pending.scope.account_id,
            room_id: RoomId::new("!two:example.invalid"),
        },
        id: pending.id.clone(),
    };
    assert_eq!(
        std::collections::HashSet::from([pending.clone(), pending, other_account, other_room])
            .len(),
        3
    );
}

#[test]
fn room_scope_distinguishes_accounts_and_rooms() {
    let scope = |account, room| RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    };
    let original = scope(1, "!one:example.invalid");
    assert_eq!(original, original.clone());
    assert_eq!(original, scope(1, "!one:example.invalid"));
    assert_ne!(original, scope(2, "!one:example.invalid"));
    assert_ne!(original, scope(1, "!two:example.invalid"));
    let keys = std::collections::HashSet::from([
        original,
        scope(1, "!one:example.invalid"),
        scope(2, "!one:example.invalid"),
        scope(1, "!two:example.invalid"),
    ]);
    assert_eq!(keys.len(), 3);
}

#[test]
fn transaction_id_preserves_caller_supplied_text() {
    for value in ["local-001", "$looks_like_event", "", "opaque/日本語"] {
        assert_eq!(TransactionId::new(value).as_str(), value);
    }
}

#[test]
fn event_id_preserves_adapter_supplied_text() {
    for value in [
        "$server_event",
        "$legacy:example.invalid",
        "",
        "opaque/日本語",
    ] {
        assert_eq!(EventId::new(value).as_str(), value);
    }
}

#[test]
fn room_id_preserves_adapter_supplied_text() {
    for value in ["!room:example.invalid", "!opaque", "", "opaque/日本語"] {
        assert_eq!(RoomId::new(value).as_str(), value);
    }
}
