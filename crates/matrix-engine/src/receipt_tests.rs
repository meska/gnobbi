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
        sync.sliding_round().await.unwrap();
        let sent = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p054-message"),
            serde_json::json!({"msgtype":"m.text","body":"città 🦀"})).await;
        let event = sent["event_id"].as_str().unwrap();
        room_request(&config, "alice", reqwest::Method::POST,
            &format!("rooms/{room}/receipt/m.read/{event}"), serde_json::json!({})).await;
        for change in sync.sliding_round().await.unwrap() {
            if let RoomListDiff::Upsert(summary) = change {
                assert!(summary.info.public_read_receipts.is_empty(), "own ACK/marker is not peer reading");
            }
        }
        let mut batch = sync.sliding_round_after(async {
            room_request(&config, "bob", reqwest::Method::POST,
                &format!("rooms/{room}/receipt/m.read/{event}"), serde_json::json!({})).await;
        }).await.unwrap();
        let independent: serde_json::Value = serde_json::from_str(&http(&config)
            .get(format!("{}_matrix/client/v3/sync", config["url"].as_str().unwrap()))
            .bearer_auth(config["alice_token"].as_str().unwrap())
            .send().await.unwrap().error_for_status().unwrap().text().await.unwrap()).unwrap();
        assert!(independent["rooms"]["join"][room]["ephemeral"]["events"].as_array().unwrap().iter().any(|e|
            e["type"] == "m.receipt" && e["content"][event]["m.read"]["@bob:matrix-desktop.test"].is_object()), "independent HTTP readback must identify exact peer/event");
        let mut found = false;
        for _ in 0..8 {
            for change in batch {
                if let RoomListDiff::Upsert(summary) = change {
                    assert_eq!(summary.scope.account_id, AccountId::new(54));
                    assert_eq!(summary.scope.room_id.as_str(), room);
                    if summary.info.public_read_receipts.iter().any(|r| r.event_id.as_str() == event && r.readers == 1) {
                        found = true;
                    }
                }
            }
            if found { break; }
            batch = sync.sliding_round().await.unwrap();
        }
        assert!(found, "exact public peer marker must reach Dime after incremental sync");
        assert!(sync.sliding_round().await.unwrap().is_empty(), "unchanged markers must not emit duplicate diffs");
        println!("RECEIPT {}", serde_json::json!({"room":room,"event":event,"peer":"@bob:matrix-desktop.test","readers":1,"account":54,"independent_http":true}));
        let next = room_request(&config, "alice", reqwest::Method::PUT,
            &format!("rooms/{room}/send/m.room.message/p054-next"),
            serde_json::json!({"msgtype":"m.text","body":"Next marker"})).await;
        let next = next["event_id"].as_str().unwrap();
        for change in sync.sliding_round_after(async {
            room_request(&config, "bob", reqwest::Method::POST,
                &format!("rooms/{room}/receipt/m.read.private/{next}"), serde_json::json!({})).await;
        }).await.unwrap() {
            if let RoomListDiff::Upsert(summary) = change {
                assert!(!summary.info.public_read_receipts.iter().any(|r| r.event_id.as_str() == next), "private marker must not advance public peer state");
            }
        }
        let updated = sync.sliding_round_after(async {
            room_request(&config, "bob", reqwest::Method::POST,
                &format!("rooms/{room}/receipt/m.read/{next}"), serde_json::json!({})).await;
        }).await.unwrap();
        let summary = updated.iter().find_map(|d| match d { RoomListDiff::Upsert(s) if s.scope.room_id.as_str() == room => Some(s), _ => None }).expect("changed peer marker emits room update");
        assert_eq!(summary.info.public_read_receipts.len(), 1);
        assert_eq!(summary.info.public_read_receipts[0].event_id.as_str(), next);
        assert_eq!(summary.info.public_read_receipts[0].readers, 1);
        println!("RECEIPT_UPDATE {}", serde_json::json!({"room":room,"previous_event":event,"event":next,"readers":1,"private_did_not_advance":true}));
        room_request(&config, "alice", reqwest::Method::POST,
            &format!("rooms/{room}/leave"), serde_json::json!({})).await;
        assert!(sync.sliding_round().await.unwrap().iter().any(|d| matches!(d, RoomListDiff::Remove(s) if s.room_id.as_str() == room)));
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
        // open_history needs the SDK event cache, which only the store-backed session has.
        let root = std::env::temp_dir().join(format!("gnobbi-test-unread-{}", std::process::id()));
        crate::fsx::create_private_dir(&root).unwrap();
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
                for change in sync.sliding_round().await.unwrap() {
                    if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room {
                        unread = Some(s.unread);
                    }
                }
                if unread == Some(want) { break; }
            }
            unread
        };
        // Sent while the sliding run is live: it counts unread from the deliveries it sees.
        let mut unread = None;
        let batch = sync.sliding_round_after(async {
            for i in 0..N {
                room_request(&config, "alice", reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/unread-{i}"),
                    serde_json::json!({"msgtype":"m.text","body":format!("unread {i}")})).await;
            }
        }).await.unwrap();
        for change in batch {
            if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room {
                unread = Some(s.unread);
            }
        }
        // Tuwunel counts 2 of the 3 (measured, same before the Timeline change): the contract is "some unread, then 0".
        let unread = settle(&mut sync, unread, N).await;
        assert!(unread.is_some_and(|n| (1..=N).contains(&n)), "precondition: {N} unread messages shown before opening, got {unread:?}");

        let scope = matrix_domain::RoomScope {
            account_id: AccountId::new(56),
            room_id: matrix_domain::RoomId::new(room.clone()),
        };
        let (page, _) = sync.open_history(ViewRequests::new().start(scope).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        assert!(page.events.len() >= N as usize, "the opened room shows the {N} messages");

        let unread = settle(&mut sync, unread, 0).await;
        drop(sync);
        let _ = std::fs::remove_dir_all(&root); // before the assert: a RED panic must not leave state
        assert_eq!(unread, Some(0), "opening the room must clear the unread counter, but it is still {unread:?}");
    });
}

/// Room menu: «Segna come non letto» sets the SDK flag, «Segna come letto» clears it and the counter.
/// Run: `python3 the auth_smoke script --filter disposable_room_menu_marks_read_and_unread`
#[test]
#[ignore = "requires disposable Synapse; needs the external test harness (not in this repo)"]
fn disposable_room_menu_marks_read_and_unread() {
    use crate::{RoomListDiff, ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let send = async |tag: &str| {
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/menu-{tag}"),
                serde_json::json!({"msgtype":"m.text","body":tag})).await;
        };
        send("a").await;
        let root = std::env::temp_dir().join(format!("gnobbi-test-menu-{}", std::process::id()));
        crate::fsx::create_private_dir(&root).unwrap();
        let bob = AccountStore::create(&root, AccountId::new(57)).unwrap()
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
        // (unread, marked_unread) of the room once `done` holds, or the last seen after 8 batches.
        let settle = async |sync: &mut RoomSync, mut seen: Option<(u64, bool)>, done: fn((u64, bool)) -> bool| {
            for _ in 0..8 {
                for change in sync.sliding_round().await.unwrap() {
                    if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room {
                        seen = Some((s.unread, s.info.marked_unread));
                    }
                }
                if seen.is_some_and(done) { break; }
            }
            seen
        };
        let seen = settle(&mut sync, None, |(unread, _)| unread == 1).await;
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(57), room_id: matrix_domain::RoomId::new(room.clone()) };
        // Fills the event cache that «Segna come letto» takes the newest event from.
        let visible = crate::receipts::ReadVisibility { window_focused: false, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), visible).await.unwrap();
        let unread_result = sync.mark_room_for_test(&scope, false).await;
        let flagged = settle(&mut sync, seen, |(_, marked)| marked).await;
        let read_result = sync.mark_room_for_test(&scope, true).await;
        let cleared = settle(&mut sync, flagged, |(unread, marked)| unread == 0 && !marked).await;
        drop(sync);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(seen, Some((1, false)), "precondition: one unread message, no flag");
        assert!(unread_result.is_ok() && read_result.is_ok(), "both commands accepted");
        assert_eq!(flagged.map(|s| s.1), Some(true), "«Segna come non letto» must set the room flag");
        assert_eq!(cleared, Some((0, false)), "«Segna come letto» must clear the flag and the counter");
    });
}

/// Probe: bob reads through the Timeline, alice sees his receipt on the item and bob's counter drops.
/// Run: `python3 the auth_smoke script --filter disposable_receipt_bob_reads_through_timeline`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_receipt_bob_reads_through_timeline() {
    use crate::{RoomListDiff, ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let root = std::env::temp_dir().join(format!("gnobbi-test-bobreads-{}", std::process::id()));
        crate::fsx::create_private_dir(&root).unwrap();
        let (a_root, b_root) = (root.join("a"), root.join("b"));
        crate::fsx::create_private_dir(&a_root).unwrap();
        crate::fsx::create_private_dir(&b_root).unwrap();
        let mut sessions = Vec::new();
        for (dir, id, user, key) in [(&a_root, 57, "@alice:matrix-desktop.test", "alice_password"), (&b_root, 58, "@bob:matrix-desktop.test", "bob_password")] {
            sessions.push(AccountStore::create(dir, AccountId::new(id)).unwrap()
                .password_login_with_http(HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(), user,
                    config[key].as_str().unwrap(), "long disposable fixture passphrase", http(&config))
                .await.expect("fixture login accepted"));
        }
        let (bob, alice) = (sessions.pop().unwrap(), sessions.pop().unwrap());
        let (mut alice, mut bob) = (RoomSync::new(alice), RoomSync::new(bob));
        alice.sliding_round().await.unwrap();
        // Alice sends while bob's sliding run is live, so bob counts it as unread.
        let mut bob_unread = None;
        for change in bob.sliding_round_after(async {
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/bob-reads"),
                serde_json::json!({"msgtype":"m.text","body":"read me"})).await;
        }).await.unwrap() {
            if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room { bob_unread = Some(s.unread); }
        }
        assert_eq!(bob_unread, Some(1), "bob has one unread message");
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(58), room_id: matrix_domain::RoomId::new(room.clone()) };
        let (page, _) = bob.open_history(ViewRequests::new().start(scope).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let event = page.events.last().unwrap().event_id.as_str().to_owned();
        let (mut alice_sees, mut bob_unread) = (false, Some(1));
        for _ in 0..8 {
            for change in alice.sliding_round().await.unwrap().into_iter().chain(bob.sliding_round().await.unwrap()) {
                if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room {
                    if s.scope.account_id == AccountId::new(57) && s.info.public_read_receipts.iter().any(|r| r.event_id.as_str() == event && r.readers == 1) { alice_sees = true; }
                    if s.scope.account_id == AccountId::new(58) { bob_unread = Some(s.unread); }
                }
            }
            if alice_sees && bob_unread == Some(0) { break; }
        }
        drop((alice, bob));
        let _ = std::fs::remove_dir_all(&root);
        assert!(alice_sees, "alice sees bob's receipt on the item");
        assert_eq!(bob_unread, Some(0), "bob's counter dropped");
        println!("BOB_READS {}", serde_json::json!({"room": room, "event": event, "alice_sees_receipt": true, "bob_unread": 0}));
    });
}

/// Progressive reading: alice sends N messages, bob (never read) moves his marker forward in steps, as the app does while
/// the user scrolls: each step puts the public or private receipt and `m.fully_read` on that exact event, the server count
/// goes down, alice sees only the public one, and reaching the newest message leaves nothing unread.
/// Run: `python3 the auth_smoke script --filter disposable_progressive_read [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_progressive_read_moves_marker_and_counter() {
    use crate::{RoomListDiff, ViewRequests, room_sync::RoomSync};
    const N: usize = 15;
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let root = std::env::temp_dir().join(format!("gnobbi-test-progress-{}", std::process::id()));
        crate::fsx::create_private_dir(&root).unwrap();
        let bob = AccountStore::create(&root, AccountId::new(160)).unwrap()
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
        let mut ids = Vec::new();
        let mut unread = None;
        let batch = sync.sliding_round_after(async {
            for i in 0..N {
                let sent = room_request(&config, "alice", reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/progress-{i}"),
                    serde_json::json!({"msgtype":"m.text","body":format!("progress {i}")})).await;
                ids.push(sent["event_id"].as_str().unwrap().to_owned());
            }
        }).await.unwrap();
        for change in batch {
            if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room { unread = Some(s.unread); }
        }
        // The room list's count once it is at or below `max` (or after 8 batches).
        let settle = async |sync: &mut RoomSync, mut unread: Option<u64>, max: u64| {
            for _ in 0..8 {
                if unread.is_some_and(|n| n <= max) { break; }
                for change in sync.sliding_round().await.unwrap() {
                    if let RoomListDiff::Upsert(s) = change && s.scope.room_id.as_str() == room { unread = Some(s.unread); }
                }
            }
            unread
        };
        let before = settle(&mut sync, unread, u64::MAX).await.expect("unread shown before opening");
        assert!(before >= 1, "precondition: unread messages before opening");
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(160), room_id: matrix_domain::RoomId::new(room.clone()) };
        // Opened without the newest message in view: the open itself sends no receipt (the app keeps it back for the first-unread anchor).
        let away = crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 5000.0, public_receipts: true };
        let (page, _) = sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), away).await.unwrap();
        assert!(page.events.len() >= N, "the room shows the {N} messages");
        let (client, timelines) = sync.live_handles_for_test();
        let account = AccountId::new(160);
        let marks = async || crate::read_progress::markers(&client, &timelines, account, &scope).await.unwrap();
        let known = |m: &[matrix_domain::EventId], id: &str| m.iter().any(|e| e.as_str() == id);
        let m0 = marks().await;
        assert!(!ids.iter().any(|id| known(&m0, id)), "nothing of the {N} is read yet: {}", m0.len());
        // Alice's independent view of bob's public marker.
        let calls = std::cell::Cell::new(0);
        let public_marker = async || {
            // Synapse caches identical sync requests for minutes: every call carries its own filter.
            calls.set(calls.get() + 1);
            let filter = serde_json::json!({"room": {"timeline": {"limit": calls.get()}}}).to_string();
            let all: serde_json::Value = serde_json::from_str(&http(&config)
                .get(format!("{}_matrix/client/v3/sync?filter={filter}", config["url"].as_str().unwrap()))
                .bearer_auth(config["alice_token"].as_str().unwrap())
                .send().await.unwrap().error_for_status().unwrap().text().await.unwrap()).unwrap();
            all["rooms"]["join"][&room]["ephemeral"]["events"].as_array().into_iter().flatten()
                .filter(|e| e["type"] == "m.receipt")
                .flat_map(|e| e["content"].as_object().into_iter().flatten())
                .filter(|(_, r)| r["m.read"]["@bob:matrix-desktop.test"].is_object())
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        let mut last = before;
        for (step, (at, public)) in [(4usize, true), (9, true), (12, false)].into_iter().enumerate() {
            let sent = crate::read_progress::advance(&client, &timelines, account, &scope, &matrix_domain::EventId::new(ids[at].clone()), public).await.unwrap();
            assert!(sent, "step {step}: a receipt goes out");
            let now = settle(&mut sync, Some(last), last - 1).await.unwrap();
            // The own receipt and fully_read come back through sync into the store the markers are read from.
            let mut stored = known(&marks().await, &ids[at]);
            for _ in 0..8 {
                if stored { break; }
                sync.sliding_round().await.unwrap();
                stored = known(&marks().await, &ids[at]);
            }
            assert!(stored, "step {step}: the marker is on the event");
            assert!(now < last, "step {step}: the server count goes down ({last} -> {now})");
            last = now;
            let seen = public_marker().await;
            let want = if public { &ids[at] } else { &ids[9] };
            assert_eq!(seen, [want.clone()], "step {step}: peers see only the public marker (ids: {ids:?})");
        }
        // Same event again: nothing sent. Then the newest: everything is read.
        assert!(!crate::read_progress::advance(&client, &timelines, account, &scope, &matrix_domain::EventId::new(ids[12].clone()), false).await.unwrap());
        assert!(crate::read_progress::advance(&client, &timelines, account, &scope, &matrix_domain::EventId::new(ids[N - 1].clone()), true).await.unwrap());
        let end = settle(&mut sync, Some(last), 0).await;
        assert_eq!(public_marker().await, [ids[N - 1].clone()]);
        println!("PROGRESS_READ {}", serde_json::json!({"n": N, "before": before, "end": end}));
        drop(sync);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(end, Some(0), "reaching the newest message leaves nothing unread");
    });
}
