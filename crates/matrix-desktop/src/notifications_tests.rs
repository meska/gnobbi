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
    let worker = NotificationWorker::spawn_with(move |notification| {
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
    let result = deliver_os(alert(), &Click::default());
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
fn worker_reports_failure_without_remote_error_details() {
    let worker = NotificationWorker::spawn_with(|_| Err(NotificationError::Unavailable)).unwrap();
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
    let worker = NotificationWorker::spawn_with(move |notification| {
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
        let preview = MessagePreview { content: PreviewContent::Text("ciao".into()), sender: None, direct: true };
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
    let worker = NotificationWorker::spawn_clicks(click, move |notification| {
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
