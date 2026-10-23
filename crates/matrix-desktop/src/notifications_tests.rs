use super::*;
use matrix_domain::{AccountId, RoomId, RoomScope, notifications::NotificationPolicy};
use std::sync::mpsc;

fn alert() -> Notification {
    let scope = RoomScope {
        account_id: AccountId::new(1),
        room_id: RoomId::new("!secret:fixture.invalid"),
    };
    let mut policy = NotificationPolicy::new(scope.account_id);
    policy.set_enabled(true);
    policy.update(&scope, 0);
    policy.update(&scope, 1).unwrap()
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
        "Nuovi messaggi"
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
    policy.update(&scope, 0);
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while !handle.submit({
        policy.remove(&scope);
        policy.update(&scope, 0);
        policy.update(&scope, 1).unwrap()
    }) {
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "1 nuovi messaggi"
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
    let result = deliver_os(alert());
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
    assert_eq!(title, "Matrix Desktop");
    assert_eq!(body, "Nuovi messaggi");
    worker.shutdown().unwrap();
}

#[test]
fn macos_alert_passes_text_only_as_argv_never_inside_the_script() {
    let args = macos_osascript_args(&alert());
    assert_eq!(args[0], "-e");
    let script_end = args.iter().position(|a| a == "--").expect("argv separator");
    let script = args[..script_end].join("\n");
    assert!(script.contains("on run argv"));
    assert!(script.contains("display notification (item 2 of argv) with title (item 1 of argv)"));
    assert!(!script.contains("Matrix Desktop") && !script.contains("Nuovi messaggi"));
    assert_eq!(
        &args[script_end + 1..],
        ["Matrix Desktop", "Nuovi messaggi"]
    );
}
