// Portable reaction retry contracts on real isolated TLS servers.
struct ReactionResponseLoss {
    child: std::process::Child,
    output: std::io::BufReader<std::process::ChildStdout>,
    url: String,
}
impl ReactionResponseLoss {
    fn start() -> Self {
        Self::start_for("reaction")
    }
    fn start_for(operation: &str) -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/support/reaction_response_loss.py"
            ))
            .arg(operation)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut url = String::new();
        output.read_line(&mut url).unwrap();
        assert!(url.starts_with("https://localhost:"));
        Self {
            child,
            output,
            url: url.trim().into(),
        }
    }
    fn dropped_event(&mut self) -> String {
        use std::io::BufRead;
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["dropped_event_id"]
            .as_str()
            .unwrap()
            .into()
    }
}
impl Drop for ReactionResponseLoss {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_reaction_recovers_committed_event_after_lost_tls_response() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let mut proxy = ReactionResponseLoss::start();
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"lost response"})).await["event_id"].as_str().unwrap().to_owned();
        let mut proxied = config.clone();
        proxied["url"] = proxy.url.clone().into();
        let mut sync = RoomSync::new(login(&proxied, "alice", 52).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(52), room_id: RoomId::new(&room) },
            transaction_id: TransactionId::new("p052-lost-response"),
            target: EventId::new(&target),
            operation: Operation::React { key: "👍".into() },
        };
        let first = sync.operate(&request).await;
        let committed = proxy.dropped_event();
        // The SDK send queue may resend the lost request itself (same transaction, same event).
        assert!(first == Err(OperationError::Transport) || first == Ok(EventId::new(&committed)));
        let event = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{committed}"), serde_json::json!({})).await;
        assert_eq!(event["type"], "m.reaction");
        assert_eq!(event["content"]["m.relates_to"]["event_id"], target);
        let recovered = sync.operate(&request).await.expect("retry recovers exact transaction evidence");
        assert_eq!(recovered.as_str(), committed);
        assert_eq!(sync.operate(&request).await.unwrap(), recovered);
        let messages = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        assert_eq!(messages["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.reaction").count(), 1);
        let removal = sync.operate(&OperationRequest {
            transaction_id: TransactionId::new("p052-remove-recovered"),
            target: recovered.clone(), operation: Operation::Redact, ..request
        }).await.unwrap();
        let removed = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{committed}"), serde_json::json!({})).await;
        assert!(removed["content"].as_object().unwrap().is_empty());
        assert_eq!(removed["unsigned"]["redacted_because"]["event_id"], removal.as_str());
        println!("readback {}", serde_json::json!({"committed":committed,"recovered":recovered.as_str(),"removal":removal.as_str(),"target":target,"reaction_count":1}));
    });
}
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_reaction_rejects_changed_uncertain_request() {
    use crate::{
        operations::{Operation, OperationError, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let proxy = ReactionResponseLoss::start();
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let other = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let mut targets = Vec::new();
        for (room, tx) in [(&room, "one"), (&room, "two"), (&other, "three")] {
            targets.push(room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/{tx}"),
                serde_json::json!({"msgtype":"m.text","body":"immutable retry"})).await["event_id"].as_str().unwrap().to_owned());
        }
        let mut proxied = config.clone();
        proxied["url"] = proxy.url.clone().into();
        let mut sync = RoomSync::new(login(&proxied, "alice", 52).await);
        sync.next_batch().await.unwrap();
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(52), room_id: RoomId::new(&room) },
            transaction_id: TransactionId::new("p052-immutable-uncertain"),
            target: EventId::new(&targets[0]), operation: Operation::React { key: "👍".into() },
        };
        // The SDK send queue may resend the lost request itself (same transaction, same event).
        let first = sync.operate(&request).await;
        assert!(first.is_ok() || first == Err(OperationError::Transport));
        let variants = [
            OperationRequest { operation: Operation::React { key: "👎".into() }, ..request.clone() },
            OperationRequest { target: EventId::new(&targets[1]), ..request.clone() },
            OperationRequest { scope: RoomScope { account_id: AccountId::new(52), room_id: RoomId::new(&other) }, target: EventId::new(&targets[2]), ..request.clone() },
            OperationRequest { operation: Operation::Redact, ..request.clone() },
        ];
        for changed in variants {
            assert_eq!(sync.operate(&changed).await, Err(OperationError::InvalidInput), "uncertain transaction must keep exact request");
        }
        let mut foreign = request.clone();
        foreign.scope.account_id = AccountId::new(999);
        assert_eq!(sync.operate(&foreign).await, Err(OperationError::WrongScope));
        let recovered = sync.operate(&request).await.unwrap();
        for changed in [OperationRequest { operation: Operation::React { key: "👎".into() }, ..request.clone() }, OperationRequest { target: EventId::new(&targets[1]), ..request.clone() }] {
            assert_eq!(sync.operate(&changed).await, Err(OperationError::InvalidInput));
        }
        for (id, expected) in [(&room, 1), (&other, 0)] {
            let events = room_request(&config, "alice", reqwest::Method::GET,
                &format!("rooms/{id}/messages?dir=b&limit=100"), serde_json::json!({})).await;
            assert_eq!(events["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.reaction").count(), expected);
        }
        println!("immutable {}", serde_json::json!({"event_id":recovered.as_str(),"room":room,"foreign_room":other,"reaction_counts":[1,0]}));
    });
}
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_reaction_does_not_adopt_unrelated_annotation() {
    use crate::{
        operations::{Operation, OperationRequest},
        room_sync::RoomSync,
    };
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private"})).await["room_id"].as_str().unwrap().to_owned();
        let target = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/target"),
            serde_json::json!({"msgtype":"m.text","body":"unrelated annotation"})).await["event_id"].as_str().unwrap().to_owned();
        let session = login(&config, "alice", 52).await;
        let client = session.client.as_ref().unwrap().clone();
        let mut sync = RoomSync::new(session);
        sync.next_batch().await.unwrap();
        let existing = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap()
            .send_raw("m.reaction", serde_json::json!({"m.relates_to":{"rel_type":"m.annotation","event_id":target,"key":"👍"}}))
            .with_transaction_id("p052-unrelated-existing".into()).await.unwrap().response.event_id;
        let request = OperationRequest {
            scope: RoomScope { account_id: AccountId::new(52), room_id: RoomId::new(&room) },
            transaction_id: TransactionId::new("p052-new-intent"),
            target: EventId::new(&target), operation: Operation::React { key: "👍".into() },
        };
        assert!(sync.operate(&request).await.is_err(), "existing annotation is not proof of this transaction");
        let messages = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        let reactions: Vec<_> = messages["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.reaction").collect();
        assert_eq!(reactions.len(), 1, "must not create a second reaction");
        assert_eq!(reactions[0]["event_id"], existing.as_str());
        println!("unrelated {}", serde_json::json!({"event_id":existing.as_str(),"reaction_count":reactions.len()}));
    });
}
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_operation_reaction_worker_reuses_confirmed_identity() {
    use crate::operations::{Operation, OperationCommand, OperationRequest, OperationResult};
    use matrix_domain::{EventId, RoomId, RoomScope, TransactionId};
    let config = fixture();
    let (mut worker, _events, room) = ready_worker(&config);
    let request = runtime().block_on(async {
        let target = room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p052-worker-target"),
            serde_json::json!({"msgtype":"m.text","body":"worker"}),
        )
        .await;
        OperationRequest {
            scope: RoomScope {
                account_id: AccountId::new(22),
                room_id: RoomId::new(&room),
            },
            transaction_id: TransactionId::new("p052-worker-react"),
            target: EventId::new(target["event_id"].as_str().unwrap()),
            operation: Operation::React { key: "👍".into() },
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
        let event = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", ids[0].as_str()),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            event["content"]["m.relates_to"]["event_id"],
            request.target.as_str()
        );
        println!(
            "worker {}",
            serde_json::json!({"event_id":ids[0].as_str(),"retry_event_id":ids[1].as_str()})
        );
    });
    worker.shutdown().unwrap();
}
