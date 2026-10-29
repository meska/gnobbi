use crate::{RoomListEvent, RoomSyncStatus, room_sync::RoomListWorker, sending::SendRequest};
use matrix_domain::{RoomId, RoomScope, TransactionId};
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_join_preserves_committed_sdk_state() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let phrase = "disposable persistence passphrase";
    let room = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"stored snapshot","visibility":"private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let (session, original_device) = runtime().block_on(async {
        let session = AccountStore::create(root, AccountId::new(22))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                phrase,
                http(&config),
            )
            .await
            .unwrap();
        let device = session.whoami().await.unwrap().device_id().to_owned();
        (session, device)
    });
    let sealed_before = std::fs::read(root.join("account-22/session.sealed")).unwrap();
    let (worker, mut events) = RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if matches!(event, RoomListEvent::Status(RoomSyncStatus::Ready)) {
                    break;
                }
            }
        })
        .await
        .unwrap();
    });
    worker.shutdown().expect("clean backend join");
    let _paused = PausedServer::new(&config);
    // Build/restore without remote whoami strictly inside this test to prove
    // data came from SQLite, not a new /sync or another account's projection.
    runtime().block_on(async {
        let store =
            AccountStore::open(root, AccountId::new(22)).expect("worker released account lock");
        let value = store.unseal(phrase).unwrap();
        let client = matrix_sdk::Client::builder()
            .homeserver_url(config["url"].as_str().unwrap())
            .disable_well_known_lookup(true)
            .respect_login_well_known(false)
            .http_client(http(&config))
            .sqlite_store(&store.path, Some(phrase))
            .build()
            .await
            .unwrap();
        let sdk_session: matrix_sdk::authentication::matrix::MatrixSession =
            serde_json::from_value(value["session"].clone()).unwrap();
        client.restore_session(sdk_session).await.unwrap();
        assert_eq!(client.device_id().unwrap().as_str(), original_device);
        assert!(
            client
                .state_store()
                .get_kv_data(matrix_sdk::store::StateStoreDataKey::SyncToken)
                .await
                .unwrap()
                .is_some(),
            "committed sync cursor survives join"
        );
        assert_eq!(
            client
                .get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap())
                .unwrap()
                .name()
                .as_deref(),
            Some("stored snapshot")
        );
    });
    assert_eq!(
        std::fs::read(root.join("account-22/session.sealed")).unwrap(),
        sealed_before
    );
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_full_result_queue_does_not_deadlock_shutdown() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let (mut sends, _results) = worker.take_send_route().unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            for index in 0..12 {
                assert!(
                    sends
                        .send(SendRequest {
                            scope: RoomScope {
                                account_id: AccountId::new(22),
                                room_id: RoomId::new(&room)
                            },
                            transaction_id: TransactionId::new(format!("p022-drain-{index}")),
                            body: format!("queued {index}"),
 attachment: None, mentions: Vec::new(),
 })
                        .await
                        .is_ok()
                );
            }
        })
        .await
        .unwrap();
    });
    worker
        .shutdown()
        .expect("full results must not prevent draining accepted writes");
    runtime().block_on(async {
        let messages = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"),
            serde_json::json!({}),
        )
        .await;
        let actual: std::collections::BTreeSet<_> = messages["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|event| event["content"]["body"].as_str())
            .collect();
        let expected: std::collections::BTreeSet<_> =
            (0..12).map(|i| format!("queued {i}")).collect();
        assert_eq!(actual, expected.iter().map(String::as_str).collect());
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_revoked_session_stops_reconnect() {
    let config = fixture();
    let session = runtime().block_on(login(&config, "alice", 22));
    let token = session.client.as_ref().unwrap().access_token().unwrap();
    let (worker, mut events) = RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    let outcome = runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !matches!(
                events.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .unwrap();
        let response = http(&config)
            .post(format!(
                "{}_matrix/client/v3/logout",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(token)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = events.recv().await {
                if matches!(
                    event,
                    RoomListEvent::Status(RoomSyncStatus::AuthenticationFailed)
                ) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    });
    worker.shutdown().unwrap();
    assert!(
        outcome,
        "revoked credentials are terminal, not a network reconnect loop"
    );
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_drops_sdk_store_inside_the_runtime() {
    // The SDK client of a stored session keeps a deadpool/sqlite pool whose
    // connections close via spawn_blocking: the worker must drop the last
    // client clone while its runtime is still alive, or cleanup aborts with
    // "there is no reactor running" (deadpool-runtime) on SIGTERM shutdown.
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let session = runtime().block_on(async {
        AccountStore::create(root, AccountId::new(22))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                "shutdown drop order passphrase",
                http(&config),
            )
            .await
            .unwrap()
    });
    let (worker, mut events) = RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = events.recv().await {
                if matches!(event, RoomListEvent::Status(RoomSyncStatus::Ready)) {
                    break;
                }
            }
        })
        .await
        .unwrap();
    });
    worker.shutdown().expect("no destructor panic after the runtime drop");
}

fn ready_worker(
    config: &serde_json::Value,
) -> (RoomListWorker, crate::Receiver<RoomListEvent>, String) {
    let room = runtime().block_on(async {
        room_request(
            config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"lifecycle","visibility":"private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let login_config = config.clone();
    let (worker, mut events) =
        RoomListWorker::spawn(move || async move { Ok(login(&login_config, "alice", 22).await) })
            .unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !matches!(
                events.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .unwrap();
    });
    (worker, events, room)
}

fn operation_request(config: &serde_json::Value, room: &str) -> crate::operations::OperationRequest {
    let target = runtime().block_on(async {
        room_request(config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p022-operation-target"),
            serde_json::json!({"msgtype":"m.text","body":"original"})).await["event_id"]
            .as_str().unwrap().to_owned()
    });
    crate::operations::OperationRequest {
        scope: RoomScope { account_id: AccountId::new(22), room_id: RoomId::new(room) },
        transaction_id: TransactionId::new("p022-lifecycle-edit"),
        target: matrix_domain::EventId::new(target),
        operation: crate::operations::Operation::Edit { body: "edited during drain".into() },
    }
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_drains_accepted_operation() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let request = operation_request(&config, &room);
    let target = request.target.clone();
    let (mut commands, _results) = worker.take_operation_route().unwrap();
    runtime().block_on(async {
        assert!(commands.send(crate::operations::OperationCommand::Apply(request)).await.is_ok());
    });
    worker.shutdown().expect("accepted edit is drained before join");
    runtime().block_on(async {
        let messages = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        let edits: Vec<_> = messages["chunk"].as_array().unwrap().iter()
            .filter(|event| event["content"]["m.relates_to"]["rel_type"] == "m.replace").collect();
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["content"]["m.relates_to"]["event_id"], target.as_str());
        assert_eq!(edits[0]["content"]["m.new_content"]["body"], "edited during drain");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_dropped_operations_cannot_hide_unconfirmed_write() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let request = operation_request(&config, &room);
    let (mut commands, results) = worker.take_operation_route().unwrap();
    drop(results);
    let _paused = PausedServer::new(&config);
    runtime().block_on(async {
        assert!(commands.send(crate::operations::OperationCommand::Apply(request)).await.is_ok());
    });
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(worker.shutdown(), Err(crate::room_sync::ShutdownError::WriteUnconfirmed));
}

struct PausedServer(String);
impl PausedServer {
    fn new(config: &serde_json::Value) -> Self {
        let pid = config["server_pid"].to_string();
        assert!(
            std::process::Command::new("/usr/bin/kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
        Self(pid)
    }
}
impl Drop for PausedServer {
    fn drop(&mut self) {
        let _ = std::process::Command::new("/usr/bin/kill")
            .args(["-CONT", &self.0])
            .status();
    }
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_reports_unconfirmed_writes() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let (mut sends, _results) = worker.take_send_route().unwrap();
    let _paused = PausedServer::new(&config);
    runtime().block_on(async {
        assert!(
            sends
                .send(SendRequest {
                    scope: RoomScope {
                        account_id: AccountId::new(22),
                        room_id: RoomId::new(&room)
                    },
                    transaction_id: TransactionId::new("p022-offline-shutdown"),
                    body: "Unconfirmed shutdown".into(),
 attachment: None, mentions: Vec::new(),
 })
                .await
                .is_ok()
        );
    });
    let start = Instant::now();
    assert!(
        worker.shutdown().is_err(),
        "join must not claim a clean flush when delivery is unconfirmed"
    );
    assert!(
        start.elapsed() < Duration::from_secs(7),
        "shutdown has a bounded drain deadline"
    );
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_dropped_results_cannot_hide_unconfirmed_send() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let (mut sends, results) = worker.take_send_route().unwrap();
    drop(results);
    let _paused = PausedServer::new(&config);
    runtime().block_on(async {
        assert!(sends.send(SendRequest {
            scope: RoomScope { account_id: AccountId::new(22), room_id: RoomId::new(room) },
            transaction_id: TransactionId::new("p022-dropped-results"),
            body: "Unconfirmed after UI drop".into(),
 attachment: None, mentions: Vec::new(),
 }).await.is_ok());
    });
    // Let the failed send hit the closed result route before requesting stop.
    std::thread::sleep(Duration::from_secs(6));
    assert_eq!(worker.shutdown(), Err(crate::room_sync::ShutdownError::WriteUnconfirmed));
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_reconnect_restores_ready_without_duplicate_send() {
    let config = fixture();
    let (mut worker, mut events, room) = ready_worker(&config);
    let (mut sends, mut results) = worker.take_send_route().unwrap();
    let request = SendRequest {
        scope: RoomScope {
            account_id: AccountId::new(22),
            room_id: RoomId::new(&room),
        },
        transaction_id: TransactionId::new("p022-reconnect-identity"),
        body: "survives reconnect".into(),
 attachment: None, mentions: Vec::new(),
 };
    runtime().block_on(async {
        assert!(sends.send(request.clone()).await.is_ok());
        let original = next_result(&mut results, 8).await.result.unwrap();
        let paused = PausedServer::new(&config);
        tokio::time::timeout(Duration::from_secs(20), async {
            while !matches!(
                events.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::SyncFailed))
            ) {}
        })
        .await
        .expect("offline status");
        drop(paused);
        tokio::time::timeout(Duration::from_secs(20), async {
            while !matches!(
                events.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .expect("recovered status");
        // The queue is still usable after the reconnect: a second message goes out once, and the
        // first one is not sent again.
        let second = SendRequest {
            transaction_id: TransactionId::new("p022-after-reconnect"),
            body: "after reconnect".into(),
            ..request.clone()
        };
        assert!(sends.send(second.clone()).await.is_ok());
        let second_id = next_result(&mut results, 8).await.result.unwrap();
        let messages = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"),
            serde_json::json!({}),
        )
        .await;
        let actual: Vec<_> = messages["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "m.room.message")
            .collect();
        assert_eq!(actual.len(), 2);
        // `dir=b` lists the newest first.
        assert_eq!(actual[1]["event_id"].as_str().unwrap(), original.as_str());
        assert_eq!(actual[1]["content"]["body"], request.body);
        assert_eq!(actual[0]["event_id"].as_str().unwrap(), second_id.as_str());
        assert_eq!(actual[0]["content"]["body"], second.body);
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_deadline_is_explicit() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let (mut sends, _results) = worker.take_send_route().unwrap();
    let _paused = PausedServer::new(&config);
    runtime().block_on(async {
        for index in 0..3 {
            assert!(
                sends
                    .send(SendRequest {
                        scope: RoomScope {
                            account_id: AccountId::new(22),
                            room_id: RoomId::new(&room)
                        },
                        transaction_id: TransactionId::new(format!("p022-deadline-{index}")),
                        body: "Unconfirmed".into(),
 attachment: None, mentions: Vec::new(),
 })
                    .await
                    .is_ok()
            );
        }
    });
    let start = Instant::now();
    assert_eq!(
        worker.shutdown(),
        Err(crate::room_sync::ShutdownError::DrainDeadline)
    );
    assert!(start.elapsed() < Duration::from_secs(7));
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_cancels_inflight_history_and_inspection() {
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let target = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"read target"}),
        )
        .await["event_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let (mut history, _pages) = worker.take_history_route().unwrap();
    let (mut operations, _results) = worker.take_operation_route().unwrap();
    let _paused = PausedServer::new(&config);
    let mut views = crate::ViewRequests::new();
    let tag = views
        .start(RoomScope {
            account_id: AccountId::new(22),
            room_id: RoomId::new(room),
        })
        .unwrap();
    runtime().block_on(async {
        assert!(
            history
                .send(crate::history::HistoryRequest {
                    tag: tag.clone(),
                    before: None,
                    open: false,
                })
                .await
                .is_ok()
        );
        assert!(
            operations
                .send(crate::operations::OperationCommand::Inspect {
                    tag,
                    target: matrix_domain::EventId::new(target)
                })
                .await
                .is_ok()
        );
    });
    std::thread::sleep(Duration::from_millis(150));
    let start = Instant::now();
    worker.shutdown().unwrap();
    assert!(start.elapsed() < Duration::from_millis(750));
}
