#![cfg(feature = "sdk")]

use matrix_engine::{
    RoomListEvent, RoomSyncStatus, TryReceiveError, auth::AuthError, room_sync::RoomListWorker,
};
use std::time::{Duration, Instant};

#[test]
fn failed_restore_reports_fixed_status_then_closes_the_route() {
    let (worker, mut route) = RoomListWorker::spawn(|| async { Err(AuthError::Storage) }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut statuses = Vec::new();
    loop {
        match route.try_recv() {
            Ok(RoomListEvent::Status(status)) => statuses.push(status),
            Ok(RoomListEvent::Diff(_) | RoomListEvent::TimelineActivity(_) | RoomListEvent::Order(_)) => {
                panic!("failed restore must not invent room activity")
            }
            Err(TryReceiveError::Closed) => break,
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
        assert!(Instant::now() < deadline, "failed restore route must close");
    }
    assert!(
        statuses
            == [
                RoomSyncStatus::Connecting,
                RoomSyncStatus::AuthenticationFailed
            ]
    );
    worker.shutdown().unwrap();
}

#[test]
fn shutdown_during_restore_does_not_claim_to_flush_queued_sends() {
    use matrix_domain::{AccountId, RoomId, RoomScope, TransactionId};
    use matrix_engine::{room_sync::ShutdownError, sending::SendRequest};
    let (mut worker, _events) = RoomListWorker::spawn(std::future::pending).unwrap();
    let (mut sends, _results) = worker.take_send_route().unwrap();
    assert!(
        sends
            .try_send(SendRequest {
                scope: RoomScope {
                    account_id: AccountId::new(22),
                    room_id: RoomId::new("!fixture:invalid")
                },
                transaction_id: TransactionId::new("p022-before-restore"),
                body: "Unconfirmed".into()
            })
            .is_ok()
    );
    assert_eq!(worker.shutdown(), Err(ShutdownError::WriteUnconfirmed));
}

#[test]
fn shutdown_cancels_a_restore_future_that_never_finishes() {
    let (worker, mut route) = RoomListWorker::spawn(std::future::pending).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if matches!(
            route.try_recv(),
            Ok(RoomListEvent::Status(RoomSyncStatus::Connecting))
        ) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "worker must start without awaiting restore"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    worker.shutdown().unwrap();
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "join must cancel pending restore"
    );
    assert!(matches!(route.try_recv(), Err(TryReceiveError::Closed)));
}
