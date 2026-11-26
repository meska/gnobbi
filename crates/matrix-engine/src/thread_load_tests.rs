fn thread_load_probe(encrypted: bool, count: usize, n: u64) {
    use crate::{ViewRequests, room_sync::RoomSync};
    use matrix_sdk::ruma::events::room::message::{RoomMessageEventContent as Msg, Relation};
    use matrix_sdk::ruma::events::relation::Thread;
    let config = fixture();
    runtime().block_on(async {
        {
            let mut create = serde_json::json!({"invite":["@bob:matrix-desktop.test"], "preset":"private_chat"});
            if encrypted { create["initial_state"] = serde_json::json!([{"type":"m.room.encryption","state_key":"", "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]); }
            let room = room_request(&config, "alice", reqwest::Method::POST, "createRoom", create).await["room_id"].as_str().unwrap().to_owned();
            room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
            let peer = login(&config, "bob", n + 1).await.client.unwrap();
            peer.encryption().wait_for_e2ee_initialization_tasks().await;
            peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
            let mut alice = RoomSync::new(login(&config, "alice", n).await);
            alice.sliding_round().await.unwrap();
            peer.sync_once(matrix_sdk::config::SyncSettings::default()).await.unwrap();
            alice.sliding_round().await.unwrap();
            let scope = matrix_domain::RoomScope { account_id: AccountId::new(n), room_id: matrix_domain::RoomId::new(room.clone()) };
            let root = alice.send_text(&crate::sending::SendRequest { scope: scope.clone(), transaction_id: matrix_domain::TransactionId::new(format!("timing-root-{n}")), body: "root".into(), attachment: None, poll: None, mentions: Vec::new() }).await.unwrap();
            let root_ev = matrix_sdk::ruma::OwnedEventId::try_from(root.as_str().to_owned()).unwrap();
            let peer_room = peer.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
            for i in 0..count {
                let mut m = Msg::text_plain(format!("reply {i:02}"));
                m.relates_to = Some(Relation::Thread(Thread::plain(root_ev.clone(), root_ev.clone())));
                peer_room.send(m).await.unwrap();
            }
            alice.sliding_round().await.unwrap();
            alice.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false }).await.unwrap();
            for opening in 1..=2 {
                let started = std::time::Instant::now();
                let page = alice.open_thread(&scope, &root, 50).await.unwrap();
                println!("THREAD_TIME {}", serde_json::json!({"encrypted":encrypted,"count":count,"opening":opening,"ms":started.elapsed().as_millis(),"replies":page.replies.len(),"root":page.root.is_some(),"at_start":page.at_start}));
                assert!(!page.replies.is_empty());
                // A thread with fewer replies than asked for must not wait for the missing ones.
                assert!(started.elapsed().as_millis() < 250, "open_thread waited {} ms", started.elapsed().as_millis());
            }
            // The panel's first paint: the cache alone (want 0), root included, no network.
            let started = std::time::Instant::now();
            let page = alice.open_thread(&scope, &root, 0).await.unwrap();
            println!("THREAD_TIME {}", serde_json::json!({"cache_only_ms":started.elapsed().as_millis(),"replies":page.replies.len(),"root":page.root.is_some()}));
            assert!(page.root.is_some() && page.replies.len() == count.min(50) && started.elapsed().as_millis() < 100);
        }
    });
}

#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_thread_timing_plain_5() { thread_load_probe(false, 5, 200); }
#[test]
#[ignore = "requires disposable homeserver"]
fn disposable_thread_timing_plain_50() { thread_load_probe(false, 50, 202); }
