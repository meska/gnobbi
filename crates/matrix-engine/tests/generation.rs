use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::ViewRequests;

fn scope(account: u64, room: &str) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new(room),
    }
}

#[test]
fn explicit_cancellation_is_idempotent_and_visible_to_workers() {
    let mut views = ViewRequests::default();
    let tag = views.start(scope(1, "!a:offline")).unwrap();
    views.cancel();
    views.cancel();
    assert!(tag.is_cancelled());
    assert!(!views.accepts(&tag));
    let fresh = views.start(scope(1, "!a:offline")).unwrap();
    assert!(views.accepts(&fresh));
    assert!(!fresh.is_cancelled());
}

#[test]
fn dropping_view_owner_cancels_work_still_held_by_worker() {
    let tag = {
        let mut views = ViewRequests::new();
        views.start(scope(1, "!a:offline")).unwrap()
    };
    assert!(tag.is_cancelled());
}

#[test]
fn unrelated_view_owner_cannot_reuse_an_equal_numeric_generation() {
    let mut first = ViewRequests::new();
    let old = first.start(scope(1, "!a:offline")).unwrap();
    let mut second = ViewRequests::new();
    let current = second.start(scope(1, "!a:offline")).unwrap();
    assert_eq!(old.generation(), current.generation());
    assert!(!second.accepts(&old));
    assert!(second.accepts(&current));
    drop(first);
    assert!(!second.accepts(&old));
}

#[test]
fn starting_new_view_cancels_old_work_and_rejects_stale_tags() {
    let mut views = ViewRequests::new();
    let old = views.start(scope(1, "!a:offline")).unwrap();
    let worker = old.clone();
    assert_eq!(old.scope(), &scope(1, "!a:offline"));
    assert_eq!(old.generation(), 0);
    assert!(!worker.is_cancelled());
    assert!(views.accepts(&old));
    let current = views.start(scope(2, "!a:offline")).unwrap();
    assert!(worker.is_cancelled());
    assert!(!views.accepts(&old));
    assert!(views.accepts(&current));
    assert_eq!(current.generation(), 1);
    let returned = views.start(scope(1, "!a:offline")).unwrap();
    assert!(!views.accepts(&old));
    assert!(!views.accepts(&current));
    assert!(views.accepts(&returned));
    assert_eq!(returned.generation(), 2);
}
