use matrix_desktop::EventIdCopies;
use matrix_domain::{AccountId, EventId, RoomId, RoomScope, ScopedTimelineId, TimelineId};

fn event() -> ScopedTimelineId {
    ScopedTimelineId {
        scope: RoomScope {
            account_id: AccountId::new(80),
            room_id: RoomId::new("!copy:test"),
        },
        id: TimelineId::Server(EventId::new("$exact_-/ID:test")),
    }
}

#[test]
fn removed_changed_or_away_back_sources_never_resolve_to_replacements() {
    let id = event();
    for change in ["text", "event", "room", "account", "retire"] {
        let mut copies = EventIdCopies::default();
        let old = copies.target(&id, "body").unwrap();
        let mut replacement = id.clone();
        let body = if change == "text" { "changed" } else { "body" };
        match change {
            "event" => replacement.id = TimelineId::Server(EventId::new("$new:test")),
            "room" => replacement.scope.room_id = RoomId::new("!new:test"),
            "account" => replacement.scope.account_id = AccountId::new(81),
            "retire" => copies.clear(),
            _ => (),
        }
        copies.retain_current([(&replacement, body)]);
        let new = copies.target(&replacement, body).unwrap();
        assert!(copies.resolve(&old).is_none(), "{change}");
        assert!(copies.resolve(&new).is_some());
        copies.retain_current([(&id, "body")]);
        let returned = copies.target(&id, "body").unwrap();
        assert!(copies.resolve(&old).is_none(), "return {change}");
        assert!(copies.resolve(&returned).is_some());
    }
}

#[test]
fn captured_stamp_copies_only_its_current_exact_server_id() {
    let mut copies = EventIdCopies::default();
    let id = event();
    let target = copies.target(&id, "caffè 🙂\n").unwrap();
    assert_eq!(copies.resolve(&target), Some("$exact_-/ID:test"));
    assert_eq!(
        copies.target(&id, "caffè 🙂\n").unwrap().instance(),
        target.instance()
    );
    let mut pending = id.clone();
    pending.id = TimelineId::Pending(matrix_domain::TransactionId::new("$exact_-/ID:test"));
    assert!(copies.target(&pending, "caffè 🙂\n").is_none());
    copies.target(&id, "replacement").unwrap();
    assert_eq!(copies.resolve(&target), None);
    copies.target(&id, "caffè 🙂\n").unwrap();
    assert_eq!(
        copies.resolve(&target),
        None,
        "same-body return must not revive a captured callback"
    );
}
