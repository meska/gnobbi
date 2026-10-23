use matrix_domain::{AccountId, RoomId, RoomScope, notifications::NotificationPolicy};

fn scope(account: u64) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new("!private:fixture.invalid"),
    }
}

#[test]
fn enabling_notifies_only_new_increases_after_the_first_room_baseline() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    assert!(policy.update(&scope(1), 12).is_none());
    let alert = policy.update(&scope(1), 14).expect("new unread");
    assert_eq!(alert.title(), "Matrix Desktop");
    assert_eq!(alert.body(), "Nuovi messaggi");
    assert!(policy.update(&scope(1), 14).is_none());
    assert!(policy.update(&scope(1), 2).is_none());
    assert!(policy.update(&scope(1), 3).is_some());
}

#[test]
fn muted_room_consumes_counts_without_replaying_after_unmute() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    policy.update(&scope(1), 0);
    policy.set_muted(scope(1), true);
    assert!(policy.muted(&scope(1)));
    assert!(policy.update(&scope(1), 1).is_none());
    policy.set_muted(scope(1), false);
    assert!(!policy.muted(&scope(1)));
    assert!(policy.update(&scope(1), 1).is_none());
    assert!(policy.update(&scope(1), 2).is_some());
}

#[test]
fn preview_opt_in_exposes_only_a_bounded_count_never_remote_content() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.count_preview());
    policy.set_enabled(true);
    policy.set_count_preview(true);
    policy.update(&scope(1), 1);
    assert_eq!(
        policy.update(&scope(1), 3).unwrap().body(),
        "2 nuovi messaggi"
    );
    assert_eq!(
        policy.update(&scope(1), u64::MAX).unwrap().body(),
        "99+ nuovi messaggi"
    );
    policy.set_count_preview(false);
    policy.update(&scope(1), 0);
    assert_eq!(
        policy.update(&scope(1), 1).unwrap().body(),
        "Nuovi messaggi"
    );
}

#[test]
fn removal_rebaselines_without_losing_room_mute() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    policy.update(&scope(1), 0);
    policy.set_muted(scope(1), true);
    policy.remove(&scope(1));
    assert!(policy.muted(&scope(1)));
    policy.set_muted(scope(1), false);
    assert!(policy.update(&scope(1), 99).is_none());
    assert!(policy.update(&scope(1), 100).is_some());
}

#[test]
fn foreign_account_cannot_notify_or_change_the_local_room_mute() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    policy.update(&scope(1), 0);
    policy.set_muted(scope(2), true);
    assert!(!policy.muted(&scope(2)));
    assert!(policy.update(&scope(2), 1).is_none());
    assert!(policy.update(&scope(2), 2).is_none());
    assert!(policy.update(&scope(1), 1).is_some());
}

#[test]
fn disabling_consumes_counts_without_replay() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.update(&scope(1), 0);
    policy.update(&scope(1), 5);
    policy.set_enabled(true);
    assert!(policy.update(&scope(1), 5).is_none());
    assert!(policy.update(&scope(1), 6).is_some());
}

#[test]
fn notifications_are_off_by_default_even_when_unread_increases() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.enabled());
    assert!(policy.update(&scope(1), 3).is_none());
    assert!(policy.update(&scope(1), 4).is_none());
}
