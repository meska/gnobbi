use matrix_desktop::time_fmt::clock_time;
use matrix_desktop::DemoHistory;
use matrix_domain::{AccountId, RoomId, RoomScope};
use std::collections::HashSet;

fn scope() -> RoomScope {
    RoomScope {
        account_id: AccountId::new(7),
        room_id: RoomId::new("!history:fixture.invalid"),
    }
}

#[test]
fn fixture_timestamp_is_event_data_not_current_list_position() {
    // Local time of the fixture timestamp: the row keeps its own event time, whatever the zone.
    let at = |h: &DemoHistory, i: usize| clock_time(i as u64 * 60_000) == h.events()[i].time;
    let mut history = DemoHistory::new(scope(), 61);
    assert!(at(&history, 0) && at(&history, 60));
    let (first, hour) = (
        history.events()[0].time.clone(),
        history.events()[60].time.clone(),
    );
    history.prepend_older(25);
    assert_eq!(history.events()[25].time, first);
    assert_eq!(history.events()[85].time, hour);
}

#[test]
fn live_append_keeps_existing_events_after_mixed_updates() {
    let mut history = DemoHistory::new(scope(), 2000);
    let original = DemoHistory::new(scope(), 2000);
    history.append_live(3);
    assert_eq!(&history.events()[..2000], original.events());
    history.prepend_older(25);
    history.append_live(2);
    history.prepend_older(4);
    history.append_live(0);
    assert_eq!(&history.events()[29..2029], original.events());
    assert_eq!(history.events().len(), 2034);
    let ids: HashSet<_> = history.events().iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), 2034);
    assert!(history.events().iter().all(|e| e.id.scope == scope()));
    assert!(
        history.events()[2029..]
            .iter()
            .all(|e| e.text.contains("Ricezione simulata"))
    );
    let mut empty = DemoHistory::new(scope(), 0);
    empty.prepend_older(0);
    empty.append_live(1);
    empty.prepend_older(2);
    assert_eq!(empty.events().len(), 3);
}

#[test]
fn prepend_keeps_existing_scoped_events_unchanged() {
    let mut history = DemoHistory::new(scope(), 2000);
    let original = DemoHistory::new(scope(), 2000);
    history.prepend_older(25);
    assert_eq!(&history.events()[25..], original.events());
    history.prepend_older(7);
    assert_eq!(&history.events()[32..], original.events());
    history.prepend_older(0);
    assert_eq!(history.events().len(), 2032);
    let ids: HashSet<_> = history.events().iter().map(|e| &e.id).collect();
    assert_eq!(ids.len(), 2032);
    assert!(history.events().iter().all(|e| e.id.scope == scope()));
    let repeat = {
        let mut h = DemoHistory::new(scope(), 2000);
        h.prepend_older(25);
        h.prepend_older(7);
        h
    };
    assert_eq!(history.events(), repeat.events());
}
