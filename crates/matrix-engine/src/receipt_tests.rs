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
