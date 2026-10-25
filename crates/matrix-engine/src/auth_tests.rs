//! Real-server tests run only inside the disposable network namespace.
include!("receipt_tests.rs");
include!("live_activity_tests.rs");

use crate::auth::{AccountSession, AuthError};
use crate::discovery::HomeserverUrl;
#[cfg(unix)]
use crate::persistence::AccountStore;
use matrix_domain::AccountId;

include!("lifecycle_tests.rs");
include!("encryption_tests.rs");
include!("verification_tests.rs");
include!("recovery_tests.rs");
include!("media_tests.rs");
include!("account_tests.rs");
include!("signin_tests.rs");
include!("history_edits_tests.rs");
include!("reaction_retry_tests.rs");
include!("redaction_retry_tests.rs");

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn fixture() -> serde_json::Value {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("isolated fixture required");
    serde_json::from_slice(&std::fs::read(path).expect("fixture readable")).expect("fixture valid")
}

fn http(config: &serde_json::Value) -> reqwest::Client {
    let pem = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    super::secure_http_builder()
        .no_proxy()
        .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
        .build()
        .unwrap()
}

async fn login(config: &serde_json::Value, name: &str, account: u64) -> AccountSession {
    AccountSession::password_login_with_http(
        AccountId::new(account),
        HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
        &format!("@{name}:matrix-desktop.test"),
        config[format!("{name}_password")].as_str().unwrap(),
        http(config),
    )
    .await
    .expect("fixture login accepted")
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_lifecycle_shutdown_drains_accepted_send_before_join() {
    use crate::{RoomListEvent, RoomSyncStatus, room_sync::RoomListWorker, sending::SendRequest};
    use matrix_domain::{RoomId, RoomScope, TransactionId};
    let config = fixture();
    let room = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"drain","visibility":"private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let login_config = config.clone();
    let (mut worker, mut rooms) =
        RoomListWorker::spawn(move || async move { Ok(login(&login_config, "alice", 22).await) })
            .unwrap();
    let (mut sends, _results) = worker.take_send_route().unwrap();
    runtime().block_on(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !matches!(
                rooms.recv().await,
                Some(RoomListEvent::Status(RoomSyncStatus::Ready))
            ) {}
        })
        .await
        .unwrap();
        assert!(
            sends
                .send(SendRequest {
                    scope: RoomScope {
                        account_id: AccountId::new(22),
                        room_id: RoomId::new(&room)
                    },
                    transaction_id: TransactionId::new("p022-drain"),
                    body: "Shutdown città".into()
                })
                .await
                .is_ok()
        );
    });
    // Producer/result receiver deliberately remain alive: shutdown must close intake
    // and drain accepted writes rather than wait for the UI to release its routes.
    worker.shutdown().unwrap();
    runtime().block_on(async {
        let messages = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            messages["chunk"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["content"]["body"] == "Shutdown città")
                .count(),
            1,
            "accepted send must reach server before join returns"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_reply_preserves_relation_and_transaction() {
    use crate::{
        operations::{Operation, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"relations","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Original città"}),
        )
        .await;
        let mut sync = RoomSync::new(login(&config, "alice", 21).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(21),
                room_id: RoomId::new(room),
            },
            transaction_id: TransactionId::new("p021-reply"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Reply {
                body: "Risposta città\nseconda riga".into(),
            },
        };
        let first = sync.operate(&request).await.expect("reply accepted");
        let retry = sync.operate(&request).await.expect("reply retry accepted");
        assert!(
            first == retry,
            "same transaction yields same reply identity"
        );
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", first.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["content"]["body"] == "Risposta città\nseconda riga");
        assert!(
            event["content"]["m.relates_to"]["m.in_reply_to"]["event_id"]
                == request.target.as_str()
        );
    });
}

#[test]
#[ignore = "requires isolated homeserver; needs the external test harness (not in this repo)"]
fn disposable_history_rejects_replacement_with_changed_message_type() {
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"type","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let original = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/original"),
            serde_json::json!({"msgtype":"m.text","body":"Originale città 🦀"}),
        )
        .await;
        room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/invalid-type"),
            serde_json::json!({"msgtype":"m.notice","body":"* Invalid",
                "m.new_content":{"msgtype":"m.notice","body":"Invalid"},
                "m.relates_to":{"rel_type":"m.replace","event_id":original["event_id"]}}),
        )
        .await;
        let mut sync = crate::room_sync::RoomSync::new(login(&config, "alice", 51).await);
        sync.next_batch().await.unwrap();
        let mut views = crate::ViewRequests::new();
        let page = sync
            .history_page(
                views
                    .start(matrix_domain::RoomScope {
                        account_id: AccountId::new(51),
                        room_id: matrix_domain::RoomId::new(room),
                    })
                    .unwrap(),
                None,
            )
            .await
            .unwrap();
        assert_eq!(page.events.len(), 1);
        assert!(page.events[0].event_id.as_str() == original["event_id"].as_str().unwrap());
        assert_eq!(page.events[0].body, "Originale città 🦀");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_edit_uses_replacement_without_second_message() {
    use crate::{
        ViewRequests,
        operations::{Operation, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"edit","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Prima"}),
        )
        .await;
        let mut sync = RoomSync::new(login(&config, "alice", 21).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(21),
                room_id: RoomId::new(room),
            },
            transaction_id: TransactionId::new("p021-edit"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Edit {
                body: "Modificato città\nseconda riga".into(),
            },
        };
        let sent = sync.operate(&request).await.expect("edit accepted");
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", sent.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["content"]["m.new_content"]["body"] == "Modificato città\nseconda riga");
        assert!(event["content"]["m.relates_to"]["rel_type"] == "m.replace");
        assert!(event["content"]["m.relates_to"]["event_id"] == request.target.as_str());
        let mut views = ViewRequests::new();
        let page = sync
            .history_page(views.start(request.scope).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(page.events.len(), 1, "edit is not a second chat bubble");
        assert!(page.events[0].event_id == request.target);
        assert!(page.events[0].body == "Modificato città\nseconda riga");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_rejects_edit_of_another_sender() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"permission","visibility":"private","invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap();
        room_request(&config, "bob", reqwest::Method::POST, &format!("rooms/{room}/join"), serde_json::json!({})).await;
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Non modificare"})).await;
        let mut sync = RoomSync::new(login(&config, "bob", 21).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(21), room_id: RoomId::new(room) },
            transaction_id: TransactionId::new("p021-denied-edit"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Edit { body: "Non autorizzato".into() },
        };
        let details = sync.message_details(&request.scope, &request.target).await.expect("message permissions");
        assert!(!details.permissions.edit && !details.permissions.redact);
        assert!(details.permissions.reply && details.permissions.react);
        assert!(details.body == "Non modificare");
        assert!(matches!(sync.operate(&request).await, Err(OperationError::Forbidden)));
        let events = room_request(&config, "bob", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        assert_eq!(events["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.room.message").count(), 1);
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_redaction_removes_content_but_retains_timeline_identity() {
    use crate::{
        ViewRequests,
        operations::{Operation, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"redact","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Disposable removal only"}),
        )
        .await;
        let mut sync = RoomSync::new(login(&config, "alice", 21).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(21),
                room_id: RoomId::new(room),
            },
            transaction_id: TransactionId::new("p021-redact"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Redact,
        };
        let sent = sync.operate(&request).await.expect("redaction accepted");
        assert!(sync.operate(&request).await.unwrap() == sent);
        let original = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", request.target.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(original["content"].get("body").is_none());
        assert!(original["unsigned"]["redacted_because"]["event_id"] == sent.as_str());
        let mut views = ViewRequests::new();
        let page = sync
            .history_page(views.start(request.scope).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(page.events.len(), 1, "redacted placeholder must remain");
        assert!(page.events[0].event_id == request.target);
        assert!(page.events[0].body == "[Messaggio eliminato]");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_reaction_is_idempotent_and_can_be_removed() {
    use crate::{
        operations::{Operation, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"reaction","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Reazione"}),
        )
        .await;
        let mut sync = RoomSync::new(login(&config, "alice", 21).await);
        sync.next_batch().await.unwrap();
        let mut request = OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(21),
                room_id: RoomId::new(room),
            },
            transaction_id: TransactionId::new("p021-react"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::React { key: "👍".into() },
        };
        let sent = sync.operate(&request).await.expect("reaction accepted");
        assert!(sync.operate(&request).await.unwrap() == sent);
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", sent.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["type"] == "m.reaction");
        assert!(event["content"]["m.relates_to"]["rel_type"] == "m.annotation");
        assert!(event["content"]["m.relates_to"]["event_id"] == request.target.as_str());
        assert!(event["content"]["m.relates_to"]["key"] == "👍");
        request.operation = Operation::Redact;
        request.target = sent;
        request.transaction_id = TransactionId::new("p021-remove-reaction");
        sync.operate(&request)
            .await
            .expect("remove reaction accepted");
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", request.target.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["content"].as_object().unwrap().is_empty());
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_redaction_denial_is_typed() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"forbidden","visibility":"private","invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap();
        room_request(&config, "bob", reqwest::Method::POST, &format!("rooms/{room}/join"), serde_json::json!({})).await;
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Protected"})).await;
        let mut sync = RoomSync::new(login(&config, "bob", 21).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(21), room_id: RoomId::new(room) },
            transaction_id: TransactionId::new("p021-denied-redact"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Redact,
        };
        assert!(matches!(sync.operate(&request).await, Err(OperationError::Forbidden)));
        let event = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", request.target.as_str()), serde_json::json!({})).await;
        assert!(event["content"]["body"] == "Protected");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_operation_worker_routes_inspection_and_durable_reply() {
    use crate::{
        ViewRequests,
        operations::{Operation, OperationCommand, OperationRequest, OperationResult},
        room_sync::RoomListWorker,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let (room, target) = runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"worker","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"Worker original"}),
        )
        .await;
        (room, EventId::new(target["event_id"].as_str().unwrap()))
    });
    let login_config = config.clone();
    let (mut worker, mut rooms) =
        RoomListWorker::spawn(move || async move { Ok(login(&login_config, "alice", 21).await) })
            .unwrap();
    let (mut commands, mut results) = worker.take_operation_route().expect("operation route");
    runtime().block_on(async {
        while !matches!(
            rooms.recv().await,
            Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
        ) {}
        let scope = RoomScope {
            account_id: AccountId::new(21),
            room_id: RoomId::new(&room),
        };
        let mut views = ViewRequests::new();
        let tag = views.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::Inspect {
                    tag: tag.clone(),
                    target: target.clone()
                })
                .await
                .is_ok()
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
            .await
            .unwrap()
            .unwrap();
        let OperationResult::Inspected {
            tag: actual,
            target: actual_id,
            details,
        } = result
        else {
            panic!("expected inspection");
        };
        assert!(views.accepts(&actual) && actual_id == target);
        let details = details.expect("inspection succeeded");
        assert!(details.body == "Worker original");
        assert!(
            details.permissions.edit
                && details.permissions.redact
                && details.permissions.reply
                && details.permissions.react
        );
        let request = OperationRequest {
            scope,
            transaction_id: TransactionId::new("p021-worker-reply"),
            target,
            operation: Operation::Reply {
                body: "Worker reply".into(),
            },
        };
        assert!(
            commands
                .send(OperationCommand::Apply(request.clone()))
                .await
                .is_ok()
        );
        views.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
            .await
            .unwrap()
            .unwrap();
        let OperationResult::Applied {
            request: actual,
            result,
        } = result
        else {
            panic!("expected durable result");
        };
        assert!(actual.scope == request.scope && actual.transaction_id == request.transaction_id);
        let id = result.expect("reply accepted after view cancellation");
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", id.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["content"]["body"] == "Worker reply");
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_send_reuses_transaction_and_reads_actual_event() {
    use crate::{room_sync::RoomSync, sending::SendRequest};
    use matrix_domain::{RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"send","visibility":"private"}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        let mut sync = RoomSync::new(login(&config, "alice", 20).await);
        sync.next_batch().await.unwrap();
        let request = SendRequest {
            scope: RoomScope {
                account_id: AccountId::new(20),
                room_id: RoomId::new(room),
            },
            transaction_id: TransactionId::new("p020-stable-transaction"),
            body: "città · prima riga\nseconda riga".into(),
        };
        let first = sync.send_text(&request).await.expect("SDK send accepted");
        let second = sync
            .send_text(&request)
            .await
            .expect("idempotent retry accepted");
        assert!(
            first == second,
            "same Matrix transaction must return same event ID"
        );
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", first.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(event["event_id"] == first.as_str());
        assert!(event["content"]["body"] == request.body);
        assert!(event["sender"] == "@alice:matrix-desktop.test");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_send_worker_survives_offline_retry() {
    use crate::{room_sync::RoomListWorker, sending::SendRequest};
    use matrix_domain::{RoomId, RoomScope, TransactionId};
    let config = fixture();
    let id = runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"worker","visibility":"private"}),
        )
        .await["room_id"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let login_config = config.clone();
    let (mut worker, mut rooms) =
        RoomListWorker::spawn(move || async move { Ok(login(&login_config, "alice", 20).await) })
            .unwrap();
    let (mut commands, mut results) = worker.take_send_route().expect("durable route");
    runtime().block_on(async {
        while !matches!(
            rooms.recv().await,
            Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
        ) {}
        let request = SendRequest {
            scope: RoomScope {
                account_id: AccountId::new(20),
                room_id: RoomId::new(&id),
            },
            transaction_id: TransactionId::new("p020-offline-retry"),
            body: "Offline retry preserves città\nsecond line".into(),
        };
        let pid = config["server_pid"].to_string();
        struct Resume(String);
        impl Drop for Resume {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/kill")
                    .args(["-CONT", &self.0])
                    .status();
            }
        }
        assert!(
            std::process::Command::new("/usr/bin/kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
        let resume = Resume(pid);
        assert!(commands.send(request.clone()).await.is_ok());
        let failed = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(failed.result.is_err(), "offline send must report failure");
        assert!(failed.transaction_id == request.transaction_id && failed.scope == request.scope);
        // /sync has a 15s deadline; its failure must not close the send route.
        tokio::time::sleep(std::time::Duration::from_secs(16)).await;
        drop(resume);
        assert!(commands.send(request.clone()).await.is_ok());
        let sent = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
            .await
            .unwrap()
            .unwrap();
        let event_id = sent
            .result
            .expect("same worker retries after network recovery");
        assert!(sent.transaction_id == request.transaction_id && sent.scope == request.scope);
        assert!(commands.send(request.clone()).await.is_ok());
        let again = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(again.result.unwrap() == event_id);
        let messages = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{id}/messages?dir=b&limit=100"),
            serde_json::json!({}),
        )
        .await;
        let matching: Vec<_> = messages["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "m.room.message")
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "offline retry must not duplicate server event"
        );
        assert!(matching[0]["event_id"] == event_id.as_str());
        assert!(matching[0]["content"]["body"] == request.body);
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_timeline_pages_preserve_server_order_and_ids() {
    use crate::{ViewRequests, room_sync::RoomSync};
    use matrix_domain::{RoomId, RoomScope};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"pages","visibility":"private"}),
        )
        .await;
        let id = created["room_id"].as_str().unwrap();
        let mut ids = Vec::new();
        for i in 0..65 {
            let sent = room_request(
                &config,
                "alice",
                reqwest::Method::PUT,
                &format!("rooms/{id}/send/m.room.message/p019-{i}"),
                serde_json::json!({"msgtype":"m.text","body":format!("message {i:03}")}),
            )
            .await;
            ids.push(sent["event_id"].as_str().unwrap().to_owned());
        }
        let mut sync = RoomSync::new(login(&config, "alice", 19).await);
        sync.next_batch().await.unwrap();
        let mut views = ViewRequests::new();
        let scope = RoomScope {
            account_id: AccountId::new(19),
            room_id: RoomId::new(id),
        };
        let tag = views.start(scope).unwrap();
        let mut cursor = None;
        let mut collected = Vec::new();
        for _ in 0..10 {
            let page = sync
                .history_page(tag.clone(), cursor.clone())
                .await
                .expect("server history");
            let mut older = page
                .events
                .into_iter()
                .map(|e| (e.event_id.as_str().to_owned(), e.body))
                .collect::<Vec<_>>();
            older.append(&mut collected);
            collected = older;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert!(cursor.is_none(), "reached server history start");
        assert_eq!(collected.len(), 65);
        for (i, (id, body)) in collected.iter().enumerate() {
            assert!(id == &ids[i], "server event identity/order retained");
            assert!(
                body == &format!("message {i:03}"),
                "server text retained"
            );
        }
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_timeline_worker_routes_history_beside_sync() {
    use crate::{ViewRequests, history::HistoryRequest, room_sync::RoomListWorker};
    use matrix_domain::{RoomId, RoomScope};
    let config = fixture();
    let id = runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"worker","visibility":"private"}),
        )
        .await;
        let id = created["room_id"].as_str().unwrap().to_owned();
        room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{id}/send/m.room.message/worker"),
            serde_json::json!({"msgtype":"m.text","body":"Worker history"}),
        )
        .await;
        id
    });
    let (mut worker, mut rooms) =
        RoomListWorker::spawn(move || async move { Ok(login(&config, "alice", 19).await) })
            .unwrap();
    let (mut commands, mut history) = worker.take_history_route().unwrap();
    runtime().block_on(async {
        loop {
            if matches!(
                rooms.recv().await,
                Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready))
            ) {
                break;
            }
        }
        let mut views = ViewRequests::new();
        let tag = views
            .start(RoomScope {
                account_id: AccountId::new(19),
                room_id: RoomId::new(id),
            })
            .unwrap();
        assert!(
            commands
                .send(HistoryRequest { tag, before: None, open: false })
                .await
                .is_ok()
        );
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), history.recv())
            .await
            .unwrap()
            .unwrap();
        let page = result.page.expect("history while sync long-polls");
        assert!(views.accepts(&page.tag));
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].body, "Worker history");
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_timeline_cancels_inflight_request_without_server_reply() {
    use crate::{ViewRequests, history::HistoryError, room_sync::RoomSync};
    use matrix_domain::{RoomId, RoomScope};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"cancellation","visibility":"private"}),
        )
        .await;
        let mut sync = RoomSync::new(login(&config, "alice", 19).await);
        sync.next_batch().await.unwrap();
        let mut views = ViewRequests::new();
        let tag = views
            .start(RoomScope {
                account_id: AccountId::new(19),
                room_id: RoomId::new(created["room_id"].as_str().unwrap()),
            })
            .unwrap();
        let pid = config["server_pid"].to_string();
        struct Resume(String);
        impl Drop for Resume {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/kill")
                    .args(["-CONT", &self.0])
                    .status();
            }
        }
        assert!(
            std::process::Command::new("/usr/bin/kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
        let _resume = Resume(pid);
        let cancel = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            views.cancel();
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(750),
            sync.history_page(tag, None),
        )
        .await;
        cancel.await.unwrap();
        assert!(
            matches!(result, Ok(Err(HistoryError::Cancelled))),
            "room navigation must cancel actual pending HTTP within 750ms"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_sync_tracks_created_room() {
    use crate::RoomListDiff;
    use crate::room_sync::RoomSync;
    let config = fixture();
    runtime().block_on(async {
        let response = http(&config)
            .post(format!(
                "{}_matrix/client/v3/createRoom",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(config["alice_token"].as_str().unwrap())
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({"name": "laboratorio", "visibility": "private"})
                    .to_string(),
            )
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "fixture create accepted");
        let room_id = serde_json::from_str::<serde_json::Value>(&response.text().await.unwrap())
            .unwrap()["room_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut sync = RoomSync::new(login(&config, "alice", 18).await);
        let changes = sync.next_batch().await.expect("room sync");
        let room = changes
            .iter()
            .find_map(|change| match change {
                RoomListDiff::Upsert(room) if room.scope.room_id.as_str() == room_id => Some(room),
                _ => None,
            })
            .expect("created room is projected");
        assert_eq!(room.scope.account_id, AccountId::new(18));
        assert_eq!(room.name, "laboratorio");
        assert_eq!(room.unread, 0);
        assert!(
            sync.next_batch().await.unwrap().is_empty(),
            "unchanged rooms produce no diffs"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_spaces_room_details_from_sync() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Lab", "topic":"Topic from server", "visibility":"private"})).await;
        let id = created["room_id"].as_str().unwrap();
        let mut sync = RoomSync::new(login(&config, "alice", 29).await);
        let batch = sync.next_batch().await.unwrap();
        let room = batch.iter().find_map(|d| match d {
            RoomListDiff::Upsert(r) if r.scope.room_id.as_str() == id => Some(r), _ => None,
        }).unwrap();
        assert_eq!(room.info.topic.as_deref(), Some("Topic from server"));
        assert_eq!(room.info.joined_members, 1);
        assert!(!room.info.is_space);
        // SDK does not certify absence of encryption from this /sync snapshot.
        assert_eq!(room.info.encrypted, None);
        assert!(sync.next_batch().await.unwrap().is_empty());
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_spaces_children_follow_server_state() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let child = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Child", "visibility":"private"})).await;
        let child = child["room_id"].as_str().unwrap();
        let space = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Space", "visibility":"private", "creation_content":{"type":"m.space"}})).await;
        let space = space["room_id"].as_str().unwrap();
        let path = format!("rooms/{space}/state/m.space.child/{child}");
        room_request(&config, "alice", reqwest::Method::PUT, &path,
            serde_json::json!({"via":["matrix-desktop.test"]})).await;
        let mut sync = RoomSync::new(login(&config, "alice", 29).await);
        let batch = sync.next_batch().await.unwrap();
        let summary = batch.iter().find_map(|d| match d {
            RoomListDiff::Upsert(r) if r.scope.room_id.as_str() == space => Some(r), _ => None,
        }).unwrap();
        assert!(summary.info.is_space);
        assert_eq!(summary.info.children.len(), 1);
        assert_eq!(summary.info.children[0].as_str(), child);
        assert!(sync.next_batch().await.unwrap().is_empty());
        room_request(&config, "alice", reqwest::Method::PUT, &path, serde_json::json!({})).await;
        let batch = sync.next_batch().await.unwrap();
        assert!(batch.iter().any(|d| matches!(d, RoomListDiff::Upsert(r) if r.scope.room_id.as_str() == space && r.info.children.is_empty())));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_spaces_invalid_links_redaction_no_autojoin() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Encrypted", "topic":"Original", "invite":["@bob:matrix-desktop.test"],
                "initial_state":[{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]})).await;
        let room = room["room_id"].as_str().unwrap();
        let hidden = room_request(&config, "bob", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Not joined"})).await;
        let hidden = hidden["room_id"].as_str().unwrap();
        let space = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Space", "creation_content":{"type":"m.space"}})).await;
        let space = space["room_id"].as_str().unwrap();
        let nested = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"Nested", "creation_content":{"type":"m.space"}})).await;
        let nested = nested["room_id"].as_str().unwrap();
        let mut linked_event = None;
        for (parent, child) in [(space, room), (space, nested), (space, hidden), (nested, space), (room, hidden)] {
            let event = room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{parent}/state/m.space.child/{child}"), serde_json::json!({"via":["matrix-desktop.test"]})).await;
            if parent == space && child == room { linked_event = event["event_id"].as_str().map(str::to_owned); }
        }
        for content in [serde_json::json!({}), serde_json::json!({"via":[]}), serde_json::json!({"via":["not a server"]})] {
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{space}/state/m.space.child/!invalid:matrix-desktop.test"), content).await;
        }
        let mut sync = RoomSync::new(login(&config, "alice", 29).await);
        let batch = sync.next_batch().await.unwrap();
        assert_eq!(batch.len(), 3, "no unjoined child is joined or projected");
        let summaries: Vec<_> = batch.iter().filter_map(|d| match d { RoomListDiff::Upsert(r) => Some(r), _ => None }).collect();
        let parent = summaries.iter().find(|r| r.scope.room_id.as_str() == space).unwrap();
        assert_eq!(parent.info.children.len(), 3, "invalid via is not an active relationship");
        let encrypted = summaries.iter().find(|r| r.scope.room_id.as_str() == room).unwrap();
        assert_eq!(encrypted.info.encrypted, Some(true));
        assert!(encrypted.info.children.is_empty(), "ordinary rooms cannot invent Spaces");
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        room_request(&config, "alice", reqwest::Method::PUT, &format!("rooms/{room}/state/m.room.topic"), serde_json::json!({"topic":"Updated"})).await;
        room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{space}/redact/{}/p029-link-redact", linked_event.unwrap()), serde_json::json!({})).await;
        let batch = sync.next_batch().await.unwrap();
        assert!(batch.iter().any(|d| matches!(d, RoomListDiff::Upsert(r) if r.scope.room_id.as_str() == room && r.info.topic.as_deref() == Some("Updated") && r.info.joined_members == 2 && r.info.encrypted == Some(true))));
        assert!(batch.iter().any(|d| matches!(d, RoomListDiff::Upsert(r) if r.scope.room_id.as_str() == space && r.info.children.len() == 2 && !r.info.children.iter().any(|id| id.as_str() == room))));
        assert!(sync.next_batch().await.unwrap().is_empty());
    });
}

async fn room_request(
    config: &serde_json::Value,
    actor: &str,
    method: reqwest::Method,
    path: &str,
    body: serde_json::Value,
) -> serde_json::Value {
    let response = http(config)
        .request(
            method,
            format!(
                "{}_matrix/client/v3/{path}",
                config["url"].as_str().unwrap()
            ),
        )
        .bearer_auth(config[format!("{actor}_token")].as_str().unwrap())
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "room fixture request accepted: status {}",
        response.status().as_u16()
    );
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_sync_removes_left_room() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "leave", "visibility": "private"})).await;
        let id = created["room_id"].as_str().unwrap();
        let mut sync = RoomSync::new(login(&config, "alice", 18).await);
        assert_eq!(sync.next_batch().await.unwrap().len(), 1);
        room_request(&config, "alice", reqwest::Method::POST, &format!("rooms/{id}/leave"), serde_json::json!({})).await;
        let changes = sync.next_batch().await.unwrap();
        assert!(changes.iter().any(|change| matches!(change, RoomListDiff::Remove(scope) if scope.room_id.as_str() == id && scope.account_id == AccountId::new(18))), "left room must be removed");
        assert!(sync.next_batch().await.unwrap().is_empty());
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_sync_tracks_name_unread_read_marker() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "bob", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "shared", "visibility": "private", "invite": ["@alice:matrix-desktop.test"]})).await;
        let id = created["room_id"].as_str().unwrap();
        room_request(&config, "alice", reqwest::Method::POST, &format!("rooms/{id}/join"), serde_json::json!({})).await;
        let mut sync = RoomSync::new(login(&config, "alice", 18).await);
        assert_eq!(sync.next_batch().await.unwrap().len(), 1);
        room_request(&config, "bob", reqwest::Method::PUT, &format!("rooms/{id}/state/m.room.name"), serde_json::json!({"name": "renamed"})).await;
        let sent = room_request(&config, "bob", reqwest::Method::PUT, &format!("rooms/{id}/send/m.room.message/p018-message"), serde_json::json!({"msgtype": "m.text", "body": "Disposable unread fixture"})).await;
        let changes = sync.next_batch().await.unwrap();
        let room = changes.iter().find_map(|change| match change {
            RoomListDiff::Upsert(room) if room.scope.room_id.as_str() == id => Some(room), _ => None,
        }).expect("rename/message updated room");
        assert_eq!(room.name, "renamed");
        assert!(room.unread >= 1, "server notification count must reach projection");
        room_request(&config, "alice", reqwest::Method::POST, &format!("rooms/{id}/read_markers"), serde_json::json!({"m.fully_read": sent["event_id"], "m.read": sent["event_id"]})).await;
        let changes = sync.next_batch().await.unwrap();
        assert!(changes.iter().any(|change| matches!(change, RoomListDiff::Upsert(room) if room.scope.room_id.as_str() == id && room.unread == 0)), "read marker must clear projected notification count");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_unnamed_room_is_listed_by_participants_not_by_id() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility": "private", "invite": ["@bob:matrix-desktop.test"]})).await;
        let id = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("rooms/{id}/join"), serde_json::json!({})).await;
        let mut sync = RoomSync::new(login(&config, "alice", 18).await);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let name = loop {
            let found = sync.next_batch().await.unwrap().into_iter().find_map(|change| match change {
                RoomListDiff::Upsert(room) if room.scope.room_id.as_str() == id && room.name != id => Some(room.name),
                _ => None,
            });
            if let Some(name) = found { break name; }
            assert!(std::time::Instant::now() < deadline, "unnamed room never got a computed name");
        };
        assert!(name.contains("bob"), "computed name should come from the participant, got {name}");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_preview_is_last_real_message_and_clears_when_deleted() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility": "private", "name": "preview", "invite": ["@bob:matrix-desktop.test"]})).await;
        let id = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("rooms/{id}/join"), serde_json::json!({})).await;
        let send = async |txn: &str, body: &str| room_request(&config, "bob", reqwest::Method::PUT,
            &format!("rooms/{id}/send/m.room.message/{txn}"), serde_json::json!({"msgtype": "m.text", "body": body})).await;
        send("t1", "primo").await;
        let last = send("t2", "ultimo messaggio").await["event_id"].as_str().unwrap().to_owned();
        let mut sync = RoomSync::new(login(&config, "alice", 19).await);
        let mut wait_for = async |want: Option<&str>| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                for change in sync.next_batch().await.unwrap() {
                    if let RoomListDiff::Upsert(room) = change
                        && room.scope.room_id.as_str() == id
                        && room.info.preview.as_deref() == want
                    {
                        return;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "preview never became {want:?}");
            }
        };
        wait_for(Some("ultimo messaggio")).await;
        room_request(&config, "bob", reqwest::Method::PUT, &format!("rooms/{id}/redact/{last}/t3"), serde_json::json!({})).await;
        wait_for(None).await;
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_worker_delivers_on_bounded_background_route() {
    use crate::{
        RoomListDiff,
        room_sync::{RoomListEvent, RoomListWorker},
    };
    let config = fixture();
    runtime().block_on(async {
        room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name": "worker", "visibility": "private"}),
        )
        .await;
    });
    let start = std::time::Instant::now();
    let (worker, mut receiver) =
        RoomListWorker::spawn(move || async move { Ok(login(&config, "alice", 18).await) })
            .unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "startup must not await login or network"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(RoomListEvent::Diff(RoomListDiff::Upsert(room))) = receiver.try_recv() {
            assert_eq!(room.name, "worker");
            assert_eq!(room.scope.account_id, AccountId::new(18));
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "worker update deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let stop = std::time::Instant::now();
    worker.shutdown().unwrap();
    assert!(
        stop.elapsed() < std::time::Duration::from_secs(2),
        "shutdown must cancel sync"
    );
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_list_gui_store_trusted_restore() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let phrase = "disposable GUI passphrase only";
    runtime().block_on(async {
        let session = AccountStore::create(root, AccountId::new(18))
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
        assert_eq!(
            session.whoami().await.unwrap().user_id(),
            "@alice:matrix-desktop.test"
        );
    });
    runtime().block_on(async {
        let session = AccountStore::open(root, AccountId::new(18))
            .unwrap()
            .restore_with_trusted_root(
                phrase,
                &std::fs::read(config["ca"].as_str().unwrap()).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            session.whoami().await.unwrap().user_id(),
            "@alice:matrix-desktop.test"
        );
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires the closed disposable GUI profile"]
fn disposable_gui_delivery_exact_transaction_readback() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let output = std::path::Path::new(config["delivery_readback"].as_str().unwrap());
    let evidence: serde_json::Value = serde_json::from_slice(
        &std::fs::read(output.join("delivery-server-readback.json")).unwrap(),
    )
    .unwrap();
    runtime().block_on(async {
        let session = AccountStore::open(root, AccountId::new(18)).unwrap()
            .restore_with_http("disposable GUI passphrase only", http(&config)).await.unwrap();
        let identity = session.whoami().await.unwrap();
        assert_eq!(identity.user_id(), "@alice:matrix-desktop.test");
        let token = session.client.as_ref().unwrap().access_token().unwrap();
        let client = http(&config);
        let room = evidence["room"].as_str().unwrap();
        let event_id = evidence["event"]["event_id"].as_str().unwrap();
        let base = format!("{}_matrix/client/v3/rooms/{room}", config["url"].as_str().unwrap());
        let response = client.get(format!("{base}/event/{event_id}")).bearer_auth(&token)
            .send().await.unwrap().error_for_status().unwrap();
        let event: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(event["event_id"], event_id);
        assert_eq!(event["sender"], identity.user_id());
        assert_eq!(event["type"], "m.room.message");
        assert_eq!(event["content"], evidence["event"]["content"]);
        let tx = event["unsigned"]["transaction_id"].as_str().expect("same-device transaction proof");
        assert!(!tx.is_empty());
        let response = client.put(format!("{base}/send/m.room.message/{tx}"))
            .bearer_auth(&token).header("Content-Type", "application/json")
            .body(event["content"].to_string()).send().await.unwrap().error_for_status().unwrap();
        let replay: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        assert_eq!(replay["event_id"], event_id);
        let response = client.get(format!("{base}/messages?dir=b&limit=100")).bearer_auth(&token)
            .send().await.unwrap().error_for_status().unwrap();
        let history: serde_json::Value = serde_json::from_str(&response.text().await.unwrap()).unwrap();
        let matching = history["chunk"].as_array().unwrap().iter()
            .filter(|row| row["content"] == event["content"]).collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0]["event_id"], event_id);
        std::fs::write(output.join("delivery-transaction-readback.json"), serde_json::to_vec_pretty(
            &serde_json::json!({"room": room, "event_id": event_id, "sender": identity.user_id(),
                "device": identity.device_id(), "transaction_id": tx, "replay_event_id": replay["event_id"],
                "matching_events_after_replay": matching.len()})).unwrap()).unwrap();
    });
}

async fn token_status(config: &serde_json::Value, token: &str) -> u16 {
    http(config)
        .get(format!(
            "{}_matrix/client/v3/account/whoami",
            config["url"].as_str().unwrap()
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("whoami HTTP response")
        .status()
        .as_u16()
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_persistent_session_restores_after_runtime_restart() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let passphrase = "long disposable restore fixture passphrase";
    let (user, device, token, signing_key) = runtime().block_on(async {
        let store = AccountStore::create(root, AccountId::new(117)).unwrap();
        let session = store
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await
            .expect("persistent login");
        let identity = session.whoami().await.unwrap();
        (
            identity.user_id().to_owned(),
            identity.device_id().to_owned(),
            session.client.as_ref().unwrap().access_token().unwrap(),
            session
                .client
                .as_ref()
                .unwrap()
                .encryption()
                .get_own_device()
                .await
                .unwrap()
                .expect("own crypto device")
                .ed25519_key()
                .expect("signing key")
                .to_base64(),
        )
    });
    // Dropping the entire runtime closes the SDK pools/tasks, not just the wrapper.
    runtime().block_on(async {
        let store = AccountStore::open(root, AccountId::new(117)).unwrap();
        let session = store
            .restore_with_http(passphrase, http(&config))
            .await
            .expect("restore");
        assert_eq!(session.account_id(), AccountId::new(117));
        let identity = session.whoami().await.unwrap();
        assert!(identity.user_id() == user);
        assert!(identity.device_id() == device);
        assert!(session.client.as_ref().unwrap().access_token().unwrap() == token);
        let restored_key = session
            .client
            .as_ref()
            .unwrap()
            .encryption()
            .get_own_device()
            .await
            .unwrap()
            .expect("restored own crypto device")
            .ed25519_key()
            .expect("restored signing key")
            .to_base64();
        assert!(
            restored_key == signing_key,
            "crypto identity must survive restart"
        );
        assert_eq!(token_status(&config, &token).await, 200);
    });
    for entry in std::fs::read_dir(root.join("account-117")).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let bytes = std::fs::read(path).unwrap();
            for secret in [
                token.as_str(),
                passphrase,
                config["alice_password"].as_str().unwrap(),
            ] {
                assert!(
                    !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                    "secret on disk"
                );
            }
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_persistent_logout_preserves_other_account() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let passphrase = "long disposable account separation passphrase";
    runtime().block_on(async {
        let mut alice = AccountStore::create(root, AccountId::new(117))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await
            .unwrap();
        let bob = AccountStore::create(root, AccountId::new(118))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await
            .unwrap();
        let token = alice.client.as_ref().unwrap().access_token().unwrap();
        alice.logout().await.expect("persistent logout");
        assert!(
            !root.join("account-117/session.sealed").exists(),
            "logout must remove its durable credential"
        );
        assert!(root.join("account-118/session.sealed").is_file());
        assert_eq!(token_status(&config, &token).await, 401);
        assert!(bob.whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
    });
    runtime().block_on(async {
        let alice = AccountStore::open(root, AccountId::new(117))
            .unwrap()
            .restore_with_http(passphrase, http(&config))
            .await;
        assert!(matches!(alice, Err(AuthError::Storage)));
        let bob = AccountStore::open(root, AccountId::new(118))
            .unwrap()
            .restore_with_http(passphrase, http(&config))
            .await
            .unwrap();
        assert!(bob.whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_persistent_restore_fails_closed_after_revocation() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let passphrase = "long disposable revocation fixture passphrase";
    let token = runtime().block_on(async {
        let session = AccountStore::create(root, AccountId::new(117))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await
            .unwrap();
        session.client.as_ref().unwrap().access_token().unwrap()
    });
    runtime().block_on(async {
        let sealed = root.join("account-117/session.sealed");
        let before = std::fs::read(&sealed).unwrap();
        let wrong_key = AccountStore::open(root, AccountId::new(117))
            .unwrap()
            .restore("wrong disposable unlock passphrase")
            .await;
        assert!(matches!(wrong_key, Err(AuthError::Storage)));
        assert_eq!(std::fs::read(&sealed).unwrap(), before);
        assert_eq!(token_status(&config, &token).await, 200);
        let response = http(&config)
            .post(format!(
                "{}_matrix/client/v3/logout",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(token_status(&config, &token).await, 401);
        let revoked = AccountStore::open(root, AccountId::new(117))
            .unwrap()
            .restore_with_http(passphrase, http(&config))
            .await;
        assert!(matches!(revoked, Err(AuthError::SessionExpired)));
        assert_eq!(std::fs::read(sealed).unwrap(), before);
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_persistent_existing_store_cannot_be_rebound() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap())
        .parent()
        .unwrap();
    let passphrase = "long disposable rebinding fixture passphrase";
    runtime().block_on(async {
        let mut alice = AccountStore::create(root, AccountId::new(117))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await
            .unwrap();
        alice.logout().await.unwrap();
    });
    runtime().block_on(async {
        let result = AccountStore::open(root, AccountId::new(117))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
                passphrase,
                http(&config),
            )
            .await;
        assert!(
            matches!(result, Err(AuthError::Storage)),
            "retained crypto store must never be reused by a new login"
        );
        assert!(!root.join("account-117/session.sealed").exists());
    });
}

#[test]
fn public_login_rejects_localparts_before_network() {
    runtime().block_on(async {
        let result = AccountSession::password_login(
            AccountId::new(16),
            HomeserverUrl::parse("https://fixture.invalid").unwrap(),
            "alice",
            "unused-fixture-password",
        )
        .await;
        assert!(matches!(result, Err(AuthError::InvalidUserId)));
    });
}

#[test]
fn authentication_errors_contain_only_fixed_labels() {
    for (error, label) in [
        (AuthError::Transport, "Transport"),
        (AuthError::InvalidUserId, "InvalidUserId"),
        (AuthError::LoginFailed, "LoginFailed"),
        (AuthError::CredentialsRejected, "CredentialsRejected"),
        (AuthError::IdentityMismatch, "IdentityMismatch"),
        (AuthError::LoggedOut, "LoggedOut"),
        (AuthError::SessionExpired, "SessionExpired"),
        (AuthError::Storage, "Storage"),
    ] {
        assert_eq!(format!("{error:?}"), label);
    }
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_login_rate_limit_has_operation_deadline() {
    let config = fixture();
    runtime().block_on(async {
        let url = format!("{}_matrix/client/v3/login", config["url"].as_str().unwrap());
        let body = serde_json::json!({"type": "m.login.password", "identifier": {
            "type": "m.id.user", "user": "@alice:matrix-desktop.test"},
            "password": config["alice_password"].as_str().unwrap()})
        .to_string();
        let mut limited = false;
        for _ in 0..12 {
            let response = http(&config)
                .post(&url)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await
                .expect("fixture HTTP response");
            if response.status().as_u16() == 429 {
                limited = true;
                break;
            }
            assert_eq!(response.status().as_u16(), 200);
        }
        assert!(limited, "fixture must actually enforce login rate limit");
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            AccountSession::password_login_with_http(
                AccountId::new(16),
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                http(&config),
            ),
        )
        .await;
        assert!(
            matches!(outcome, Ok(Err(AuthError::Transport))),
            "SDK login retry/backoff must not escape whole-operation deadline"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_public_login_rejects_untrusted_ca() {
    let config = fixture();
    runtime().block_on(async {
        let result = AccountSession::password_login(
            AccountId::new(16),
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test",
            config["alice_password"].as_str().unwrap(),
        )
        .await;
        assert!(
            matches!(result, Err(AuthError::Transport | AuthError::LoginFailed)),
            "untrusted TLS must not authenticate"
        );
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_logout_timeout_retains_session_for_retry() {
    let config = fixture();
    runtime().block_on(async {
        let mut alice = login(&config, "alice", 16).await;
        let pid = config["server_pid"].as_u64().unwrap().to_string();
        struct ResumeServer(String);
        impl Drop for ResumeServer {
            fn drop(&mut self) {
                let _ = std::process::Command::new("/usr/bin/kill")
                    .args(["-CONT", &self.0])
                    .status();
            }
        }
        assert!(
            std::process::Command::new("/usr/bin/kill")
                .args(["-STOP", &pid])
                .status()
                .unwrap()
                .success()
        );
        let resume = ResumeServer(pid);
        let outcome =
            tokio::time::timeout(std::time::Duration::from_secs(15), alice.logout()).await;
        assert!(
            matches!(outcome, Ok(Err(AuthError::Transport))),
            "logout must have a bounded operation deadline"
        );
        assert!(
            alice.client.is_some(),
            "timeout must not claim local logout success"
        );
        drop(resume);
        // A timed-out request may still reach the server: revocation is uncertain
        // until retry, not proof that the old token remains remotely valid.
        alice.logout().await.expect("retry confirms revocation");
        assert!(matches!(alice.whoami().await, Err(AuthError::LoggedOut)));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_remotely_revoked_session_can_be_cleared() {
    let config = fixture();
    runtime().block_on(async {
        let mut alice = login(&config, "alice", 16).await;
        let token = alice.client.as_ref().unwrap().access_token().unwrap();
        let response = http(&config)
            .post(format!(
                "{}_matrix/client/v3/logout",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(&token)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .expect("external fixture revocation response");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(token_status(&config, &token).await, 401);
        assert!(matches!(
            alice.whoami().await,
            Err(AuthError::SessionExpired)
        ));
        alice
            .logout()
            .await
            .expect("already revoked token is safe to clear");
        assert!(matches!(alice.whoami().await, Err(AuthError::LoggedOut)));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_logout_revokes_only_this_session() {
    let config = fixture();
    runtime().block_on(async {
        let mut alice = login(&config, "alice", 16).await;
        let bob = login(&config, "bob", 17).await;
        let alice_token = alice
            .client
            .as_ref()
            .unwrap()
            .access_token()
            .expect("authenticated token");
        assert_eq!(token_status(&config, &alice_token).await, 200);
        alice.logout().await.expect("server logout accepted");
        assert_eq!(token_status(&config, &alice_token).await, 401);
        assert!(matches!(alice.whoami().await, Err(AuthError::LoggedOut)));
        assert_eq!(alice.account_id(), AccountId::new(16));
        assert!(
            bob.whoami()
                .await
                .expect("other account still active")
                .user_id()
                == "@bob:matrix-desktop.test"
        );
        // The independent provisioning session of the SAME user must survive too.
        assert_eq!(
            token_status(&config, config["alice_token"].as_str().unwrap()).await,
            200
        );
        assert_eq!(
            token_status(&config, config["bob_token"].as_str().unwrap()).await,
            200
        );
        alice
            .logout()
            .await
            .expect("repeat logout is locally idempotent");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_invalid_password_is_typed_rejection() {
    let config = fixture();
    runtime().block_on(async {
        let result = AccountSession::password_login_with_http(
            AccountId::new(16),
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test",
            "incorrect-disposable-fixture-password",
            http(&config),
        )
        .await;
        assert!(matches!(result, Err(AuthError::CredentialsRejected)));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_password_login_is_confirmed_by_server_whoami() {
    let config = fixture();
    runtime().block_on(async {
        let session = AccountSession::password_login_with_http(
            AccountId::new(16),
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test",
            config["alice_password"].as_str().unwrap(),
            http(&config),
        )
        .await
        .expect("password login accepted");
        assert_eq!(session.account_id(), AccountId::new(16));
        let identity = session.whoami().await.expect("server whoami accepted");
        assert!(identity.user_id() == "@alice:matrix-desktop.test");
        assert!(!identity.device_id().is_empty());
    });
}
