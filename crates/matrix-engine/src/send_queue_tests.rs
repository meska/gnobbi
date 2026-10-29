// Text sends through the SDK send queue: retry, cancel, edit before send, restart. Included from
// auth_tests.rs; needs a disposable homeserver (Synapse or Tuwunel).

fn queued(room: &str, id: &str, body: &str) -> crate::sending::SendRequest {
    crate::sending::SendRequest {
        scope: matrix_domain::RoomScope {
            account_id: AccountId::new(22),
            room_id: matrix_domain::RoomId::new(room),
        },
        transaction_id: matrix_domain::TransactionId::new(id),
        body: body.into(),
        attachment: None,
        mentions: Vec::new(),
    }
}


/// A loopback relay in front of the homeserver that can go "offline" the way a lost network does:
/// live connections are cut and new ones refused at once (a stopped server would only hang).
struct Relay {
    port: u16,
    upstream: u16,
    state: std::sync::Arc<std::sync::Mutex<RelayState>>,
}
#[derive(Default)]
struct RelayState {
    online: bool,
    open: Vec<std::net::TcpStream>,
}
impl Relay {
    fn start(config: &serde_json::Value) -> Self {
        let upstream: u16 = config["url"].as_str().unwrap().rsplit(':').next().unwrap().trim_end_matches('/').parse().unwrap();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let relay = Self { port, upstream, state: Default::default() };
        relay.serve(listener);
        relay
    }
    fn serve(&self, listener: std::net::TcpListener) {
        let (state, upstream) = (self.state.clone(), self.upstream);
        state.lock().unwrap().online = true;
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            while state.lock().unwrap().online {
                let Ok((client, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let _ = client.set_nonblocking(false);
                let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", upstream))
                    .or_else(|_| std::net::TcpStream::connect(("::1", upstream))) else { continue };
                let mut open = state.lock().unwrap();
                open.open.extend(client.try_clone().into_iter().chain(server.try_clone()));
                drop(open);
                for (mut from, mut to) in [(client.try_clone().unwrap(), server.try_clone().unwrap()), (server, client)] {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                        let _ = to.shutdown(std::net::Shutdown::Both);
                    });
                }
            }
            // `listener` drops here: the port refuses connections.
        });
    }
    fn url(&self) -> String {
        format!("https://localhost:{}/", self.port)
    }
    /// The config a client behind the relay logs in with.
    fn config(&self, config: &serde_json::Value) -> serde_json::Value {
        let mut behind = config.clone();
        behind["url"] = self.url().into();
        behind
    }
    fn go_offline(&self) {
        let mut state = self.state.lock().unwrap();
        state.online = false;
        for stream in state.open.drain(..) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(100));
    }
    fn go_online(&self) {
        self.serve(std::net::TcpListener::bind(("127.0.0.1", self.port)).unwrap());
    }
}

/// A worker for alice (account 22) whose homeserver is reached through `relay`, and a fresh room.
fn relayed_worker(config: &serde_json::Value, relay: &Relay) -> (RoomListWorker, crate::Receiver<RoomListEvent>, String) {
    let room = runtime().block_on(async {
        room_request(config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"relayed","visibility":"private"})).await["room_id"]
            .as_str().unwrap().to_owned()
    });
    let behind = relay.config(config);
    let (worker, mut events) =
        RoomListWorker::spawn(move || async move { Ok(login(&behind, "alice", 22).await) }).unwrap();
    runtime().block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            while !matches!(events.recv().await, Some(RoomListEvent::Status(RoomSyncStatus::Ready))) {}
        }).await.expect("worker ready");
    });
    (worker, events, room)
}

/// Bodies of the room's messages, oldest first.
async fn room_bodies(config: &serde_json::Value, room: &str) -> Vec<String> {
    let messages = room_request(
        config,
        "alice",
        reqwest::Method::GET,
        &format!("rooms/{room}/messages?dir=f&limit=100"),
        serde_json::json!({}),
    )
    .await;
    messages["chunk"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "m.room.message")
        .map(|e| e["content"]["body"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_send_queue_retry_cancel_and_edit_before_send() {
    use crate::sending::SendNotice;
    use matrix_domain::TransactionId;
    let config = fixture();
    let relay = Relay::start(&config);
    let (mut worker, _events, room) = relayed_worker(&config, &relay);
    let (mut sends, mut results) = worker.take_send_route().unwrap();
    let slot = worker.send_control();
    let control = move || slot.lock().unwrap().clone().expect("send control while the worker runs");
    runtime().block_on(async {
        relay.go_offline();
        assert!(sends.send(queued(&room, "q-a", "first")).await.is_ok());
        // The SDK tries a few times, then the queue reports it and switches itself off.
        let failed = next_result(&mut results, 60).await;
        assert!(failed.transaction_id.as_str() == "q-a" && failed.result.is_err());
        // B and C wait behind it, nothing of theirs is on the wire yet.
        assert!(sends.send(queued(&room, "q-b", "withdrawn")).await.is_ok());
        assert!(sends.send(queued(&room, "q-c", "draft")).await.is_ok());
        for _ in 0..50 {
            if control().cancel(&TransactionId::new("q-b")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(matches!(next_notice(&mut results, "q-b", 5).await, SendNotice::Cancelled));
        for _ in 0..50 {
            if control().edit(&TransactionId::new("q-c"), "edited draft", &[]) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        match next_notice(&mut results, "q-c", 5).await {
            SendNotice::Edited { body } => assert_eq!(body, "edited draft"),
            _ => panic!("edit before send is reported with the new text"),
        }
        // A cancelled message is gone from the queue; an unknown one cannot be retried.
        assert!(!control().cancel(&TransactionId::new("q-b")));
        assert!(!control().retry(&TransactionId::new("q-unknown")));
        relay.go_online();
        // Retry by hand: the failed message goes out, then the edited one.
        assert!(control().retry(&TransactionId::new("q-a")));
        let sent = sent_all(&mut results, &["q-a", "q-c"], 40).await;
        assert!(sent["q-a"] != sent["q-c"]);
        // Once accepted by the server a message can no longer be withdrawn.
        assert!(!control().cancel(&TransactionId::new("q-a")));
        assert_eq!(room_bodies(&config, &room).await, ["first", "edited draft"]);
    });
    worker.shutdown().unwrap();
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_send_queue_message_left_in_the_queue_restarts_at_relaunch() {
    use crate::sending::SendNotice;
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let phrase = "disposable send queue passphrase";
    let room = runtime().block_on(async {
        room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"relaunch","visibility":"private"})).await["room_id"]
            .as_str().unwrap().to_owned()
    });
    let start = |open: bool| {
        let session = runtime().block_on(async {
            if open {
                AccountStore::open(root, AccountId::new(22)).unwrap()
                    .restore_with_http(phrase, http(&config)).await.unwrap()
            } else {
                AccountStore::create(root, AccountId::new(22)).unwrap()
                    .password_login_with_http(
                        HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                        "@alice:matrix-desktop.test",
                        config["alice_password"].as_str().unwrap(),
                        phrase,
                        http(&config),
                    ).await.unwrap()
            }
        });
        let (worker, mut events) = RoomListWorker::spawn(move || async move { Ok(session) }).unwrap();
        runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(15), async {
                while !matches!(events.recv().await, Some(RoomListEvent::Status(RoomSyncStatus::Ready))) {}
            }).await.expect("worker ready");
        });
        (worker, events)
    };
    let (mut worker, _events) = start(false);
    let (mut sends, _results) = worker.take_send_route().unwrap();
    {
        let _paused = PausedServer::new(&config);
        runtime().block_on(async {
            assert!(sends.send(queued(&room, "q-left", "left behind")).await.is_ok());
            tokio::time::sleep(Duration::from_millis(500)).await;
        });
        // Leaving with the message unconfirmed is reported, but the message stays in the store.
        assert!(worker.shutdown().is_err());
    }
    let (mut worker, _events) = start(true);
    let (_sends, mut results) = worker.take_send_route().unwrap();
    runtime().block_on(async {
        // Shown again as it was, with the queue's own transaction ID (the window's is gone).
        let (id, body) = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let crate::sending::SendEvent::Notice { transaction_id, notice: SendNotice::Restored { body, failed }, .. } =
                    results.recv().await.expect("send route open")
                {
                    assert!(!failed);
                    return (transaction_id, body);
                }
            }
        }).await.expect("restored message announced");
        assert_eq!(body, "left behind");
        let sent = sent_all(&mut results, &[id.as_str()], 30).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(room_bodies(&config, &room).await, ["left behind"]);
    });
    worker.shutdown().unwrap();
}
