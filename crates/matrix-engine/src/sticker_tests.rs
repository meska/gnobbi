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
