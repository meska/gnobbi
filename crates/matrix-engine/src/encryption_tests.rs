// real encrypted rooms, no hand-made ciphertext or crypto mocks.
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_e2ee_missing_keys_remains_visible_in_history() {
    runtime().block_on(async {
        let config = fixture();
        let (room, alice, peer) = encrypted_pair(&config).await;
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(23),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let sent = alice
            .send_text(&crate::sending::SendRequest {
                scope,
                transaction_id: matrix_domain::TransactionId::new("p023-missing"),
                body: "Secret before new device città".into(),
                attachment: None, mentions: Vec::new(),
            })
            .await
            .unwrap();
        // A new device after the send never received the old Megolm session.
        // No export/import, key forwarding, backup or verification bypass.
        drop(peer);
        let mut fresh = crate::room_sync::RoomSync::new(login(&config, "bob", 24).await);
        fresh.sliding_round().await.unwrap();
        let mut views = crate::ViewRequests::default();
        let tag = views
            .start(matrix_domain::RoomScope {
                account_id: AccountId::new(24),
                room_id: matrix_domain::RoomId::new(&room),
            })
            .unwrap();
        let page = fresh.history_page(tag, None).await.unwrap();
        let event = page
            .events
            .iter()
            .find(|event| event.event_id == sent)
            .expect("encrypted event must not disappear when keys are unavailable");
        assert_eq!(
            event.body,
            "[Messaggio cifrato · chiavi non disponibili o decifratura fallita]"
        );
        assert!(!event.outgoing);
        assert!(event.timestamp_ms > 0);
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_e2ee_server_ciphertext_peer_decrypts_and_app_reads_reply() {
    runtime().block_on(async {
        let config = fixture();
        let (room, mut alice, peer) = encrypted_pair(&config).await;
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(23),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let request = crate::sending::SendRequest {
            scope: scope.clone(),
            transaction_id: matrix_domain::TransactionId::new("p023-encrypted-retry"),
            body: "private città 🦀\nsecond line".into(),
            attachment: None, mentions: Vec::new(),
        };
        let sent = alice.send_text(&request).await.unwrap();
        let wire = room_request(
            &config,
            "bob",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", sent.as_str()),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(wire["type"], "m.room.encrypted");
        assert_eq!(wire["content"]["algorithm"], "m.megolm.v1.aes-sha2");
        assert!(
            wire["content"]["ciphertext"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
        assert!(wire["content"].get("body").is_none());
        assert!(!wire.to_string().contains(&request.body));
        peer.sync_once(matrix_sdk::config::SyncSettings::default())
            .await
            .unwrap();
        let peer_room = peer
            .get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap())
            .unwrap();
        let page = peer_room
            .messages(matrix_sdk::room::MessagesOptions::backward())
            .await
            .unwrap();
        let decrypted = page
            .chunk
            .iter()
            .find(|event| {
                event
                    .event_id()
                    .is_some_and(|id| id.as_str() == sent.as_str())
            })
            .unwrap();
        assert!(
            decrypted.encryption_info().is_some(),
            "peer must actually decrypt, not read plaintext"
        );
        assert_eq!(
            decrypted
                .raw()
                .get_field::<serde_json::Value>("content")
                .unwrap()
                .unwrap()["body"],
            request.body
        );
        let reply = peer_room
            .send(
                matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(
                    "peer reply città",
                ),
            )
            .await
            .unwrap()
            .response
            .event_id;
        alice.sliding_round().await.unwrap();
        let mut views = crate::ViewRequests::default();
        let tag = views.start(scope).unwrap();
        let page = alice.history_page(tag, None).await.unwrap();
        assert_eq!(
            page.events
                .iter()
                .filter(|event| event.event_id == sent)
                .count(),
            1
        );
        assert_eq!(
            page.events
                .iter()
                .find(|event| event.event_id.as_str() == reply.as_str())
                .unwrap()
                .body,
            "peer reply città"
        );
        let reply_wire = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{reply}"),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(reply_wire["type"], "m.room.encrypted");
        assert!(reply_wire["content"].get("body").is_none());
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_e2ee_unsupported_algorithm_does_not_send_plaintext() {
    runtime().block_on(async {
        let config = fixture();
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private", "initial_state":[
                {"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"org.example.unsupported"}}
            ]})).await["room_id"].as_str().unwrap().to_owned();
        let mut alice = crate::room_sync::RoomSync::new(login(&config, "alice", 23).await);
        alice.sliding_round().await.unwrap();
        let result = alice.send_text(&crate::sending::SendRequest {
            scope: matrix_domain::RoomScope { account_id: AccountId::new(23), room_id: matrix_domain::RoomId::new(&room) },
            transaction_id: matrix_domain::TransactionId::new("p023-unsupported"),
            body: "must never become plaintext".into(), attachment: None, mentions: Vec::new() }).await;
        assert!(matches!(result, Err(crate::sending::SendError::Encryption)), "unsupported room encryption must report an encryption error");
        let wire = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        assert!(!wire["chunk"].as_array().unwrap().iter().any(|event|
            event["type"] == "m.room.message" || event["type"] == "m.room.encrypted"));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_e2ee_redacted_ciphertext_shows_deleted_not_missing_keys() {
    runtime().block_on(async {
        let config = fixture();
        let (room, mut alice, _peer) = encrypted_pair(&config).await;
        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(23),
            room_id: matrix_domain::RoomId::new(&room),
        };
        let sent = alice
            .send_text(&crate::sending::SendRequest {
                scope: scope.clone(),
                transaction_id: matrix_domain::TransactionId::new("p023-redacted"),
                body: "erased disposable content".into(),
                attachment: None, mentions: Vec::new(),
            })
            .await
            .unwrap();
        room_request(
            &config,
            "alice",
            reqwest::Method::PUT,
            &format!("rooms/{room}/redact/{}/p023-redact", sent.as_str()),
            serde_json::json!({}),
        )
        .await;
        alice.sliding_round().await.unwrap();
        let mut views = crate::ViewRequests::default();
        let page = alice
            .history_page(views.start(scope).unwrap(), None)
            .await
            .unwrap();
        assert_eq!(
            page.events
                .iter()
                .find(|event| event.event_id == sent)
                .unwrap()
                .body,
            "[Messaggio eliminato]"
        );
    });
}

#[cfg(unix)]
#[test]
#[ignore = "prepares private GUI fixture; needs the external test harness (not in this repo)"]
fn disposable_e2ee_gui_store_and_history() {
    runtime().block_on(async {
        let config = fixture();
        let (room, alice, _peer) = encrypted_pair(&config).await;
        alice
            .send_text(&crate::sending::SendRequest {
                scope: matrix_domain::RoomScope {
                    account_id: AccountId::new(23),
                    room_id: matrix_domain::RoomId::new(&room),
                },
                transaction_id: matrix_domain::TransactionId::new("p023-gui-seed"),
                body: "old ciphertext unavailable to new GUI device".into(),
                attachment: None, mentions: Vec::new(),
            })
            .await
            .unwrap();
        let root = std::path::Path::new(config["ca"].as_str().unwrap())
            .parent()
            .unwrap();
        let session = AccountStore::create(root, AccountId::new(18))
            .unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                "disposable GUI passphrase only",
                http(&config),
            )
            .await
            .unwrap();
        assert_eq!(
            session.whoami().await.unwrap().user_id(),
            "@alice:matrix-desktop.test"
        );
    });
}

async fn encrypted_pair(
    config: &serde_json::Value,
) -> (String, crate::room_sync::RoomSync, matrix_sdk::Client) {
    let room = room_request(
        config,
        "alice",
        reqwest::Method::POST,
        "createRoom",
        serde_json::json!({"name":"encrypted disposable", "visibility":"private",
            "preset":"private_chat", "invite":["@bob:matrix-desktop.test"],
            "initial_state":[{"type":"m.room.encryption", "state_key":"",
                "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]}),
    )
    .await["room_id"]
        .as_str()
        .unwrap()
        .to_owned();
    room_request(
        config,
        "bob",
        reqwest::Method::POST,
        &format!("join/{room}"),
        serde_json::json!({}),
    )
    .await;
    let mut alice = crate::room_sync::RoomSync::new(login(config, "alice", 23).await);
    let peer_session = login(config, "bob", 24).await;
    let peer = peer_session.client.unwrap();
    peer.encryption().wait_for_e2ee_initialization_tasks().await;
    peer.sync_once(matrix_sdk::config::SyncSettings::default())
        .await
        .unwrap();
    alice.sliding_round().await.unwrap();
    // A subsequent sync flushes SDK device tracking/key-upload work.
    peer.sync_once(matrix_sdk::config::SyncSettings::default())
        .await
        .unwrap();
    alice.sliding_round().await.unwrap();
    (room, alice, peer)
}
