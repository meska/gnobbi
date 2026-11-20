// Real test-only key material stays in memory; never include it in assertions.
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_rejects_empty_or_oversized_secret_before_import() {
    runtime().block_on(async {
        let config = fixture();
        let mut session = login(&config, "alice", 25).await;
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(25), room_id: matrix_domain::RoomId::new("!input:matrix-desktop.test") };
        for invalid in [String::new(), "a".repeat(1025)] {
            assert_eq!(session.recover_room_keys(&invalid, &scope).await, Err(crate::recovery::RecoveryError::InvalidSecret));
        }
    });
}
#[test]
fn recovery_secret_zeroizes_on_drop() {
    fn require_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    require_zeroize_on_drop::<crate::recovery::RecoverySecret>();
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_wrong_valid_key_preserves_local_keys_and_remote_backup() {
    runtime().block_on(async {
        let config = fixture();
        let (mut session, room, event) = recovery_seed(&config).await;
        let secret = session.enable_new_recovery().await.expect("setup");
        session.upload_key_backup().await.unwrap();
        let before = recovery_backup_wire(&config).await;
        let other = login(&config, "bob", 27).await;
        let other_client = other.client.as_ref().unwrap();
        other_client.encryption().wait_for_e2ee_initialization_tasks().await;
        let other_store = other_client.encryption().secret_storage().create_secret_store().await.unwrap();
        let wrong_key = other_store.secret_storage_key(); // Valid format, different key/MAC.
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(25), room_id: matrix_domain::RoomId::new(&room) };
        let result = session.recover_room_keys(&wrong_key, &scope).await;
        assert_eq!(result, Err(crate::recovery::RecoveryError::Failed));
        recovery_assert_message(session.client.as_ref().unwrap(), &room, &event, true).await;
        assert!(recovery_backup_wire(&config).await == before, "failure must not delete or mutate server backup");
        session.recover_room_keys(secret.expose(), &scope).await.expect("original key remains usable");
        recovery_assert_message(session.client.as_ref().unwrap(), &room, &event, true).await;
    });
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_transport_failure_preserves_keys_and_retry() {
    runtime().block_on(async {
        let config = fixture();
        let (mut session, room, event) = recovery_seed(&config).await;
        let secret = session.enable_new_recovery().await.expect("setup");
        session.upload_key_backup().await.unwrap();
        let before = recovery_backup_wire(&config).await;
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(25), room_id: matrix_domain::RoomId::new(&room) };
        let paused = PausedServer::new(&config);
        let start = std::time::Instant::now();
        assert!(session.recover_room_keys(secret.expose(), &scope).await.is_err(), "offline recovery must fail, not erase keys");
        assert!(start.elapsed() < std::time::Duration::from_secs(32), "bounded operation deadline");
        drop(paused);
        recovery_assert_message(session.client.as_ref().unwrap(), &room, &event, true).await;
        assert!(recovery_backup_wire(&config).await == before, "transport failure preserves backup");
        session.recover_room_keys(secret.expose(), &scope).await.expect("retry succeeds");
    });
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_missing_backup_is_not_a_successful_room_restore() {
    runtime().block_on(async {
        let config = fixture();
        let source = login(&config, "alice", 25).await;
        let client = source.client.as_ref().unwrap();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        client.encryption().bootstrap_cross_signing(None).await.unwrap();
        // Deliberately incomplete upstream secret storage: no Megolm backup exists.
        let store = client.encryption().secret_storage().create_secret_store().await.unwrap();
        let key = store.secret_storage_key();
        let mut target = login(&config, "alice", 26).await;
        let result = target.recover_room_keys(&key, &matrix_domain::RoomScope {
            account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new("!missing:matrix-desktop.test"),
        }).await;
        assert!(result.is_err(), "SDK download no-op without backup must not count as restore");
    });
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_requires_complete_identity_before_new_setup() {
    runtime().block_on(async {
        let config = fixture();
        let mut session = login(&config, "alice", 25).await;
        let client = session.client.as_ref().unwrap().clone();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        assert!(session.enable_new_recovery().await.is_err(), "incomplete identity must not produce a misleading recovery key");
        assert!(!client.encryption().backups().fetch_exists_on_server().await.unwrap(), "no partial backup created");
        assert!(client.encryption().secret_storage().fetch_default_key_id().await.unwrap().is_none());
    });
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_rejects_wrong_local_account_before_import() {
    runtime().block_on(async {
        let config = fixture();
        let (mut source, room, event) = recovery_seed(&config).await;
        let secret = source.enable_new_recovery().await.expect("source setup");
        source.upload_key_backup().await.unwrap();
        let mut target = login(&config, "alice", 26).await;
        let client = target.client.as_ref().unwrap().clone();
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let result = target.recover_room_keys(secret.expose(), &matrix_domain::RoomScope {
            account_id: AccountId::new(999), room_id: matrix_domain::RoomId::new(&room),
        }).await;
        assert!(result.is_err(), "wrong local account must not import any secrets");
        recovery_assert_message(&client, &room, &event, false).await;
    });
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_new_account_recovery_key_is_valid_and_backup_is_on() {
    use matrix_sdk::encryption::recovery::RecoveryState;
    runtime().block_on(async {
        let config = fixture();
        // Fresh account: no identity, no secret storage, no backup (what «Crea chiave di recupero» meets).
        let mut session = login(&config, "alice", 25).await;
        let client = session.client.as_ref().unwrap().clone();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        assert_eq!(crate::recovery::recovery_disabled(&client), Some(true));
        let secret = session.reset_recovery_key().await.expect("create from Disabled");
        for _ in 0..20 {
            if client.encryption().recovery().state() == RecoveryState::Enabled { break; }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        assert_eq!(client.encryption().recovery().state(), RecoveryState::Enabled);
        assert!(client.encryption().backups().fetch_exists_on_server().await.unwrap(), "backup active");
        assert_eq!(crate::recovery::recovery_disabled(&client), Some(false));
        // The key opens the secret storage from a second device.
        let other = login(&config, "alice", 26).await;
        let other_client = other.client.as_ref().unwrap().clone();
        other_client.encryption().wait_for_e2ee_initialization_tasks().await;
        other_client.encryption().recovery().recover(secret.expose()).await.expect("recover() with the new key");
        assert_eq!(other_client.encryption().recovery().state(), RecoveryState::Enabled);
        // A second «create» on the same account must not rotate the key.
        assert!(session.enable_new_recovery().await.is_err());
    });
}
#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_new_device_decrypts_backup_after_runtime_restart() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let passphrase = "disposable encrypted store passphrase";
    let (room, event, secret) = runtime().block_on(async {
        let (mut source, room, event) = recovery_seed(&config).await;
        let secret = source.enable_new_recovery().await.expect("source recovery enabled");
        source.upload_key_backup().await.expect("source keys uploaded");
        (room, event, secret)
    }); // Source runtime and all SDK tasks are gone before the fresh login.
    let device = runtime().block_on(async {
        let mut recovered = AccountStore::create(root, AccountId::new(26)).unwrap()
            .password_login_with_http(HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(), passphrase, http(&config)).await.unwrap();
        let client = recovered.client.as_ref().unwrap().clone();
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        recovery_assert_message(&client, &room, &event, false).await;
        recovered.recover_room_keys(secret.expose(), &matrix_domain::RoomScope {
            account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&room),
        }).await.expect("recover secrets and room backup");
        recovery_assert_message(&client, &room, &event, true).await;
        // The SSSS recovery key must never be persisted as plaintext by our facade.
        for entry in std::fs::read_dir(root.join("account-26")).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(!bytes.windows(secret.expose().len()).any(|w| w == secret.expose().as_bytes()), "recovery key leaked to disk");
        }
        recovered.whoami().await.unwrap().device_id().to_owned()
    });
    drop(secret); // No recovery secret supplied at restart, only encrypted SDK store.
    runtime().block_on(async {
        let mut recovered = AccountStore::open(root, AccountId::new(26)).unwrap()
            .restore_with_http(passphrase, http(&config)).await.unwrap();
        assert!(recovered.whoami().await.unwrap().device_id() == device);
        let client = recovered.client.as_ref().unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(client).await.unwrap();
        recovery_assert_message(client, &room, &event, true).await;
        assert!(client.encryption().backups().are_enabled().await, "backup key/version survive restart");
        recovered.upload_key_backup().await.expect("backup remains usable after restart");
    });
}

async fn recovery_assert_message(client: &matrix_sdk::Client, room: &str, event: &matrix_sdk::ruma::EventId, decryptable: bool) {
    let room = client.get_room(&matrix_sdk::ruma::RoomId::parse(room).unwrap()).unwrap();
    let page = room.messages(matrix_sdk::room::MessagesOptions::backward()).await.unwrap();
    let row = page.chunk.iter().find(|row| row.event_id() == Some(event)).expect("old event exists");
    assert_eq!(row.encryption_info().is_some(), decryptable, "actual SDK decryption required");
    if decryptable {
        assert!(row.raw().get_field::<serde_json::Value>("content").unwrap().unwrap()["body"] == "recover old encrypted città 🦀");
    }
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_uploads_actual_room_key_as_ciphertext() {
    runtime().block_on(async {
        let config = fixture();
        let (mut session, room, _event) = recovery_seed(&config).await;
        let secret = session.enable_new_recovery().await.expect("new recovery enabled");
        session.upload_key_backup().await.expect("backup upload confirmed");
        let backup = recovery_backup_wire(&config).await;
        let sessions = backup["rooms"][&room]["sessions"].as_object().expect("room included in backup");
        assert!(!sessions.is_empty());
        for value in sessions.values() {
            assert!(value["session_data"]["ciphertext"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(value["session_data"].get("session_key").is_none());
        }
        assert!(!backup.to_string().contains(secret.expose()), "secret storage key never uploaded in plaintext");
    });
}

async fn recovery_seed(config: &serde_json::Value) -> (AccountSession, String, matrix_sdk::ruma::OwnedEventId) {
    let room = room_request(config, "alice", reqwest::Method::POST, "createRoom",
        serde_json::json!({"name":"disposable recovery", "visibility":"private", "initial_state":[
            {"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}
        ]})).await["room_id"].as_str().unwrap().to_owned();
    let session = login(config, "alice", 25).await;
    let client = session.client.as_ref().unwrap();
    client.encryption().wait_for_e2ee_initialization_tasks().await;
    client.encryption().bootstrap_cross_signing(None).await.unwrap();
    crate::room_sync::RoomSync::sliding_catch_up(client).await.unwrap();
    let sdk_room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
    let event = sdk_room.send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("recover old encrypted città 🦀"))
        .await.unwrap().response.event_id;
    (session, room, event)
}

async fn recovery_backup_wire(config: &serde_json::Value) -> serde_json::Value {
    let version = http(config).get(format!("{}_matrix/client/v3/room_keys/version", config["url"].as_str().unwrap()))
        .bearer_auth(config["alice_token"].as_str().unwrap()).send().await.unwrap();
    assert!(version.status().is_success(), "backup metadata readable");
    let version: serde_json::Value = serde_json::from_slice(&version.bytes().await.unwrap()).unwrap();
    let mut endpoint = url::Url::parse(&format!("{}_matrix/client/v3/room_keys/keys", config["url"].as_str().unwrap())).unwrap();
    endpoint.query_pairs_mut().append_pair("version", version["version"].as_str().unwrap());
    let backup = http(config).get(endpoint)
        .bearer_auth(config["alice_token"].as_str().unwrap()).send().await.unwrap();
    assert!(backup.status().is_success(), "backup keys readable");
    serde_json::from_slice(&backup.bytes().await.unwrap()).unwrap()
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_second_enable_preserves_existing_secret_storage() {
    runtime().block_on(async {
        let config = fixture();
        let mut session = login(&config, "alice", 25).await;
        let client = session.client.as_ref().unwrap().clone();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        client.encryption().bootstrap_cross_signing(None).await.unwrap();
        let _secret = session.enable_new_recovery().await.expect("first setup succeeds");
        let before = client.encryption().secret_storage().fetch_default_key_id().await.unwrap().unwrap().deserialize().unwrap().key_id;
        assert!(session.enable_new_recovery().await.is_err(), "must not silently rotate existing secret storage");
        let after = client.encryption().secret_storage().fetch_default_key_id().await.unwrap().unwrap().deserialize().unwrap().key_id;
        assert_eq!(before, after);
    });
}
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_enables_encrypted_backup_without_exposing_key() {
    runtime().block_on(async {
        let config = fixture();
        let mut session = login(&config, "alice", 25).await;
        let client = session.client.as_ref().unwrap().clone();
        client.encryption().wait_for_e2ee_initialization_tasks().await;
        client.encryption().bootstrap_cross_signing(None).await.unwrap();
        let secret = session.enable_new_recovery().await.expect("new recovery enabled");
        assert!(!secret.expose().is_empty(), "secret present, never print its value");
        assert!(client.encryption().backups().fetch_exists_on_server().await.unwrap());
        assert!(client.encryption().backups().are_enabled().await);
        assert_eq!(client.encryption().recovery().state(), matrix_sdk::encryption::recovery::RecoveryState::Enabled);
    });
}

/// A key that entered the backup AFTER the new device did its recovery is fetched by the SDK on the
/// first decryption failure (BackupDownloadStrategy::AfterDecryptionFailure); with the default
/// Manual strategy the message stays undecryptable. Also covers a message from another device
/// of the same account.
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_downloads_later_session_key_on_first_decryption_failure() {
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    runtime().block_on(async {
        let (mut source, room, _first) = recovery_seed(&config).await;
        let secret = source.enable_new_recovery().await.expect("source recovery enabled");
        source.upload_key_backup().await.expect("source keys uploaded");
        let client_a = source.client.as_ref().unwrap().clone();
        // A new room = a new Megolm session. It is sent BEFORE B exists, so B can only get it from the backup.
        let room2 = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":"disposable later session", "visibility":"private", "initial_state":[
                {"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}
            ]})).await["room_id"].as_str().unwrap().to_owned();
        client_a.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        let event = client_a.get_room(&matrix_sdk::ruma::RoomId::parse(&room2).unwrap()).unwrap()
            .send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("recover old encrypted città 🦀"))
            .await.unwrap().response.event_id;
        source.upload_key_backup().await.expect("second session backed up");
        let mut recovered = AccountStore::create(root, AccountId::new(26)).unwrap()
            .password_login_with_http(HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(),
                "disposable encrypted store passphrase", http(&config)).await.unwrap();
        let client_b = recovered.client.as_ref().unwrap().clone();
        client_b.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        recovered.recover_room_keys(secret.expose(), &matrix_domain::RoomScope {
            account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&room),
        }).await.expect("recover secrets");
        let mut decrypted = false;
        for _ in 0..10 {
            client_b.sync_once(matrix_sdk::config::SyncSettings::default().timeout(std::time::Duration::from_millis(500))).await.unwrap();
            if let Some(r) = client_b.get_room(&matrix_sdk::ruma::RoomId::parse(&room2).unwrap()) {
                let page = r.messages(matrix_sdk::room::MessagesOptions::backward()).await.unwrap();
                decrypted = page.chunk.iter().any(|row| row.event_id() == Some(&*event) && row.encryption_info().is_some());
            }
            if decrypted { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        assert!(decrypted, "later session key must come from the backup on first decryption failure");
    });
}

/// Device B has the rooms' timelines open (UTD rows in cache) BEFORE it recovers; after recovery the
/// same timelines must show the text without being reopened, and a restart from the store only must keep it.
/// `all` = the sign-in path `recover_all_room_keys` over `rooms` rooms, else `recover_room_keys` on room 0.
/// Counters (printed, not production code): sessions imported per room via the SDK backup stream, decryptable
/// rows in `Room::messages` (= session present in the store) and in the live timeline.
async fn recovery_utd_scenario(rooms: usize, all: bool) {
    use futures_util::StreamExt;
    use matrix_sdk_ui::timeline::RoomExt;
    let config = fixture();
    let root = std::path::Path::new(config["ca"].as_str().unwrap()).parent().unwrap();
    let pass = "disposable encrypted store passphrase";
    let mut a = login(&config, "alice", 25).await;
    let client_a = a.client.as_ref().unwrap().clone();
    client_a.encryption().wait_for_e2ee_initialization_tasks().await;
    client_a.encryption().bootstrap_cross_signing(None).await.unwrap();
    let secret = a.enable_new_recovery().await.expect("source recovery enabled");
    let mut ids = Vec::new();
    for n in 0..rooms {
        ids.push(room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"name":format!("utd {n}"), "visibility":"private", "initial_state":[
                {"type":"m.room.encryption", "state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}
            ]})).await["room_id"].as_str().unwrap().to_owned());
    }
    client_a.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
    for id in &ids {
        client_a.get_room(&matrix_sdk::ruma::RoomId::parse(id).unwrap()).unwrap()
            .send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain("old città 🦀"))
            .await.unwrap();
    }
    a.upload_key_backup().await.expect("keys backed up");
    // A decrypts rows with its own keys: `Room::messages` rows of room 0 that carry encryption info.
    let readable = |client: matrix_sdk::Client, ids: Vec<String>| async move {
        let mut n = 0;
        for id in ids {
            let Some(room) = client.get_room(&matrix_sdk::ruma::RoomId::parse(&id).unwrap()) else { continue };
            let page = room.messages(matrix_sdk::room::MessagesOptions::backward()).await.unwrap();
            n += page.chunk.iter().filter(|r| r.encryption_info().is_some()).count();
        }
        n
    };
    let mut b = AccountStore::create(root, AccountId::new(26)).unwrap()
        .password_login_with_http(HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@alice:matrix-desktop.test", config["alice_password"].as_str().unwrap(), pass, http(&config)).await.unwrap();
    let client_b = b.client.as_ref().unwrap().clone();
    client_b.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
    let open = |rid: String| {
        let client = client_b.clone();
        async move {
            let room = client.get_room(&matrix_sdk::ruma::RoomId::parse(&rid).unwrap()).unwrap();
            let timeline = room.timeline().await.unwrap();
            timeline.paginate_backwards(20).await.unwrap();
            timeline
        }
    };
    let mut timelines = Vec::new();
    for id in ids.iter().take(if all { rooms } else { 1 }) { timelines.push(open(id.clone()).await); }
    async fn counts(timelines: &[matrix_sdk_ui::timeline::Timeline]) -> (usize, usize) {
        use matrix_sdk_ui::timeline::MsgLikeKind;
        let (mut utd, mut text) = (0, 0);
        for t in timelines {
            for item in t.items().await.iter() {
                match item.as_event().and_then(|e| e.content().as_msglike()).map(|m| &m.kind) {
                    Some(MsgLikeKind::UnableToDecrypt(_)) => utd += 1,
                    Some(MsgLikeKind::Message(_)) => text += 1,
                    _ => {}
                }
            }
        }
        (utd, text)
    }
    let before = counts(&timelines).await;
    assert!(before.0 > 0 && before.1 == 0, "precondition: rows are undecryptable before recovery, got {before:?}");
    let imported = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let watch = {
        let imported = imported.clone();
        let mut stream = Box::pin(client_b.encryption().backups().room_keys_for_room_stream(
            &matrix_sdk::ruma::RoomId::parse(&ids[0]).unwrap()));
        tokio::task::spawn(async move {
            while let Some(Ok(keys)) = stream.next().await {
                imported.fetch_add(keys.values().map(|s| s.len()).sum::<usize>(), std::sync::atomic::Ordering::SeqCst);
            }
        })
    };
    let started = std::time::Instant::now();
    let outcome = if all {
        b.recover_all_room_keys(secret.expose()).await
    } else {
        b.recover_room_keys(secret.expose(), &matrix_domain::RoomScope {
            account_id: AccountId::new(26), room_id: matrix_domain::RoomId::new(&ids[0]),
        }).await
    };
    let took = started.elapsed();
    let mut now = counts(&timelines).await;
    // The live timeline is given time to re-decrypt by itself; it is never reopened.
    for _ in 0..40 {
        if now.0 == 0 { break; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        now = counts(&timelines).await;
    }
    let want = if all { rooms } else { 1 };
    let in_store = readable(client_b.clone(), ids.iter().take(want).cloned().collect()).await;
    println!("RECOVERY_COUNTERS rooms={rooms} all={all} outcome={outcome:?} took_ms={} stream_imported_room0={} \
        store_readable={in_store}/{want} timeline_before={before:?} timeline_after={now:?}",
        took.as_millis(), imported.load(std::sync::atomic::Ordering::SeqCst));
    watch.abort();
    outcome.expect("recovery succeeds");
    assert_eq!(in_store, want, "every recovered session must be usable from the store");
    assert_eq!(now, (0, want), "open timelines show the text without being reopened");
    // Restart B from the store only: nothing may be asked of the recovery key again.
    let device = b.whoami().await.unwrap().device_id().to_owned();
    drop(timelines); drop(watch); drop(client_b); drop(b); drop(client_a); drop(a);
    let restored = AccountStore::open(root, AccountId::new(26)).unwrap().restore_with_http(pass, http(&config)).await.unwrap();
    assert!(restored.whoami().await.unwrap().device_id() == device);
    let client = restored.client.as_ref().unwrap().clone();
    client.sync_once(matrix_sdk::config::SyncSettings::default().timeout(std::time::Duration::from_millis(500))).await.unwrap();
    assert_eq!(readable(client, ids.iter().take(want).cloned().collect()).await, want, "after restart the store still decrypts");
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_utd_rows_decrypt_in_open_timeline_one_room() {
    runtime().block_on(recovery_utd_scenario(1, false));
}

#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_recovery_utd_rows_decrypt_in_open_timelines_many_rooms() {
    // Synapse's default join limit (burst 10, then one per 10 s) caps a user at 10 new rooms; the harness limits stay untouched.
    let rooms = if std::env::var("PROBE_SERVER").is_ok_and(|s| s == "tuwunel") { 30 } else { 10 };
    runtime().block_on(recovery_utd_scenario(rooms, true));
}
