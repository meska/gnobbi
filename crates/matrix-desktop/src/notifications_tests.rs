use super::*;
use matrix_domain::{
    AccountId, RoomId, RoomScope,
    notifications::{MessagePreview, NotificationPolicy, PreviewContent},
};
use std::sync::mpsc;

fn alert() -> Notification {
    let scope = RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!secret:fixture.invalid"),
    };
    let mut policy = NotificationPolicy::new(scope.account_id);
    policy.set_enabled(true);
    policy.alert(&scope, None).unwrap()
}

#[test]
fn cancelling_invalidates_the_bounded_pending_alert_before_delivery() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut first = true;
    let worker = NotificationWorker::spawn_with(move |notification, _| {
        started_tx.send(notification.body()).unwrap();
        if first {
            first = false;
            release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        Ok(())
    })
    .unwrap();
    let handle = worker.handle();
    assert!(handle.submit(alert()));
    assert_eq!(
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "New messages"
    );
    assert!(handle.submit(alert()));
    assert!(!handle.submit(alert()), "queue must be bounded");
    handle.cancel_pending();
    release_tx.send(()).unwrap();
    let scope = RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!private:fixture.invalid"),
    };
    let mut policy = NotificationPolicy::new(scope.account_id);
    policy.set_enabled(true);
    policy.set_count_preview(true);
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while !handle.submit(policy.alert(&scope, None).unwrap()) {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "1 new message"
    );
    worker.shutdown().unwrap();
    assert!(started_rx.try_recv().is_err());
}

#[test]
#[ignore = "private D-Bus only; needs the external test harness (not in this repo)"]
fn isolated_os_notification_roundtrip() {
    let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap();
    assert_eq!(std::env::var("MATRIX_NOTIFICATION_TEST_BUS").unwrap(), bus);
    let expected = std::env::var("MATRIX_NOTIFICATION_EXPECT").unwrap();
    let result = deliver_os(alert(), None, &Click::default());
    if expected == "timeout" {
        assert_eq!(result, Err(NotificationError::Timeout));
    } else if expected == "unavailable" {
        assert_eq!(result, Err(NotificationError::Unavailable));
    } else {
        assert_eq!(expected, "ok");
        assert_eq!(result, Ok(()));
    }
}

#[test]
#[ignore = "private D-Bus only; run with the recorder of the notification_bus script"]
fn isolated_os_notification_carries_the_avatar_as_image_data() {
    let picture = crate::notification_avatar::shrink((8, 8, vec![255; 8 * 8 * 4]));
    assert_eq!(deliver_os(alert(), picture, &Click::default()), Ok(()));
}

#[test]
fn worker_reports_failure_without_remote_error_details() {
    let worker = NotificationWorker::spawn_with(|_, _| Err(NotificationError::Unavailable)).unwrap();
    let handle = worker.handle();
    assert!(!handle.failed());
    assert!(handle.submit(alert()));
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while !handle.failed() {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    worker.shutdown().unwrap();
    assert!(!handle.submit(alert()));
}

#[test]
fn worker_delivers_content_free_payload_off_the_calling_thread() {
    let (sender, receiver) = mpsc::channel();
    let caller = std::thread::current().id();
    let worker = NotificationWorker::spawn_with(move |notification, _| {
        sender
            .send((
                std::thread::current().id(),
                notification.title().to_owned(),
                notification.body(),
            ))
            .unwrap();
        Ok(())
    })
    .unwrap();
    let handle = worker.handle();
    assert!(handle.submit(alert()));
    let (thread, title, body) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(thread, caller);
    assert_eq!(title, "Gnobbi");
    assert_eq!(body, "New messages");
    worker.shutdown().unwrap();
}

#[test]
fn saved_message_preview_switch_reaches_the_alert_body_and_defaults_to_on() {
    let scope = RoomScope { account_id: AccountId::new(1), room_id: RoomId::new("!r:x") };
    let body = |on: bool| {
        let mut policy = NotificationPolicy::new(scope.account_id);
        policy.set_enabled(true);
        policy.set_message_preview(crate::Preferences { message_preview: on, ..Default::default() }.message_preview);
        let preview = MessagePreview { content: PreviewContent::Text("ciao".into()), sender: None, direct: true, sender_avatar: None };
        policy.alert(&scope, Some(preview)).unwrap().body()
    };
    assert_eq!(body(true), "ciao");
    assert_eq!(body(false), "New messages");
    assert!(crate::Preferences::default().message_preview);
}

#[test]
fn delivered_alert_names_its_account_and_a_click_is_handed_back_once() {
    let click = Click::default();
    let slot = click.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = NotificationWorker::spawn_clicks(click, move |notification, _| {
        let (account, scope) = notification.target();
        sender
            .send((notification.title().to_owned(), account))
            .unwrap();
        // What the OS adapter does when the user clicks the alert.
        *slot.lock().unwrap() = Some((account, scope.clone()));
        Ok(())
    })
    .unwrap();
    let handle = worker.handle();
    assert!(handle.take_click().is_none());
    let alert = alert().from_account(AccountId::new(7), "gnobbi-test-bob".into());
    assert!(handle.submit(alert));
    let (title, account) = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        (title.as_str(), account),
        ("gnobbi-test-bob", Some(AccountId::new(7)))
    );
    let end = std::time::Instant::now() + Duration::from_secs(2);
    let clicked = loop {
        if let Some(c) = handle.take_click() {
            break c;
        }
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(clicked.0, Some(AccountId::new(7)));
    assert_eq!(clicked.1.room_id, RoomId::new("!secret:fixture.invalid"));
    assert!(handle.take_click().is_none(), "a click is taken once");
    worker.shutdown().unwrap();
}

#[test]
fn open_room_alert_is_suppressed_only_with_an_active_window() {
    let scope = |a: u64, r: &str| RoomScope { account_id: AccountId::new(a), room_id: RoomId::new(r) };
    let open = scope(1, "!open:x");
    assert!(reading(true, Some(&open), &open));
    assert!(!reading(false, Some(&open), &open), "inactive window: alert");
    assert!(!reading(true, Some(&open), &scope(1, "!other:x")), "other room: alert");
    assert!(!reading(true, Some(&open), &scope(2, "!open:x")), "background account: alert");
    assert!(!reading(true, None, &open));
}

#[test]
fn a_click_key_names_account_and_room_and_survives_a_restart() {
    let scope = RoomScope { account_id: AccountId::new(3), room_id: RoomId::new("!r:fixture.invalid") };
    let catalog = Some(AccountId::new(7));
    assert_eq!(target_key(catalog, &scope), "7:3:!r:fixture.invalid");
    assert_eq!(parse_target(&target_key(catalog, &scope)), Some((catalog, scope.clone())));
    assert_eq!(parse_target(&target_key(None, &scope)), Some((None, scope)));
    assert_eq!(parse_target("no-separator"), None);
    assert_eq!(parse_target("x:3:!r:fixture.invalid"), None);
    assert_eq!(parse_target("7:x:!r:fixture.invalid"), None);
}

#[test]
fn tap_on_a_plain_remote_push_opens_that_account_and_room() {
    // Two accounts: the payload of each pusher names its own catalog id and worker.
    let (catalog, scope) = parse_remote("3:7:accounts/3", "!r:fixture.invalid").unwrap();
    assert_eq!(catalog, Some(AccountId::new(3)));
    assert_eq!((scope.account_id, scope.room_id.as_str()), (AccountId::new(7), "!r:fixture.invalid"));
    assert_eq!(parse_remote("4:7:profile", "!r:fixture.invalid").unwrap().0, Some(AccountId::new(4)));
    // Same key as the extension's rewritten alert: both ways of tapping land on one room.
    assert_eq!(parse_remote("3:7:accounts/3", "!r:s").map(|(c, s)| target_key(c, &s)), Some("3:7:!r:s".into()));
    for bad in [("x:7:d", "!r:s"), ("0:7:d", "!r:s"), ("3:x:d", "!r:s"), ("3", "!r:s"), ("3:7:d", ""), ("3:7:d", "notaroom")] {
        assert_eq!(parse_remote(bad.0, bad.1), None, "{bad:?}");
    }
}

#[test]
fn android_tap_reaches_the_account_that_owns_the_room_and_malformed_extras_are_dropped() {
    assert!(!open_from_push("0:7:profile", "!r:fixture.invalid"));
    assert!(!open_from_push("3:7:accounts/3", "notaroom"));
    assert_eq!(take_early_click(Some(AccountId::new(3))), None);
    assert!(open_from_push("3:7:accounts/3", "!r:fixture.invalid"));
    // Another account's session does not take it; the owner does, once.
    assert_eq!(take_early_click(Some(AccountId::new(4))), None);
    assert_eq!(take_early_click(Some(AccountId::new(3))).map(|s| s.room_id.as_str().to_owned()), Some("!r:fixture.invalid".into()));
    assert_eq!(take_early_click(Some(AccountId::new(3))), None);
    // A tap for an account that is gone is dropped.
    assert!(open_from_push("9:7:accounts/9", "!r:fixture.invalid"));
    assert!(drop_early_click_of_gone_account(&[AccountId::new(3)]));
}

/// `(identifier, thread id, gnobbi key)`: an empty key or thread is an absent one.
fn delivered(items: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    items.iter().map(|(id, thread, key)| (id.to_string(), thread.to_string(), key.to_string())).collect()
}
/// Identifiers removed when the room `room` of worker 7 (catalog account 3) is read.
fn removed<'a>(d: &'a [(String, String, String)], room: &str) -> Vec<&'a str> {
    let scope = RoomScope { account_id: AccountId::new(7), room_id: RoomId::new(room) };
    delivered_to_remove(d, Some(AccountId::new(3)), &scope)
}

#[test]
fn reading_a_room_removes_only_its_delivered_alerts() {
    let d = delivered(&[
        ("apns-1", "!r:s", ""),            // remote push, thread set by the extension only
        ("apns-2", "!other:s", ""),
        ("3:7:!r:s", "!r:s", "3:7:!r:s"),  // our own alert
        ("3:7:!other:s", "!other:s", "3:7:!other:s"),
        ("apns-3", "", ""),                // push the extension did not touch
        ("3:7:!r:s", "", ""),              // our alert from a build without thread ids
    ]);
    assert_eq!(removed(&d, "!r:s"), ["apns-1", "3:7:!r:s", "3:7:!r:s"]);
    assert!(removed(&d, "!none:s").is_empty());
    assert!(removed(&[], "!r:s").is_empty());
}

#[test]
fn removal_ignores_alerts_without_thread_id_and_foreign_identifiers() {
    let d = delivered(&[("apns-1", "", ""), ("garbage:!r:s", "", ""), ("x:y:!r:s", "", ""), ("3:7", "", "")]);
    assert!(removed(&d, "!r:s").is_empty());
}

#[test]
fn removal_no_false_positive_on_similar_or_prefix_room_ids() {
    let d = delivered(&[
        ("a", "!r:s2", ""), ("b", "!r:s:x", ""), ("c", "!rr:s", ""), ("d", "!R:s", ""), ("e", " !r:s", ""), ("f", "!r:s ", ""),
        ("3:7:!r:s2", "", ""), ("3:7:!r:s:x", "", ""), ("3:7:!rr:s", "", ""), ("3:7:!r:s ", "", ""),
    ]);
    assert!(removed(&d, "!r:s").is_empty());
    assert_eq!(removed(&d, "!r:s2"), ["a", "3:7:!r:s2"]);
}

#[test]
fn removal_empty_room_id_matches_nothing_real() {
    // an untouched push has thread id "": an empty room id must not sweep them away
    let d = delivered(&[("apns-1", "", ""), ("3:7:!r:s", "", "")]);
    assert!(removed(&d, "").is_empty(), "empty room id removes untouched pushes");
}

#[test]
fn removal_keeps_the_other_accounts_alerts_in_the_same_room() {
    let d = delivered(&[
        ("a", "!r:s", "3:7:!r:s"),  // this account
        ("b", "!r:s", "4:7:!r:s"),  // same worker id in another profile (catalog 4)
        ("c", "!r:s", "3:8:!r:s"),  // another worker, same room
        ("d", "!r:s", "0:7:!r:s"),  // catalog unknown: may be ours
        ("3:8:!r:s", "", ""),       // old local alert of the other worker
    ]);
    assert_eq!(removed(&d, "!r:s"), ["a", "d"]);
    // the caller does not know its catalog id: the worker still tells accounts apart
    let scope = RoomScope { account_id: AccountId::new(7), room_id: RoomId::new("!r:s") };
    assert_eq!(delivered_to_remove(&d, None, &scope), ["a", "b", "d"]);
}

#[test]
fn removal_never_overrides_an_explicit_thread_of_another_room() {
    // local-looking identifier and key of room A, but the thread says room B
    let d = delivered(&[("3:7:!a:s", "!b:s", "3:7:!a:s"), ("3:7:!a:s", "!b:s", ""), ("apns", "!b:s", "3:7:!a:s")]);
    assert!(removed(&d, "!a:s").is_empty());
    // a key of another room does not match through the thread either
    let d = delivered(&[("x", "!a:s", "3:7:!b:s")]);
    assert!(removed(&d, "!a:s").is_empty());
}

#[test]
fn a_push_delivered_while_the_app_was_closed_goes_once_the_room_is_read() {
    // Rewritten by the extension (key + thread) or not (thread only): both go; nothing else does.
    let d = delivered(&[("p1", "!r:s", "3:7:!r:s"), ("p2", "!r:s", ""), ("p3", "!q:s", "3:7:!q:s")]);
    assert_eq!(removed(&d, "!r:s"), ["p1", "p2"]);
}

fn scope(room: &str) -> RoomScope {
    RoomScope { account_id: AccountId::new(7), room_id: RoomId::new(room) }
}

#[test]
fn a_zero_never_seen_above_zero_clears_nothing() {
    // Cached zeros (also of rooms with an unread push, not yet recounted) are no reason to remove an alert.
    let (a, b) = (scope("!a:s"), scope("!b:s"));
    let mut alerted = Default::default();
    for _ in 0..2 {
        assert!(alerts_to_clear(&[(&a, 0), (&b, 0)], &mut alerted).is_empty());
    }
}

#[test]
fn a_room_that_went_from_unread_to_zero_clears_once() {
    let (a, b) = (scope("!a:s"), scope("!b:s"));
    let mut alerted = Default::default();
    assert!(alerts_to_clear(&[(&a, 3), (&b, 1)], &mut alerted).is_empty());
    assert_eq!(alerts_to_clear(&[(&a, 0), (&b, 2)], &mut alerted), [a.clone()]);
    assert!(alerts_to_clear(&[(&a, 0), (&b, 2)], &mut alerted).is_empty());
}

#[test]
fn push_is_silent_only_for_the_room_on_screen() {
    let scope = RoomScope { account_id: AccountId::new(7), room_id: matrix_domain::RoomId::new("!r:fixture.invalid") };
    let other = RoomScope { room_id: matrix_domain::RoomId::new("!x:fixture.invalid"), ..scope.clone() };
    set_visible(None);
    assert!(!is_visible("3:7:accounts/3", "!r:fixture.invalid"), "nothing on screen");
    set_visible(Some((Some(AccountId::new(3)), scope.clone())));
    assert!(is_visible("3:7:accounts/3", "!r:fixture.invalid"));
    assert!(!is_visible("3:7:accounts/3", "!x:fixture.invalid"), "another room alerts");
    assert!(!is_visible("4:7:accounts/4", "!r:fixture.invalid"), "another account alerts");
    assert!(!is_visible("0:7:accounts/3", "!r:fixture.invalid") && !is_visible("3:7:accounts/3", "r"), "malformed: shown");
    set_visible(Some((Some(AccountId::new(3)), other)));
    assert!(!is_visible("3:7:accounts/3", "!r:fixture.invalid"));
    set_visible(None);
}
