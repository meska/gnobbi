// Forward end to end on a disposable Synapse: alice forwards through `forward::forward_event`, bob (second client) receives,
// downloads and opens the media. Plain -> encrypted, encrypted -> plain, encrypted -> encrypted; file and picture.
// Run: python3 tools/auth_smoke.py --filter disposable_forward   (on the CT)

fn fwd_sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 { msg.push(0); }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 { w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap()); }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
            // rotate: v[4] must be d+t1, v[0] a = t1+t2; the array above is the standard shift
        }
        for i in 0..8 { h[i] = h[i].wrapping_add(v[i]); }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

fn fwd_png(width: u32, height: u32, mut seed: u64) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |_, _| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        image::Rgb([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A zip-shaped blob (an XLSX is a zip): fixed header, pseudo-random body.
fn fwd_xlsx(len: usize) -> Vec<u8> {
    let mut seed = 0x1234_5678_9abc_def1_u64;
    let mut out = b"PK\x03\x04".to_vec();
    while out.len() < len {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.push(seed as u8);
    }
    out
}

async fn fwd_room(config: &serde_json::Value, name: &str, encrypted: bool) -> String {
    let mut body = serde_json::json!({"visibility":"private", "name": name, "invite":["@bob:matrix-desktop.test"]});
    if encrypted {
        body["initial_state"] = serde_json::json!([{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]);
    }
    let room = room_request(config, "alice", reqwest::Method::POST, "createRoom", body).await["room_id"].as_str().unwrap().to_owned();
    room_request(config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
    room
}

/// Newest event of `room` as the server stores it (what any admin or federated server sees).
async fn fwd_server_newest(config: &serde_json::Value, room: &str) -> serde_json::Value {
    for _ in 0..30 {
        let page = room_request(config, "bob", reqwest::Method::GET, &format!("rooms/{room}/messages?dir=b&limit=1"), serde_json::json!({})).await;
        if let Some(e) = page["chunk"].as_array().and_then(|c| c.first()).filter(|e| e["type"] != "m.room.member") {
            return e.clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("nothing arrived in {room}");
}

/// Bob's view: the decrypted content of `event_id`, retried while his keys arrive.
async fn fwd_bob_content(bob: &matrix_sdk::Client, room: &str, event_id: &str) -> serde_json::Value {
    let room = matrix_sdk::ruma::RoomId::parse(room).unwrap();
    let id = matrix_sdk::ruma::EventId::parse(event_id).unwrap();
    let mut last = String::from("room unknown");
    for _ in 0..20 {
        let _ = crate::room_sync::RoomSync::sliding_catch_up(bob).await;
        // A second, classic round only for this test client: to-device room keys reach it for sure.
        let _ = bob.sync_once(matrix_sdk::config::SyncSettings::default().timeout(std::time::Duration::from_millis(500))).await;
        if let Some(room) = bob.get_room(&room) {
            match room.event(&id, None).await {
                Ok(event) if event.kind.is_utd() => last = "undecryptable".into(),
                Ok(event) => {
                    let raw: serde_json::Value = serde_json::from_str(event.raw().json().get()).unwrap();
                    return raw["content"].clone();
                }
                Err(e) => last = format!("{e:?}"),
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    println!("BOB last state: {last}");
    panic!("bob could not read {event_id}");
}

async fn fwd_bob_download(bob: &matrix_sdk::Client, content: &serde_json::Value) -> Vec<u8> {
    use matrix_sdk::{media::{MediaFormat, MediaRequestParameters}, ruma::events::room::MediaSource};
    let source: MediaSource = if let Some(url) = content["url"].as_str() {
        MediaSource::Plain(url.into())
    } else {
        MediaSource::Encrypted(Box::new(serde_json::from_value(content["file"].clone()).expect("EncryptedFile")))
    };
    bob.media().get_media_content(&MediaRequestParameters { source, format: MediaFormat::File }, false).await.expect("bob downloads")
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; run with tools/auth_smoke.py"]
fn disposable_forward_file_and_picture_between_plain_and_encrypted_rooms() {
    use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
    use matrix_sdk_ui::timeline::AttachmentConfig;
    let config = fixture();
    runtime().block_on(async {
        let plain = fwd_room(&config, "fwd-plain", false).await;
        let enc1 = fwd_room(&config, "fwd-enc-1", true).await;
        let enc2 = fwd_room(&config, "fwd-enc-2", true).await;
        let rooms = [("plain", &plain), ("enc1", &enc1), ("enc2", &enc2)];
        let alice_session = login(&config, "alice", 71).await;
        let sync = crate::room_sync::RoomSync::new(alice_session);
        let (alice, timelines) = sync.live_handles_for_test();
        let bob_session = login(&config, "bob", 72).await;
        let bob = bob_session.client.clone().unwrap();
        bob.encryption().wait_for_e2ee_initialization_tasks().await;
        alice.encryption().wait_for_e2ee_initialization_tasks().await;
        // Same order as `encrypted_pair`: each side syncs twice so alice learns bob's device before she sends.
        for _ in 0..2 {
            bob.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        }

        let files: [(&str, Vec<u8>, &str); 2] = [
            ("report.xlsx", fwd_xlsx(48 * 1024), "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            ("picture.png", fwd_png(160, 120, 0x9e37_79b9_7f4a_7c15), "image/png"),
        ];
        let sdk_room = |id: &str| alice.get_room(&matrix_sdk::ruma::RoomId::parse(id).unwrap()).expect("alice knows the room");
        // Source events: each file and picture sent by alice into the plain room and into the first encrypted one.
        let mut sources = std::collections::BTreeMap::new();
        for (label, room) in [("plain", &plain), ("enc1", &enc1)] {
            for (name, bytes, mime) in &files {
                let room_sdk = sdk_room(room);
                let timeline = timelines.get(&room_sdk).await.expect("timeline");
                let event = crate::sending::queue_attachment_waiting(&alice, &room_sdk, &timeline, (name.to_string(), bytes.clone()), mime.parse().unwrap(), AttachmentConfig::default(), None)
                    .await.expect("source sent");
                sources.insert((label, *name), event.as_str().to_owned());
            }
        }
        let mut failures: Vec<String> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = sources.values().cloned().collect();
        let flows = [("plain", "enc1"), ("enc1", "plain"), ("enc1", "enc2")];
        for (from, to) in flows {
            for (name, bytes, _) in &files {
                let source_room = rooms.iter().find(|r| r.0 == from).unwrap().1;
                let dest_room = rooms.iter().find(|r| r.0 == to).unwrap().1;
                let scope = RoomScope { account_id: AccountId::new(71), room_id: RoomId::new(source_room) };
                let target = EventId::new(&sources[&(from, *name)]);
                let mut results = Vec::new();
                for _ in 0..10 {
                    results = crate::forward::forward_event(&alice, &timelines, AccountId::new(71), &scope, &target, &[dest_room.clone()]).await;
                    if results[0].1.is_ok() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                let flow = format!("{from}->{to} {name}");
                if results[0].1.is_err() { failures.push(format!("{flow}: forward returned {:?}", results[0].1)); continue; }
                // The send queue sends in the background: wait for the newest event to be the forwarded one.
                let mut server = fwd_server_newest(&config, dest_room).await;
                for _ in 0..40 {
                    if server["sender"] == "@alice:matrix-desktop.test" && server["event_id"].as_str().is_some_and(|e| !seen.contains(e)) { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    server = fwd_server_newest(&config, dest_room).await;
                }
                let event_id = server["event_id"].as_str().unwrap().to_owned();
                if !seen.insert(event_id.clone()) { failures.push(format!("{flow}: no new event arrived")); continue; }
                let server_content = &server["content"];
                println!("FWD {flow} SERVER type={} content_keys={:?}", server["type"], server_content.as_object().unwrap().keys().collect::<Vec<_>>());
                if to != "plain" {
                    if server["type"] != "m.room.encrypted" { failures.push(format!("{flow}: server type {}", server["type"])); }
                    if server_content.get("url").is_some() { failures.push(format!("{flow}: `url` in the clear on the server")); }
                    if server_content.get("m.relates_to").is_some() { failures.push(format!("{flow}: m.relates_to on the server")); }
                }
                let content = fwd_bob_content(&bob, dest_room, &event_id).await;
                println!("FWD {flow} BOB msgtype={} keys={:?}", content["msgtype"], content.as_object().unwrap().keys().collect::<Vec<_>>());
                if to != "plain" {
                    if content.get("url").is_some() { failures.push(format!("{flow}: decrypted content has a plain `url`")); }
                    if content.get("m.relates_to").is_some() { failures.push(format!("{flow}: decrypted content has m.relates_to")); }
                    if content["file"]["url"].is_null() { failures.push(format!("{flow}: no encrypted `file`")); }
                } else if content["file"]["key"].is_object() {
                    println!("FWD NOTE {flow}: the file key {{k,..}} travels in a PLAIN room event (server can decrypt that media); same exposure as the plain room's own text");
                }
                let got = fwd_bob_download(&bob, &content).await;
                let (want_hash, got_hash) = (fwd_sha256(bytes), fwd_sha256(&got));
                println!("FWD {flow} SHA256 original={want_hash} received={got_hash} len={}/{}", bytes.len(), got.len());
                if want_hash != got_hash { failures.push(format!("{flow}: hash differs")); }
            }
        }
        assert!(failures.is_empty(), "forward findings:\n{}", failures.join("\n"));
    });
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable Synapse; run with tools/auth_smoke.py"]
fn disposable_forward_edge_cases_reply_thread_edit_location_sticker() {
    use matrix_domain::{AccountId, EventId, RoomId, RoomScope};
    let config = fixture();
    runtime().block_on(async { tokio::time::timeout(std::time::Duration::from_secs(200), async {
        let plain = fwd_room(&config, "fwd-edge-plain", false).await;
        let enc = fwd_room(&config, "fwd-edge-enc", true).await;
        let sync = crate::room_sync::RoomSync::new(login(&config, "alice", 73).await);
        let (alice, timelines) = sync.live_handles_for_test();
        let bob = login(&config, "bob", 74).await.client.clone().unwrap();
        bob.encryption().wait_for_e2ee_initialization_tasks().await;
        alice.encryption().wait_for_e2ee_initialization_tasks().await;
        for _ in 0..2 {
            bob.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
            crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        }
        let put = |txn: &'static str, kind: &'static str, content: serde_json::Value| {
            let (config, plain) = (config.clone(), plain.clone());
            async move {
                room_request(&config, "alice", reqwest::Method::PUT, &format!("rooms/{plain}/send/{kind}/{txn}"), content).await["event_id"].as_str().unwrap().to_owned()
            }
        };
        println!("EDGE e2ee ready");
        let root = put("e-root", "m.room.message", serde_json::json!({"msgtype":"m.text","body":"thread root"})).await;
        let reply = put("e-reply", "m.room.message", serde_json::json!({"msgtype":"m.text","body":"> <@alice:matrix-desktop.test> thread root\n\nthe reply text",
            "m.relates_to":{"m.in_reply_to":{"event_id":root}}})).await;
        let thread = put("e-thread", "m.room.message", serde_json::json!({"msgtype":"m.text","body":"in the thread",
            "m.relates_to":{"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}})).await;
        let original = put("e-orig", "m.room.message", serde_json::json!({"msgtype":"m.text","body":"before edit"})).await;
        put("e-edit", "m.room.message", serde_json::json!({"msgtype":"m.text","body":"* after edit","m.new_content":{"msgtype":"m.text","body":"after edit"},
            "m.relates_to":{"rel_type":"m.replace","event_id":original}})).await;
        let location = put("e-loc", "m.room.message", serde_json::json!({"msgtype":"m.location","body":"Padova","geo_uri":"geo:45.4064,11.8768",
            "info":{"thumbnail_url":"mxc://matrix-desktop.test/plainthumb"}})).await;
        let sticker = put("e-stk", "m.sticker", serde_json::json!({"body":"wave","url":"mxc://matrix-desktop.test/plainsticker","info":{"mimetype":"image/png","w":64,"h":64}})).await;
        crate::room_sync::RoomSync::sliding_catch_up(&alice).await.unwrap();
        // The user opens the room first (the sliding window only brings the newest events): that fills the timeline the forward reads.
        let scope0 = RoomScope { account_id: AccountId::new(73), room_id: RoomId::new(&plain) };
        let opened = sync.open_history(crate::ViewRequests::new().start(scope0).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        println!("EDGE setup done, room opened with {} events", opened.0.events.len());
        let mut failures: Vec<String> = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (name, source, dest, want) in [
            ("reply", &reply, &enc, "the reply text"), ("thread", &thread, &enc, "in the thread"), ("edit", &original, &enc, "after edit"),
            ("location", &location, &enc, "geo:45.4064,11.8768"), ("sticker", &sticker, &enc, "plainsticker"),
            ("reply->plain", &reply, &plain, "the reply text"),
        ] {
            let scope = RoomScope { account_id: AccountId::new(73), room_id: RoomId::new(&plain) };
            let mut results = Vec::new();
            for _ in 0..2 {
                results = crate::forward::forward_event(&alice, &timelines, AccountId::new(73), &scope, &EventId::new(source), &[dest.clone()]).await;
                if results[0].1.is_ok() { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            println!("EDGE {name} forward result {:?}", results[0].1);
            if results[0].1.is_err() { failures.push(format!("{name}: forward returned {:?}", results[0].1)); continue; }
            let mut server = fwd_server_newest(&config, dest).await;
            for _ in 0..40 {
                if server["sender"] == "@alice:matrix-desktop.test" && server["event_id"].as_str().is_some_and(|e| !seen.contains(e) && ![&root, &reply, &thread, &original, &location, &sticker].iter().any(|s| *s == e)) { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                server = fwd_server_newest(&config, dest).await;
            }
            let id = server["event_id"].as_str().unwrap().to_owned();
            if !seen.insert(id.clone()) { failures.push(format!("{name}: no new event")); continue; }
            let content = fwd_bob_content(&bob, dest, &id).await;
            println!("EDGE {name} type={} content={}", server["type"], content.to_string().chars().take(420).collect::<String>());
            if content.get("m.relates_to").is_some() { failures.push(format!("{name}: m.relates_to present")); }
            if !content.to_string().contains(want) { failures.push(format!("{name}: `{want}` missing")); }
            if content["body"].as_str().is_some_and(|b| b.contains("thread root") && name != "location") { failures.push(format!("{name}: reply quote left in body")); }
            if name == "location" && content["info"].get("thumbnail_url").is_some() { failures.push("location: plain thumbnail_url sent into an encrypted room".into()); }
            if name == "sticker" { println!("EDGE sticker NOTE: url in decrypted content = {} (stickers keep a plain mxc, as Element does)", content["url"]); }
        }
        assert!(failures.is_empty(), "edge findings:\n{}", failures.join("\n"));
    }).await.expect("edge test deadline (200 s): see the EDGE lines above"); });
}
