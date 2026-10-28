// independent real accounts, not two aliases of the same Matrix user.
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_logged_out_capability_cannot_be_selected() {
    runtime().block_on(async {
        let config = fixture();
        let mut alice = login(&config, "alice", 28).await;
        alice.logout().await.unwrap();
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(alice).is_err(), "cannot register a logged-out capability");
        assert!(accounts.switch(AccountId::new(28)).is_err());
        assert!(accounts.active().is_none());
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_join_logout_one_worker_does_not_stop_the_other() {
    use crate::sending::{SendError, SendRequest};
    use matrix_domain::{RoomId, RoomScope, TransactionId};
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let phrase = "worker isolated fixture passphrase";
    let room = runtime().block_on(async {
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private", "invite":["@bob:matrix-desktop.test"]}))
            .await["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        room
    });
    let start = |name: &'static str, id: u64| {
        let config = config.clone();
        crate::room_sync::RoomListWorker::spawn(move || async move {
            let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
            AccountStore::create(root, AccountId::new(id)).unwrap().password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                &format!("@{name}:matrix-desktop.test"),
                config[format!("{name}_password")].as_str().unwrap(), phrase, http(&config)).await
        }).unwrap()
    };
    let (mut alice, mut a_rooms) = start("alice", 28);
    let (mut bob, mut b_rooms) = start("bob", 29);
    let (mut a_sends, mut a_results) = alice.take_send_route().unwrap();
    let (mut b_sends, mut b_results) = bob.take_send_route().unwrap();
    let request = |id, transaction: &str, body: &str| SendRequest {
        scope: RoomScope { account_id: AccountId::new(id), room_id: RoomId::new(&room) },
        transaction_id: TransactionId::new(transaction), body: body.into(),
 attachment: None,
 };
    runtime().block_on(async {
        for (events, id) in [(&mut a_rooms, 28), (&mut b_rooms, 29)] {
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    match events.recv().await {
                        Some(crate::RoomListEvent::Status(crate::RoomSyncStatus::Ready)) => break,
                        Some(crate::RoomListEvent::Diff(crate::RoomListDiff::Upsert(summary))) => {
                            assert!(summary.scope.account_id == AccountId::new(id));
                        }
                        Some(_) => (),
                        None => panic!("account route closed before Ready"),
                    }
                }
            }).await.unwrap();
        }
        assert!(a_sends.send(request(28, "p028-same-transaction", "Alice isolated task")).await.is_ok());
        assert!(b_sends.send(request(29, "p028-same-transaction", "Bob isolated task")).await.is_ok());
        let a = tokio::time::timeout(std::time::Duration::from_secs(10), a_results.recv()).await.unwrap().unwrap();
        let b = tokio::time::timeout(std::time::Duration::from_secs(10), b_results.recv()).await.unwrap().unwrap();
        assert!(a.scope.account_id == AccountId::new(28));
        assert!(b.scope.account_id == AccountId::new(29));
        let a_id = a.result.unwrap();
        let b_id = b.result.unwrap();
        assert!(a_id != b_id, "same transaction in different accounts cannot deduplicate each other");
        for (event_id, sender, text) in [(a_id, "@alice:matrix-desktop.test", "Alice isolated task"), (b_id, "@bob:matrix-desktop.test", "Bob isolated task")] {
            let wire = room_request(&config, "bob", reqwest::Method::GET,
                &format!("rooms/{room}/event/{}", event_id.as_str()), serde_json::json!({})).await;
            assert!(wire["sender"] == sender);
            assert!(wire["content"]["body"] == text);
        }
    });
    alice.shutdown().unwrap();
    // The store lock is released only after join. Revoke that exact device while
    // Bob's independently owned thread/sync/send routes remain live.
    runtime().block_on(async {
        let session = AccountStore::open(root, AccountId::new(28)).unwrap().restore_with_http(phrase, http(&config)).await.unwrap();
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(session).is_ok());
        accounts.switch(AccountId::new(28)).unwrap();
        accounts.logout(AccountId::new(28)).await.unwrap();
        assert!(!root.join("account-28/session.sealed").exists());
        assert!(root.join("account-29/session.sealed").exists());
        assert!(a_sends.send(request(28, "p028-after-join", "must not send")).await.is_err());
        assert!(b_sends.send(request(28, "p028-wrong-route", "must not cross")).await.is_ok());
        let rejected = tokio::time::timeout(std::time::Duration::from_secs(10), b_results.recv()).await.unwrap().unwrap();
        assert_eq!(rejected.result, Err(SendError::WrongScope));
        assert!(b_sends.send(request(29, "p028-still-live", "Bob survives Alice logout")).await.is_ok());
        let sent = tokio::time::timeout(std::time::Duration::from_secs(10), b_results.recv()).await.unwrap().unwrap();
        assert!(sent.scope.account_id == AccountId::new(29));
        let wire = room_request(&config, "bob", reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", sent.result.unwrap().as_str()), serde_json::json!({})).await;
        assert!(wire["sender"] == "@bob:matrix-desktop.test");
        assert!(wire["content"]["body"] == "Bob survives Alice logout");
    });
    bob.shutdown().unwrap();
    runtime().block_on(async {
        let bob = AccountStore::open(root, AccountId::new(29)).unwrap().restore_with_http(phrase, http(&config)).await.unwrap();
        assert!(bob.whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_collision_cannot_replace_or_revoke_the_original() {
    runtime().block_on(async {
        let config = fixture();
        let alice = login(&config, "alice", 28).await;
        let bob = login(&config, "bob", 28).await;
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(alice).is_ok());
        accounts.switch(AccountId::new(28)).unwrap();
        let returned = match accounts.insert(bob) {
            Err(session) => session,
            Ok(()) => panic!("duplicate ID must preserve the original capability"),
        };
        assert!(returned.whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@alice:matrix-desktop.test");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_inactive_logout_leaves_active_selection_unchanged() {
    runtime().block_on(async {
        let config = fixture();
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(login(&config, "alice", 28).await).is_ok());
        assert!(accounts.insert(login(&config, "bob", 29).await).is_ok());
        accounts.switch(AccountId::new(29)).unwrap();
        accounts.logout(AccountId::new(28)).await.unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
        assert_eq!(accounts.logout(AccountId::new(28)).await, Err(AuthError::LoggedOut));
        assert!(accounts.active().unwrap().account_id() == AccountId::new(29));
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_failed_logout_retains_both_sessions_for_retry() {
    runtime().block_on(async {
        let config = fixture();
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(login(&config, "alice", 28).await).is_ok());
        assert!(accounts.insert(login(&config, "bob", 29).await).is_ok());
        accounts.switch(AccountId::new(28)).unwrap();
        {
            let _paused = PausedServer::new(&config);
            assert_eq!(accounts.logout(AccountId::new(28)).await, Err(AuthError::Transport));
            assert!(accounts.active().unwrap().account_id() == AccountId::new(28));
        }
        accounts.switch(AccountId::new(29)).unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
        accounts.logout(AccountId::new(28)).await.unwrap();
        assert!(accounts.active().unwrap().account_id() == AccountId::new(29));
        assert!(accounts.switch(AccountId::new(28)).is_err());
    });
}

#[test]
#[cfg(unix)]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_logout_preserves_other_store_and_crypto_identity() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let alice_key = "alice isolated fixture passphrase";
    let bob_key = "bob isolated fixture passphrase";
    let room = runtime().block_on(async {
        room_request(&config, "bob", reqwest::Method::POST, "createRoom",
            serde_json::json!({"visibility":"private", "initial_state":[
                {"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}
            ]})).await["room_id"].as_str().unwrap().to_owned()
    });
    let (bob_signing, bob_device, event_id) = runtime().block_on(async {
        let mut accounts = crate::accounts::AccountSessions::default();
        for (name, id, key) in [("alice", 28, alice_key), ("bob", 29, bob_key)] {
            let session = AccountStore::create(root, AccountId::new(id)).unwrap()
                .password_login_with_http(
                    HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                    &format!("@{name}:matrix-desktop.test"),
                    config[format!("{name}_password")].as_str().unwrap(), key, http(&config))
                .await.unwrap();
            assert!(accounts.insert(session).is_ok());
        }
        accounts.switch(AccountId::new(29)).unwrap();
        let bob = accounts.active().unwrap();
        let client = bob.client.as_ref().unwrap();
        client.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        let bob_room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
        let event = bob_room.send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("Bob durable secret città")).await.unwrap().response.event_id.to_string();
        let wire = room_request(&config, "bob", reqwest::Method::GET,
            &format!("rooms/{room}/event/{event}"), serde_json::json!({})).await;
        assert!(wire["type"] == "m.room.encrypted");
        assert!(wire["content"].get("body").is_none());
        let signing = bob.client.as_ref().unwrap().encryption().get_own_device().await.unwrap().unwrap().ed25519_key().unwrap().to_base64();
        let device = bob.whoami().await.unwrap().device_id().to_owned();
        let sealed = std::fs::read(root.join("account-29/session.sealed")).unwrap();
        accounts.switch(AccountId::new(28)).unwrap();
        let alice = accounts.active().unwrap();
        let alice_signing = alice.client.as_ref().unwrap().encryption().get_own_device().await.unwrap().unwrap().ed25519_key().unwrap().to_base64();
        assert!(alice_signing != signing, "accounts must not share crypto identities");
        let token = alice.client.as_ref().unwrap().access_token().unwrap();
        accounts.logout(AccountId::new(28)).await.unwrap();
        assert!(accounts.active().is_none(), "logout must not implicitly expose another account");
        assert!(accounts.switch(AccountId::new(28)).is_err());
        assert!(!root.join("account-28/session.sealed").exists());
        assert!(sealed == std::fs::read(root.join("account-29/session.sealed")).unwrap());
        assert_eq!(token_status(&config, &token).await, 401);
        accounts.switch(AccountId::new(29)).unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
        (signing, device, event)
    });
    // All clients, pools and tasks destroyed before opening the remaining account.
    runtime().block_on(async {
        assert!(AccountStore::open(root, AccountId::new(29)).unwrap().restore_with_http(alice_key, http(&config)).await.is_err());
        let bob = AccountStore::open(root, AccountId::new(29)).unwrap().restore_with_http(bob_key, http(&config)).await.unwrap();
        assert!(bob.whoami().await.unwrap().device_id() == bob_device);
        let signing = bob.client.as_ref().unwrap().encryption().get_own_device().await.unwrap().unwrap().ed25519_key().unwrap().to_base64();
        assert!(signing == bob_signing, "other account crypto identity survives logout and runtime restart");
        let mut sync = crate::room_sync::RoomSync::new(bob);
        sync.next_batch().await.unwrap();
        let mut views = crate::ViewRequests::default();
        let tag = views.start(matrix_domain::RoomScope {
            account_id: AccountId::new(29), room_id: matrix_domain::RoomId::new(&room),
        }).unwrap();
        let page = sync.history_page(tag, None).await.unwrap();
        assert!(page.events.iter().any(|row| row.event_id.as_str() == event_id && row.body == "Bob durable secret città"),
            "remaining account must decrypt its old ciphertext with retained room keys");
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_accounts_switch_selects_only_the_requested_identity() {
    runtime().block_on(async {
        let config = fixture();
        let alice = login(&config, "alice", 28).await;
        let bob = login(&config, "bob", 29).await;
        let mut accounts = crate::accounts::AccountSessions::default();
        assert!(accounts.insert(alice).is_ok());
        assert!(accounts.insert(bob).is_ok());
        assert!(accounts.active().is_none(), "insertion must not implicitly select an account");
        accounts.switch(AccountId::new(28)).unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@alice:matrix-desktop.test");
        accounts.switch(AccountId::new(29)).unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@bob:matrix-desktop.test");
        assert!(accounts.switch(AccountId::new(30)).is_err());
        assert!(accounts.active().unwrap().account_id() == AccountId::new(29));
        accounts.switch(AccountId::new(28)).unwrap();
        assert!(accounts.active().unwrap().whoami().await.unwrap().user_id() == "@alice:matrix-desktop.test");
    });
}
