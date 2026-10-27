// Real post-initial-sync activity, not an explicit refresh or summary change.
#[test]
#[ignore = "fixture peer driver; needs the external test harness (not in this repo)"]
fn disposable_gui_receive_peer() {
    runtime().block_on(async {
        let config = fixture();
        let peer = login(&config, "bob", 56).await.client.unwrap();
        peer.encryption().wait_for_e2ee_initialization_tasks().await;
        peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        let room = matrix_sdk::ruma::RoomId::parse(config["receive_room"].as_str().unwrap()).unwrap();
        let room = peer.get_room(&room).unwrap();
        let body = config["receive_body"].as_str().unwrap();
        let sent = room.send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(body))
            .await.unwrap().response.event_id;
        std::fs::write(config["receive_result"].as_str().unwrap(),
            serde_json::to_vec(&serde_json::json!({"event_id":sent})).unwrap()).unwrap();
    });
}
#[test]
#[ignore = "requires isolated real server; needs the external test harness (not in this repo)"]
fn disposable_live_activity_full_route_preserves_accepted_send() {
    activity_route_shutdown(false);
}

#[test]
#[ignore = "requires isolated real server; needs the external test harness (not in this repo)"]
fn disposable_live_activity_closed_route_preserves_accepted_send() {
    activity_route_shutdown(true);
}

fn activity_route_shutdown(close: bool) {
    let config = fixture();
    let (mut worker, mut events, room) = ready_worker(&config);
    let (mut sends, _results) = worker.take_send_route().unwrap();
    runtime().block_on(async {
        // One event per sync fills the existing64 route; no second/unbounded route.
        for i in 0..70 {
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/p055-fill-{i}"),
                serde_json::json!({"msgtype":"m.text", "body":"bounded activity"})).await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        assert!(sends.send(crate::sending::SendRequest {
            scope: matrix_domain::RoomScope { account_id: AccountId::new(22), room_id: matrix_domain::RoomId::new(&room) },
            transaction_id: matrix_domain::TransactionId::new("p055-drain"),
            body: "accepted write survives activity backpressure".into(),
        }).await.is_ok());
    });
    let mut retained = if close { drop(events); None } else { Some(&mut events) };
    worker.shutdown().unwrap();
    if let Some(events) = retained.as_mut() {
        let (mut count, mut activity) = (0, 0);
        while let Ok(event) = events.try_recv() {
            // Previews and order share the route with the activity invalidations.
            if let crate::RoomListEvent::TimelineActivity(scope) = event {
                assert!(scope.account_id == AccountId::new(22) && scope.room_id.as_str() == room);
                activity += 1;
            }
            count += 1;
        }
        assert_eq!(count, 64, "route was really full and accepted events remained drainable");
        assert!(activity > 0, "accepted invalidations remained drainable");
    }
    runtime().block_on(async {
        // Shutdown may retire result delivery; independent server state proves the write.
        let history = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/messages?dir=b&limit=100"), serde_json::json!({})).await;
        let writes: Vec<_> = history["chunk"].as_array().unwrap().iter()
            .filter(|event| event["type"] == "m.room.message").collect();
        assert_eq!(writes.len(), 71, "70 fills and one accepted write, no loss/duplicate");
        let accepted: Vec<_> = writes.iter().filter(|event|
            event["content"]["body"] == "accepted write survives activity backpressure").collect();
        assert_eq!(accepted.len(), 1);
        let id = accepted[0]["event_id"].as_str().unwrap();
        let wire = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{id}"), serde_json::json!({})).await;
        assert_eq!(wire["event_id"].as_str().unwrap(), id);
        assert_eq!(wire["content"]["body"], "accepted write survives activity backpressure");
    });
}
#[test]
#[ignore = "requires isolated real server; needs the external test harness (not in this repo)"]
fn disposable_live_activity_plain_without_summary_change() {
    live_activity_peer(false, false);
}

#[test]
#[ignore = "requires isolated real server; needs the external test harness (not in this repo)"]
fn disposable_live_activity_encrypted_without_summary_change() {
    live_activity_peer(true, false);
}

#[test]
#[ignore = "requires isolated real server; needs the external test harness (not in this repo)"]
fn disposable_live_activity_reconnect_preserves_receive() {
    live_activity_peer(false, true);
}

fn live_activity_peer(encrypted: bool, reconnect: bool) {
    use crate::{RoomListEvent, RoomListDiff, RoomSyncStatus, ViewRequests,
        history::{HistoryRequest, HistoryError}, room_sync::RoomListWorker};
    use matrix_domain::{RoomScope, RoomId};
    use std::time::Duration;
    let config = fixture();
    let rt = runtime();
    let (room, peer) = rt.block_on(async {
        let mut create = serde_json::json!({"visibility":"private", "preset":"private_chat",
            "invite":["@bob:matrix-desktop.test"]});
        if encrypted {
            create["initial_state"] = serde_json::json!([{"type":"m.room.encryption", "state_key":"",
                "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]);
        }
        let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom", create)
            .await["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        room_request(&config, "alice", reqwest::Method::PUT, &format!("pushrules/global/room/{room}"),
            serde_json::json!({"actions":["dont_notify"]})).await;
        let peer = login(&config, "bob", 56).await.client.unwrap();
        peer.encryption().wait_for_e2ee_initialization_tasks().await;
        peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        (room, peer)
    });
    let login_config = config.clone();
    let (mut worker, mut events) = RoomListWorker::spawn(move || async move {
        let session = login(&login_config, "alice", 55).await;
        session.client.as_ref().unwrap().encryption().wait_for_e2ee_initialization_tasks().await;
        Ok(session)
    }).unwrap();
    let (mut commands, mut history) = worker.take_history_route().unwrap();
    rt.block_on(async {
        let initial = tokio::time::timeout(Duration::from_secs(10), async {
            let (mut summary, mut ready) = (None, false);
            while let Some(event) = events.recv().await {
                match event {
                    RoomListEvent::Diff(RoomListDiff::Upsert(room)) => summary = Some(room),
                    RoomListEvent::Status(RoomSyncStatus::Ready) => ready = true,
                    _ => {}
                }
                if ready && summary.is_some() {
                    break;
                }
            }
            summary.expect("initial room summary")
        }).await.unwrap();
        assert_eq!(initial.unread, 0);
        let scope = RoomScope { account_id: AccountId::new(55), room_id: RoomId::new(&room) };
        assert_eq!(initial.scope, scope);
        // Flush peer device tracking before encrypting for Dime's new device.
        peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
        // Settle: order, previews and (encrypted) key traffic may trail Ready; drain until quiet.
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(500), events.recv()).await {}
        if reconnect {
            let paused = PausedServer::new(&config);
            tokio::time::timeout(Duration::from_secs(20), async {
                while !matches!(events.recv().await, Some(RoomListEvent::Status(RoomSyncStatus::SyncFailed))) {}
            }).await.expect("sync transport failure is visible");
            drop(paused);
            tokio::time::timeout(Duration::from_secs(20), async {
                while !matches!(events.recv().await, Some(RoomListEvent::Status(RoomSyncStatus::Ready))) {}
            }).await.expect("sync reconnect resumes the same worker");
        }
        let peer_room = peer.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
        let body = "live città 🦀\npeer message after Ready";
        let sent = peer_room.send(matrix_sdk::ruma::events::room::message::RoomMessageEventContent::text_plain(body))
            .await.unwrap().response.event_id;
        let activity = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                // the new preview also upserts the summary and may reorder
                if let RoomListEvent::TimelineActivity(actual) = events.recv().await.unwrap() {
                    break actual;
                }
            }
        }).await.expect("post-initial peer timeline must notify");
        assert_eq!(activity, scope, "timeline notification must carry exact local account/room");
        let mut views = ViewRequests::new();
        let tag = views.start(scope.clone()).unwrap();
        assert!(commands.send(HistoryRequest { tag, before: None, open: false }).await.is_ok());
        let result = tokio::time::timeout(Duration::from_secs(5), history.recv()).await.unwrap().unwrap();
        assert!(views.accepts(&result.tag));
        let page = result.page.unwrap();
        let found: Vec<_> = page.events.iter().filter(|event| event.event_id.as_str() == sent.as_str()).collect();
        assert_eq!(found.len(), 1, "exact event appears once through existing history route");
        assert_eq!(found[0].body, body, "Dime decrypts actual ciphertext");
        assert!(!found[0].outgoing);
        let wire = room_request(&config, "alice", reqwest::Method::GET,
            &format!("rooms/{room}/event/{sent}"), serde_json::json!({})).await;
        assert_eq!(wire["event_id"].as_str().unwrap(), sent.as_str());
        assert_eq!(wire["room_id"], room);
        assert_eq!(wire["sender"], "@bob:matrix-desktop.test");
        if encrypted {
            assert_eq!(wire["type"], "m.room.encrypted");
            assert_eq!(wire["content"]["algorithm"], "m.megolm.v1.aes-sha2");
            assert!(wire["content"]["ciphertext"].as_str().is_some_and(|text| !text.is_empty()));
            assert!(wire["content"].get("body").is_none());
            assert!(!wire.to_string().contains(body));
        } else {
            assert_eq!(wire["type"], "m.room.message");
            assert_eq!(wire["content"]["body"], body);
        }
        for (bad, expected) in [
            (RoomScope { account_id: AccountId::new(999), ..scope.clone() }, HistoryError::WrongScope),
            (RoomScope { room_id: RoomId::new("!unjoined:matrix-desktop.test"), ..scope.clone() }, HistoryError::Unavailable),
        ] {
            let tag = views.start(bad).unwrap();
            assert!(commands.send(HistoryRequest { tag, before: None, open: false }).await.is_ok());
            let response = tokio::time::timeout(Duration::from_secs(5), history.recv()).await.unwrap().unwrap();
            assert!(matches!(response.page, Err(error) if error == expected));
        }
        let cancelled = views.start(scope.clone()).unwrap();
        views.cancel();
        assert!(commands.send(HistoryRequest { tag: cancelled, before: None, open: false }).await.is_ok());
        let tag = views.start(scope).unwrap();
        assert!(commands.send(HistoryRequest { tag, before: None, open: false }).await.is_ok());
        let valid = tokio::time::timeout(Duration::from_secs(5), history.recv()).await.unwrap().unwrap();
        assert!(views.accepts(&valid.tag), "cancelled history produces no stale result");
        assert!(valid.page.is_ok());
        assert!(tokio::time::timeout(Duration::from_millis(700), events.recv()).await.is_err(),
            "unchanged incremental sync must not repeat activity");
    });
    worker.shutdown().unwrap();
}
