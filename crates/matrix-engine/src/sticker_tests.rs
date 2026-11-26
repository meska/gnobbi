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

#[test]
#[ignore = "requires disposable Synapse: STICKER_URL, users alice and bob with STICKER_PASSWORD"]
fn disposable_sticker_library_never_shows_in_the_room_list_but_others_do() {
    use crate::{RoomListDiff, room_sync::RoomSync, stickers::{PackEdit, edit, forget_made}};
    use matrix_sdk::config::SyncSettings;
    let (url, password) = (std::env::var("STICKER_URL").unwrap(), std::env::var("STICKER_PASSWORD").unwrap());
    runtime().block_on(async {
        let login = |who: &'static str| {
            let (url, password) = (url.clone(), password.clone());
            async move {
                let c = matrix_sdk::Client::builder().homeserver_url(url).build().await.unwrap();
                c.matrix_auth().login_username(who, &password).send().await.unwrap();
                c
            }
        };
        let sync = |c: &matrix_sdk::Client| {
            let c = c.clone();
            async move { c.sync_once(SyncSettings::default().timeout(std::time::Duration::from_millis(500))).await.unwrap(); }
        };
        let (alice, bob) = (login("alice").await, login("bob").await);
        let tag = std::process::id();
        // bob's room with a sticker pack, alice joined and enabled it; alice's plain room.
        let pack = serde_json::json!({"images": {}, "pack": {"display_name": "Di bob", "usage": ["sticker"]}});
        let mut request = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
        request.name = Some(format!("bob-pack-{tag}"));
        request.initial_state = vec![matrix_sdk::ruma::serde::Raw::from_json(serde_json::value::to_raw_value(&serde_json::json!({"type": "m.room.image_pack", "state_key": "b", "content": pack})).unwrap())];
        request.invite = vec![alice.user_id().unwrap().to_owned()];
        let shared = bob.create_room(request).await.unwrap();
        alice.join_room_by_id(shared.room_id()).await.unwrap();
        let mut plain = matrix_sdk::ruma::api::client::room::create_room::v3::Request::new();
        plain.name = Some(format!("plain-{tag}"));
        let plain = alice.create_room(plain).await.unwrap();
        sync(&alice).await;
        edit(&alice, PackEdit::Enable { pack: format!("{}/b", shared.room_id()), on: true }).await.unwrap();
        let ready = crate::stickers::prepare(64, 64, &[200u8; 64 * 64 * 4], false).unwrap();
        edit(&alice, PackEdit::Add { pack: None, default_name: "Miei".into(), shortcode: "uno".into(), png: ready.png, side: ready.side }).await.unwrap();
        // The library is the enabled room alice created, as `library` finds it.
        let data = alice.account().fetch_account_data(matrix_sdk::ruma::events::GlobalAccountDataEventType::from("m.image_pack.rooms")).await.unwrap().unwrap();
        let data: serde_json::Value = serde_json::from_str(data.json().get()).unwrap();
        let library = data["rooms"].as_object().unwrap().keys().find(|k| *k != shared.room_id().as_str()).cloned().expect("library enabled");

        let listed = |diffs: Vec<RoomListDiff>| -> Vec<String> { diffs.into_iter().filter_map(|d| match d { RoomListDiff::Upsert(r) => Some(r.scope.room_id.as_str().to_owned()), _ => None }).collect() };
        let mut room_sync = RoomSync::from_client(matrix_domain::AccountId::new(1), alice.clone());
        // Right after the creation, before any sync brought its state or the account data.
        let early = listed(room_sync.saved_batch().await.unwrap());
        assert!(!early.contains(&library), "library flashes before the sync: {early:?}");
        sync(&alice).await;
        let after = listed(room_sync.saved_batch().await.unwrap());
        let all: Vec<_> = early.iter().chain(&after).collect();
        assert!(!all.iter().any(|id| **id == library), "library listed: {all:?}");
        assert!(all.iter().any(|id| **id == plain.room_id().as_str()) && all.iter().any(|id| **id == shared.room_id().as_str()), "normal rooms missing: {all:?}");
        // A restart: no memory of the creation, only the synced account data and create event.
        forget_made();
        let again = login("alice").await;
        sync(&again).await;
        let fresh = listed(RoomSync::from_client(matrix_domain::AccountId::new(1), again).saved_batch().await.unwrap());
        assert!(!fresh.contains(&library) && fresh.iter().any(|id| id == plain.room_id().as_str()) && fresh.iter().any(|id| id == shared.room_id().as_str()), "{fresh:?}");
        eprintln!("STICKER LIBRARY HIDDEN ok");
    });
}

#[test]
#[ignore = "requires disposable Synapse"]
fn disposable_gif_goes_out_animated_and_joins_the_stickers_with_its_mxc() {
    use crate::{sending::{Attachment, SendRequest, fresh_transaction_id, send_text, tests::two_frame_gif}, stickers::{PackEdit, edit, packs, send}};
    runtime().block_on(async {
        let config = fixture();
        let rooms_path = "user/@alice:matrix-desktop.test/account_data/m.image_pack.rooms";
        room_request(&config, "alice", reqwest::Method::PUT, rooms_path, serde_json::json!({"rooms": {}})).await;
        let alice = login(&config, "alice", 302).await;
        let account = alice.account_id();
        let client = alice.client.unwrap();
        let made = room_request(&config, "alice", reqwest::Method::POST, "createRoom", serde_json::json!({"name": "gif-room", "visibility": "private"})).await;
        let room = made["room_id"].as_str().unwrap().to_owned();
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let scope = RoomScope { account_id: account, room_id: matrix_domain::RoomId::new(&room) };
        let gif = two_frame_gif(400, 300);
        let request = SendRequest { scope: scope.clone(), transaction_id: fresh_transaction_id(), body: String::new(), attachment: Some(Attachment { filename: "gatto felice.gif".into(), data: gif.clone(), voice: None, ..Default::default() }), poll: None, mentions: Vec::new() };
        let sent = send_text(&client, account, &request).await.expect("gif sent");
        let event = |id: String| { let (config, room) = (config.clone(), room.clone()); async move { room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{room}/event/{id}"), serde_json::json!({})).await } };
        let raw = event(sent.as_str().to_owned()).await;
        let content = &raw["content"];
        let url = content["url"].as_str().unwrap().to_owned();
        println!("GIF_EVENT {}", serde_json::json!({"msgtype": content["msgtype"], "url": url, "info": content["info"]}));
        assert_eq!((content["msgtype"].as_str(), content["info"]["mimetype"].as_str()), (Some("m.image"), Some("image/gif")));
        assert_eq!((content["info"]["w"].as_u64(), content["info"]["h"].as_u64(), content["info"]["size"].as_u64()), (Some(400), Some(300), Some(gif.len() as u64)));
        assert_eq!((content["info"]["thumbnail_info"]["mimetype"].as_str(), content["info"]["thumbnail_info"]["w"].as_u64()), (Some("image/png"), Some(320)));
        assert!(content["info"]["thumbnail_url"].as_str().is_some_and(|u| u.starts_with("mxc://") && u != url));
        // The server holds the original bytes, unchanged.
        let stored = client.media().get_media_content(&matrix_sdk::media::MediaRequestParameters { source: matrix_sdk::ruma::events::room::MediaSource::Plain(url.as_str().into()), format: matrix_sdk::media::MediaFormat::File }, false).await.unwrap();
        assert_eq!(stored, gif);
        // «Add to my stickers»: same mxc and mimetype, no new upload; then sent as a sticker.
        edit(&client, PackEdit::AddMessage { pack: None, default_name: "Miei".into(), room: room.clone(), event: sent.as_str().to_owned() }).await.expect("gif added");
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let listed = packs(&client, account, &scope).await.unwrap();
        let sticker = listed.iter().flat_map(|p| &p.stickers).find(|s| s.shortcode == "gatto_felice").expect("sticker in the pack").clone();
        assert_eq!((sticker.url.as_str(), sticker.mimetype.as_deref(), sticker.width, sticker.size), (url.as_str(), Some("image/gif"), Some(400), Some(gif.len() as u32)));
        let as_sticker = send(&client, account, &scope, &sticker).await.expect("sticker sent");
        let raw = event(as_sticker.as_str().to_owned()).await;
        println!("GIF_STICKER {}", serde_json::json!({"type": raw["type"], "url": raw["content"]["url"], "info": raw["content"]["info"]}));
        assert_eq!((raw["type"].as_str(), raw["content"]["url"].as_str(), raw["content"]["info"]["mimetype"].as_str()), (Some("m.sticker"), Some(url.as_str()), Some("image/gif")));
        // The room timeline (/messages) shows both with the same mxc.
        let messages = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{room}/messages?dir=b&limit=10"), serde_json::json!({})).await;
        let urls: Vec<_> = messages["chunk"].as_array().unwrap().iter().filter(|e| e["type"] == "m.sticker" || e["content"]["msgtype"] == "m.image").map(|e| (e["type"].as_str().unwrap().to_owned(), e["content"]["url"].as_str().unwrap().to_owned(), e["content"]["info"]["mimetype"].as_str().unwrap().to_owned())).collect();
        println!("GIF_MESSAGES {urls:?}");
        assert!(urls.len() == 2 && urls.iter().all(|(_, u, m)| u == &url && m == "image/gif"), "{urls:?}");
    });
}
