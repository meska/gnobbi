#[test]
#[ignore = "requires isolated real TLS homeserver"]
fn disposable_receipt_public_peer_reaches_incremental_room_projection() {
    use crate::{RoomListDiff, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let mut sync = RoomSync::new(login(&config, "alice", 54).await);
        sync.next_batch().await.unwrap();
        let sent = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p054-message"),
            serde_json::json!({"msgtype":"m.text","body":"città 🦀"})).await;
        let event = sent["event_id"].as_str().unwrap();
        room_request(&config, "alice", reqwest::Method::POST,
            &format!("rooms/{room}/receipt/m.read/{event}"), serde_json::json!({})).await;
        for change in sync.next_batch().await.unwrap() {
            if let RoomListDiff::Upsert(summary) = change {
                assert!(summary.info.public_read_receipts.is_empty(), "own ACK/marker is not peer reading");
            }
        }
        room_request(&config, "bob", reqwest::Method::POST,
            &format!("rooms/{room}/receipt/m.read/{event}"), serde_json::json!({})).await;
        let independent: serde_json::Value = serde_json::from_str(&http(&config)
            .get(format!("{}_matrix/client/v3/sync", config["url"].as_str().unwrap()))
            .bearer_auth(config["alice_token"].as_str().unwrap())
            .send().await.unwrap().error_for_status().unwrap().text().await.unwrap()).unwrap();
        assert!(independent["rooms"]["join"][room]["ephemeral"]["events"].as_array().unwrap().iter().any(|e|
            e["type"] == "m.receipt" && e["content"][event]["m.read"]["@bob:matrix-desktop.test"].is_object()), "independent HTTP readback must identify exact peer/event");
        let mut found = false;
        for _ in 0..8 {
            for change in sync.next_batch().await.unwrap() {
                if let RoomListDiff::Upsert(summary) = change {
                    assert_eq!(summary.scope.account_id, AccountId::new(54));
                    assert_eq!(summary.scope.room_id.as_str(), room);
                    if summary.info.public_read_receipts.iter().any(|r| r.event_id.as_str() == event && r.readers == 1) {
                        found = true;
                    }
                }
            }
            if found { break; }
        }
        assert!(found, "exact public peer marker must reach Dime after incremental sync");
        assert!(sync.next_batch().await.unwrap().is_empty(), "unchanged markers must not emit duplicate diffs");
        println!("P054_RECEIPT {}", serde_json::json!({"room":room,"event":event,"peer":"@bob:matrix-desktop.test","readers":1,"account":54,"independent_http":true}));
        let next = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p054-next"),
            serde_json::json!({"msgtype":"m.text","body":"Next marker"})).await;
        let next = next["event_id"].as_str().unwrap();
        room_request(&config, "bob", reqwest::Method::POST,
            &format!("rooms/{room}/receipt/m.read.private/{next}"), serde_json::json!({})).await;
        for change in sync.next_batch().await.unwrap() {
            if let RoomListDiff::Upsert(summary) = change {
                assert!(!summary.info.public_read_receipts.iter().any(|r| r.event_id.as_str() == next), "private marker must not advance public peer state");
            }
        }
        room_request(&config, "bob", reqwest::Method::POST,
            &format!("rooms/{room}/receipt/m.read/{next}"), serde_json::json!({})).await;
        let updated = sync.next_batch().await.unwrap();
        let summary = updated.iter().find_map(|d| match d { RoomListDiff::Upsert(s) if s.scope.room_id.as_str() == room => Some(s), _ => None }).expect("changed peer marker emits room update");
        assert_eq!(summary.info.public_read_receipts.len(), 1);
        assert_eq!(summary.info.public_read_receipts[0].event_id.as_str(), next);
        assert_eq!(summary.info.public_read_receipts[0].readers, 1);
        println!("P054_UPDATE {}", serde_json::json!({"room":room,"previous_event":event,"event":next,"readers":1,"private_did_not_advance":true}));
        room_request(&config, "alice", reqwest::Method::POST,
            &format!("rooms/{room}/leave"), serde_json::json!({})).await;
        assert!(sync.next_batch().await.unwrap().iter().any(|d| matches!(d, RoomListDiff::Remove(s) if s.room_id.as_str() == room)));
    });
}

/// RED (known bug): opening a room with N unread messages leaves the counter at N.
///
/// Scenario on a disposable Synapse with synthetic accounts only: alice sends N messages into a
/// two-member room, bob's room list shows unread == N; bob then opens the room through the engine
/// (`open_history`, the call the desktop makes) and, after further syncs, the counter must drop to 0.
/// Today nothing sends a read receipt or fully-read marker on open, so the server keeps counting
/// and the assertion at the end fails with "unread still 3".
/// Run: `python3 the auth_smoke script --filter disposable_open_room_clears_unread_counter`
/// (fresh Synapse in a loopback-only bwrap sandbox, via `the ct script`).
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_open_room_clears_unread_counter() {
    use crate::{RoomListDiff, ViewRequests, room_sync::RoomSync};
    const N: u64 = 3;
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        for i in 0..N {
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/unread-{i}"),
                serde_json::json!({"msgtype":"m.text","body":format!("unread {i}")})).await;
        }
        // open_history needs the SDK event cache, which only the store-backed session has.
        let root = std::env::temp_dir().join(format!("gnobbi-test-unread-{}", std::process::id()));
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&root).unwrap();
        }
        let bob = AccountStore::create(&root, AccountId::new(56)).unwrap()
            .password_login_with_http(
                HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
                "@bob:matrix-desktop.test",
                config["bob_password"].as_str().unwrap(),
                "long disposable fixture passphrase",
                http(&config),
            )
            .await
            .expect("fixture login accepted");
        let mut sync = RoomSync::new(bob);
        // Latest unread counter shown for the room in the list, once it equals `want` or after 8 batches.
        let settle = async |sync: &mut RoomSync, mut unread: Option<u64>, want: u64| {
            for _ in 0..8 {
                for change in sync.next_batch().await.unwrap() {
                    if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room {
                        unread = Some(s.unread);
                    }
                }
                if unread == Some(want) { break; }
            }
            unread
        };
        let unread = settle(&mut sync, None, N).await;
        assert_eq!(unread, Some(N), "precondition: {N} unread messages shown before opening");

        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(56),
            room_id: matrix_domain::RoomId::new(room.clone()),
        };
        let (page, _) = sync.open_history(ViewRequests::new().start(scope).unwrap()).await.unwrap();
        assert!(page.events.len() >= N as usize, "the opened room shows the {N} messages");

        let unread = settle(&mut sync, unread, 0).await;
        drop(sync);
        let _ = std::fs::remove_dir_all(&root); // before the assert: a RED panic must not leave state
        assert_eq!(unread, Some(0), "opening the room must clear the unread counter, but it is still {unread:?}");
    });
}
