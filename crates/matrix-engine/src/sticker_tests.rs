#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_sticker_pack_is_listed_and_sent_with_the_plain_url_in_plain_and_encrypted_rooms() {
    use crate::stickers::{Sticker, packs, send};
    runtime().block_on(async {
        let config = fixture();
        let alice = login(&config, "alice", 300).await;
        let account = alice.account_id();
        let client = alice.client.unwrap();
        let pack = serde_json::json!({"images": {"ciao": {"url": "mxc://matrix-desktop.test/ciaoSticker1", "body": "Ciao", "info": {"mimetype": "image/png", "w": 64, "h": 64}}},
            "pack": {"display_name": "Prova", "usage": ["sticker"]}});
        let encryption = serde_json::json!({"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}});
        let mut rooms = Vec::new();
        for (name, encrypted) in [("sticker-plain", false), ("sticker-enc", true)] {
            let mut state = vec![serde_json::json!({"type": "m.room.image_pack", "state_key": "p1", "content": pack})];
            if encrypted { state.push(encryption.clone()); }
            let made = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": name, "visibility": "private", "initial_state": state})).await;
            rooms.push(made["room_id"].as_str().unwrap().to_owned());
        }
        // The plain room's pack is also enabled for the account (`m.image_pack.rooms`), so it must be listed once.
        room_request(&config, "alice", reqwest::Method::PUT, "user/@alice:matrix-desktop.test/account_data/m.image_pack.rooms",
            serde_json::json!({"rooms": {rooms[0].clone(): {"p1": {}}}})).await;
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        for (i, id) in rooms.iter().enumerate() {
            let scope = RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(id) };
            let listed = packs(&client, account, &scope).await.unwrap();
            // Only the pack enabled by the account (the plain room's) is listed, in both rooms; the other room's own pack is not synced.
            assert_eq!(listed.len(), 1, "room {i}: {:?}", listed.iter().map(|p| &p.id).collect::<Vec<_>>());
            let sticker: &Sticker = &listed[0].stickers[0];
            assert_eq!((listed[0].name.as_str(), sticker.body.as_str(), sticker.mimetype.as_deref()), ("Prova", "Ciao", Some("image/png")));
            let event = send(&client, account, &scope, sticker).await.expect("sticker sent");
            let raw = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{id}/event/{}", event.as_str()), serde_json::json!({})).await;
            if i == 0 {
                assert_eq!(raw["type"], "m.sticker");
                assert_eq!(raw["content"]["url"], "mxc://matrix-desktop.test/ciaoSticker1");
                assert!(raw["content"].get("file").is_none());
            } else {
                // Megolm hides the event; the url inside is the pack's, no new upload happened.
                assert_eq!(raw["type"], "m.room.encrypted");
            }
        }
        eprintln!("STICKER ok");
    });
}

#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_sticker_made_from_a_picture_is_a_synced_pack_and_reaches_a_second_account() {
    use crate::stickers::{PackEdit, edit, packs, prepare, send};
    runtime().block_on(async {
        let config = fixture();
        let rooms_path = "user/@alice:matrix-desktop.test/account_data/m.image_pack.rooms";
        room_request(&config, "alice", reqwest::Method::PUT, rooms_path, serde_json::json!({"rooms": {}})).await;
        let alice = login(&config, "alice", 301).await;
        let account = alice.account_id();
        let client = alice.client.unwrap();
        let made = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "sticker-bob", "visibility": "private", "invite": ["@bob:matrix-desktop.test"]})).await;
        let room = made["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("rooms/{room}/join"), serde_json::json!({})).await;
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let scope = RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(&room) };
        for code in ["gatto", "cane"] {
            let ready = prepare(300, 200, &[200u8; 300 * 200 * 4], false).unwrap();
            edit(&client, PackEdit::Add { pack: None, default_name: "Miei".into(), shortcode: code.into(), png: ready.png, side: ready.side }).await.expect("sticker added");
            crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        }
        // One library, one pack, both stickers: the second add found the pack of the first.
        let listed = packs(&client, account, &scope).await.unwrap();
        assert_eq!(listed.len(), 1, "{listed:?}");
        let pack = &listed[0];
        assert_eq!((pack.name.as_str(), pack.own, pack.enabled, pack.stickers.len()), ("Miei", true, true, 2));
        let (library, key) = pack.id.split_once('/').unwrap();
        let data = room_request(&config, "alice", reqwest::Method::GET, rooms_path, serde_json::json!({})).await;
        assert!(data["rooms"][library][key].is_object(), "{data}");
        let state = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{library}/state/m.room.image_pack/{key}"), serde_json::json!({})).await;
        assert_eq!((state["pack"]["usage"][0].as_str(), state["images"]["gatto"]["info"]["w"].as_u64()), (Some("sticker"), Some(300)));
        // Bob, in the chat room only, reads the sticker event with the pack's url.
        let sticker = pack.stickers.iter().find(|s| s.shortcode == "gatto").unwrap();
        let event = send(&client, account, &scope, sticker).await.expect("sticker sent");
        let raw = room_request(&config, "bob", reqwest::Method::GET, &format!("rooms/{room}/event/{}", event.as_str()), serde_json::json!({})).await;
        assert_eq!((raw["type"].as_str(), raw["content"]["url"].as_str(), raw["content"]["body"].as_str()), (Some("m.sticker"), Some(sticker.url.as_str()), Some("gatto")));
        assert_eq!((raw["content"]["info"]["w"].as_u64(), raw["content"]["info"]["mimetype"].as_str()), (Some(300), Some("image/png")));
        // A pack of another room is imported by room id; a room that cannot be read is refused.
        let other = serde_json::json!({"images": {"ciao": {"url": sticker.url}}, "pack": {"display_name": "Altro"}});
        let shared = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "sticker-altro", "initial_state": [{"type": "m.room.image_pack", "state_key": "x", "content": other}]})).await;
        let hidden = room_request(&config, "bob", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "sticker-bob-solo"})).await;
        edit(&client, PackEdit::Import { room: shared["room_id"].as_str().unwrap().into() }).await.expect("imported");
        assert_eq!(edit(&client, PackEdit::Import { room: hidden["room_id"].as_str().unwrap().into() }).await, Err(crate::operations::OperationError::Forbidden));
        // Second pack, move, rename, remove, disable, delete.
        edit(&client, PackEdit::Create { name: "Secondo".into() }).await.expect("created");
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let listed = packs(&client, account, &scope).await.unwrap();
        assert_eq!(listed.len(), 3, "{listed:?}");
        let second = listed.iter().find(|p| p.name == "Secondo").unwrap().id.clone();
        assert!(second.starts_with(library), "the second pack is in the same library room");
        edit(&client, PackEdit::Move { pack: pack.id.clone(), shortcode: "cane".into(), to: second.clone() }).await.expect("moved");
        edit(&client, PackEdit::Rename { pack: second.clone(), name: "Animali".into() }).await.expect("renamed");
        edit(&client, PackEdit::Remove { pack: pack.id.clone(), shortcode: "gatto".into() }).await.expect("removed");
        edit(&client, PackEdit::Enable { pack: format!("{}/x", shared["room_id"].as_str().unwrap()), on: false }).await.expect("disabled");
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let listed = packs(&client, account, &scope).await.unwrap();
        let shape: Vec<_> = listed.iter().map(|p| (p.name.as_str(), p.stickers.iter().map(|s| s.shortcode.as_str()).collect::<Vec<_>>())).collect();
        assert_eq!(shape, [("Miei", vec![]), ("Animali", vec!["cane"])]);
        edit(&client, PackEdit::Delete { pack: second }).await.expect("deleted");
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        assert_eq!(packs(&client, account, &scope).await.unwrap().len(), 1);
        eprintln!("STICKER PACKS ok");
    });
}
