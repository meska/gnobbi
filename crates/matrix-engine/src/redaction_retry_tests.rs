// Portable redaction retry contracts on real isolated TLS servers.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_redaction_recovers_committed_event_after_lost_tls_response() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let mut proxy = ReactionResponseLoss::start_for("redaction");
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let other = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let other_target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{other}/send/m.room.message/other"),
            serde_json::json!({"msgtype":"m.text","body":"untouched"})).await["event_id"].as_str().unwrap().to_owned();
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"lost response"})).await["event_id"].as_str().unwrap().to_owned();
        let mut proxied = config.clone();
        proxied["url"] = proxy.url.clone().into();
        let session = login(&proxied, "alice", 53).await;
        let client = session.client.as_ref().unwrap().clone();
        let mut sync = RoomSync::new(session);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(53), room_id: RoomId::new(&room) },
            transaction_id: TransactionId::new("p053-lost-response"),
            target: EventId::new(&target), operation: Operation::Redact,
        };
        assert_eq!(sync.operate(&request).await, Err(OperationError::Transport));
        let committed = proxy.dropped_event();
        let changed = [
            OperationRequest { target: EventId::new(&other_target), ..request.clone() },
            OperationRequest { scope: RoomScope { account_id: AccountId::new(53), room_id: RoomId::new(&other) }, target: EventId::new(&other_target), ..request.clone() },
            OperationRequest { operation: Operation::React { key: "👍".into() }, ..request.clone() },
        ];
        for variant in &changed {
            assert_eq!(sync.operate(variant).await, Err(OperationError::InvalidInput));
        }
        let mut foreign = request.clone();
        foreign.scope.account_id = AccountId::new(999);
        assert_eq!(sync.operate(&foreign).await, Err(OperationError::WrongScope));
        let event = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{committed}"), serde_json::json!({})).await;
        assert_eq!(event["type"], "m.room.redaction");
        let own = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap()
            .event(&matrix_sdk::ruma::EventId::parse(&committed).unwrap(), None).await.unwrap();
        let own: serde_json::Value = serde_json::from_str(own.raw().json().get()).unwrap();
        let proven = own["unsigned"]["transaction_id"].as_str() == Some(request.transaction_id.as_str());
        for _ in 0..3 {
            let result = sync.operate(&request).await;
            if proven {
                assert_eq!(result, Ok(EventId::new(&committed)));
            } else {
                assert_eq!(result, Err(OperationError::Transport), "deleted target is not transaction confirmation");
            }
        }
        for variant in &changed {
            assert_eq!(sync.operate(variant).await, Err(OperationError::InvalidInput));
        }
        let untouched = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{other}/event/{other_target}"), serde_json::json!({})).await;
        assert_eq!(untouched["content"]["body"], "untouched");
        let foreign_events = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{other}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        assert_eq!(foreign_events["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.room.redaction").count(), 0);
        let messages = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        let redactions: Vec<_> = messages["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.room.redaction").collect();
        assert_eq!(redactions.len(), 1);
        assert_eq!(redactions[0]["event_id"], committed);
        let removed = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{target}"), serde_json::json!({})).await;
        assert!(removed["content"].as_object().unwrap().is_empty());
        assert_eq!(removed["event_id"], target);
        assert_eq!(removed["unsigned"]["redacted_because"]["event_id"], committed);
        println!("readback {}", serde_json::json!({"committed":committed,"confirmed":proven,"target":target,"redaction_ids":[redactions[0]["event_id"]]}));
    });
}

#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_redaction_worker_reuses_confirmed_identity() {
    use crate::operations::{Operation, OperationCommand, OperationRequest, OperationResult};
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let request = runtime().block_on(async {
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p053-worker-target"),
            serde_json::json!({"msgtype":"m.text","body":"worker"}),
        )
        .await;
        OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(22),
                room_id: RoomId::new(&room),
            },
            transaction_id: TransactionId::new("p053-worker-react"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::Redact,
        }
    });
    let (mut commands, mut results) = worker.take_operation_route().unwrap();
    runtime().block_on(async {
        let mut ids = Vec::new();
        for _ in 0..2 {
            assert!(
                commands
                    .send(OperationCommand::Apply(request.clone()))
                    .await
                    .is_ok()
            );
            let OperationResult::Applied {
                request: actual,
                result,
            } = tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
                .await
                .unwrap()
                .unwrap()
            else {
                panic!("expected applied")
            };
            assert!(actual == request);
            ids.push(result.unwrap());
        }
        assert_eq!(ids[0], ids[1]);
        let mut changed = request.clone();
        changed.target = EventId::new("$unrelated");
        assert!(
            commands
                .send(OperationCommand::Apply(changed))
                .await
                .is_ok()
        );
        let OperationResult::Applied { result, .. } =
            tokio::time::timeout(std::time::Duration::from_secs(8), results.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("expected applied")
        };
        assert_eq!(result, Err(crate::operations::OperationError::InvalidInput));
        let messages = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"),
            serde_json::json!({}),
        )
        .await;
        let redactions: Vec<_> = messages["chunk"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["type"] == "m.room.redaction")
            .collect();
        assert_eq!(redactions.len(), 1);
        assert_eq!(redactions[0]["event_id"], ids[0].as_str());
        let original = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", request.target.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert!(original["content"].as_object().unwrap().is_empty());
        assert_eq!(
            original["unsigned"]["redacted_because"]["event_id"],
            ids[0].as_str()
        );
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", ids[0].as_str()),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(event["content"]["redacts"], request.target.as_str());
        println!(
            "worker {}",
            serde_json::json!({"event_id":ids[0].as_str(),"retry_event_id":ids[1].as_str()})
        );
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_redaction_does_not_adopt_unrelated_deletion_after_uncommitted_write() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let proxy = ReactionResponseLoss::start_for("redaction-before");
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"not committed"})).await["event_id"].as_str().unwrap().to_owned();
        let mut proxied = config.clone();
        proxied["url"] = proxy.url.clone().into();
        let session = login(&proxied, "alice", 53).await;
        let client = session.client.as_ref().unwrap().clone();
        let mut sync = RoomSync::new(session);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(53), room_id: RoomId::new(&room) },
            transaction_id: TransactionId::new("p053-uncommitted"),
            target: EventId::new(&target), operation: Operation::Redact,
        };
        for _ in 0..2 {
            assert_eq!(sync.operate(&request).await, Err(OperationError::Transport));
        }
        let before = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{target}"), serde_json::json!({})).await;
        assert_eq!(before["content"]["body"], "not committed");
        // Same sender and device, but a different transaction is still unrelated.
        let unrelated = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap()
            .redact(&matrix_sdk::ruma::EventId::parse(&target).unwrap(), None,
                Some("p053-unrelated".into())).await.unwrap().event_id;
        for _ in 0..2 {
            assert_eq!(sync.operate(&request).await, Err(OperationError::Transport));
        }
        let messages = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        let redactions: Vec<_> = messages["chunk"].as_array().unwrap().iter()
            .filter(|event| event["type"] == "m.room.redaction").collect();
        assert_eq!(redactions.len(), 1);
        assert_eq!(redactions[0]["event_id"], unrelated.as_str());
        println!("unrelated {}", serde_json::json!({"target":target,"unrelated":unrelated.as_str(),"redaction_ids":[redactions[0]["event_id"]],"confirmed":false,"uncommitted_retries":2}));
    });
}
