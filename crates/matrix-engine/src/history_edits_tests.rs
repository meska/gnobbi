// Real standard Matrix relations; all payloads are disposable fixtures.
#[test]
#[ignore = "requires isolated homeserver; needs the external test harness (not in this repo)"]
fn disposable_history_edits_preserve_paginated_identity() {
    use crate::{ViewRequests, room_sync::RoomSync};
    use matrix_domain::{RoomId, RoomScope};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"pages","visibility":"private",
                "invite":["@bob:matrix-desktop.test"]}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap();
        room_request(
            &config,
            "bob",
            reqwest::Method::POST,
            &format!("join/{room}"),
            serde_json::json!({}),
        )
        .await;
        let other = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"name":"other scope","visibility":"private"}),
        )
        .await;
        let other_room = other["room_id"].as_str().unwrap();
        let mut ids = Vec::new();
        for i in 0..30 {
            let sent = room_request(
                &config,
                "alice",
                reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/original-{i}"),
                serde_json::json!({"msgtype":"m.text","body":format!("Originale città 🦀 {i:02}")}),
            )
            .await;
            ids.push(sent["event_id"].as_str().unwrap().to_owned());
        }
        let body = "Modificato città 🦀\n第二行";
        let mut edit_id = String::new();
        for (transaction, text) in [("older-edit", "Modifica vecchia"), ("latest-edit", body)] {
            let sent = room_request(
                &config,
                "alice",
                reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/{transaction}"),
                serde_json::json!({"msgtype":"m.text","body":format!("* {text}"),
                    "m.new_content":{"msgtype":"m.text","body":text},
                    "m.relates_to":{"rel_type":"m.replace","event_id":ids[0]}}),
            )
            .await;
            edit_id = sent["event_id"].as_str().unwrap().to_owned();
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        // More than one relation page of newer, unauthorized replacements.
        for i in 0..26 {
            room_request(
                &config,
                "bob",
                reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/foreign-{i}"),
                serde_json::json!({"msgtype":"m.text","body":"* Foreign sender",
                    "m.new_content":{"msgtype":"m.text","body":"Foreign sender"},
                    "m.relates_to":{"rel_type":"m.replace","event_id":ids[0]}}),
            )
            .await;
        }
        let cross_room = http(&config)
            .put(format!(
                "{}_matrix/client/v3/rooms/{other_room}/send/m.room.message/cross-room",
                config["url"].as_str().unwrap()
            ))
            .bearer_auth(config["alice_token"].as_str().unwrap())
            .header("Content-Type", "application/json")
            .body(
                serde_json::json!({"msgtype":"m.text","body":"* Foreign room",
                "m.new_content":{"msgtype":"m.text","body":"Foreign room"},
                "m.relates_to":{"rel_type":"m.replace","event_id":ids[0]}})
                .to_string(),
            )
            .send()
            .await
            .unwrap();
        let cross_room_status = cross_room.status().as_u16();
        let cross_room_response: serde_json::Value =
            serde_json::from_str(&cross_room.text().await.unwrap()).unwrap();
        println!(
            "cross-room status={cross_room_status} errcode={}",
            cross_room_response["errcode"]
        );
        assert!(
            cross_room_status == 200
                || (cross_room_status == 400 && cross_room_response["errcode"] == "M_UNKNOWN")
        );
        let original = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{}", ids[0]),
            serde_json::json!({}),
        )
        .await;
        let replacement = room_request(
            &config,
            "alice",
            reqwest::Method::GET,
            &format!("rooms/{room}/event/{edit_id}"),
            serde_json::json!({}),
        )
        .await;
        assert_ne!(ids[0], edit_id);
        assert_eq!(original["event_id"], ids[0]);
        assert_eq!(original["room_id"], room);
        assert_eq!(original["content"]["body"], "Originale città 🦀 00");
        assert_eq!(replacement["event_id"], edit_id);
        assert_eq!(replacement["room_id"], room);
        assert_eq!(replacement["sender"], original["sender"]);
        assert_eq!(replacement["content"]["m.new_content"]["body"], body);
        assert_eq!(replacement["content"]["m.relates_to"]["event_id"], ids[0]);
        let mut sync = RoomSync::new(login(&config, "alice", 51).await);
        sync.sliding_round().await.unwrap();
        let mut views = ViewRequests::new();
        let tag = views
            .start(RoomScope {
                account_id: AccountId::new(51),
                room_id: RoomId::new(room),
            })
            .unwrap();
        let mut cursor = None;
        let mut collected = Vec::new();
        let mut pages = 0;
        for _ in 0..10 {
            let page = sync
                .history_page(tag.clone(), cursor.clone())
                .await
                .unwrap();
            let mut older = page.events;
            older.append(&mut collected);
            collected = older;
            pages += 1;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert!(cursor.is_none(), "reached original history start");
        assert!(pages >= 3, "edits and original span multiple history pages");
        assert_eq!(collected.len(), ids.len());
        for (i, row) in collected.iter().enumerate() {
            assert!(
                row.event_id.as_str() == ids[i],
                "original identity/order retained"
            );
            assert_eq!(
                row.body,
                if i == 0 {
                    body.into()
                } else {
                    format!("Originale città 🦀 {i:02}")
                }
            );
            assert!(row.outgoing);
        }
        assert_eq!(
            collected[0].timestamp_ms,
            original["origin_server_ts"].as_u64().unwrap()
        );
        println!(
            "EDIT_EVENT_EVIDENCE {}",
            serde_json::json!({
                "room_id":room, "original_id":ids[0], "replacement_id":edit_id,
                "original_body":original["content"]["body"], "replacement_body":body,
                "bundled":original["unsigned"]["m.relations"]["m.replace"].is_object(),
                "projected_ids":ids, "history_pages":pages, "foreign_sender_edits":26
            })
        );
    });
}
