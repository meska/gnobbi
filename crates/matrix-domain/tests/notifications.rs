use matrix_domain::{AccountId, RoomId, RoomScope, notifications::NotificationPolicy};

fn scope(account: u64) -> RoomScope {
    RoomScope {
        account_id: AccountId::new(account),
        room_id: RoomId::new("!private:fixture.invalid"),
    }
}

#[test]
fn notifications_are_off_by_default_and_content_free_when_on() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.enabled());
    assert!(policy.alert(&scope(1)).is_none());
    policy.set_enabled(true);
    let alert = policy.alert(&scope(1)).expect("enabled");
    assert_eq!(alert.title(), "Gnobbi");
    assert_eq!(alert.body(), "Nuovi messaggi");
}

#[test]
fn preview_opt_in_exposes_only_a_fixed_count_never_remote_content() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    assert!(!policy.count_preview());
    policy.set_enabled(true);
    policy.set_count_preview(true);
    assert_eq!(policy.alert(&scope(1)).unwrap().body(), "1 nuovo messaggio");
    policy.set_count_preview(false);
    assert_eq!(policy.alert(&scope(1)).unwrap().body(), "Nuovi messaggi");
}

#[test]
fn foreign_account_cannot_notify() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    assert!(policy.alert(&scope(2)).is_none());
}

#[test]
fn an_alert_names_its_account_and_room_for_the_click() {
    let mut policy = NotificationPolicy::new(AccountId::new(1));
    policy.set_enabled(true);
    let alert = policy.alert(&scope(1)).unwrap();
    assert_eq!(alert.target(), (None, &scope(1)));
    // The window gives the catalog ID: worker IDs repeat between profile folders.
    let alert = alert.from_account(AccountId::new(7), "gnobbi-test-bob".into());
    assert_eq!(alert.title(), "gnobbi-test-bob");
    assert_eq!(alert.target(), (Some(AccountId::new(7)), &scope(1)));
    assert_eq!(alert.body(), "Nuovi messaggi");
}
