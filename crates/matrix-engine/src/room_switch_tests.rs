/// Replies already seen must still be there after leaving the room and coming back, and after a restart.
/// Covers both kinds of reply: thread replies (`m.thread`) and plain replies in the main timeline (`m.in_reply_to`).
/// Run: `python3 the auth_smoke script --filter disposable_room_switch [--server tuwunel]`
/// Every step prints one `RSW_SNAP {json}` line (timeline rows, thread page, SDK store, receipts); the test fails at the end if a step lost anything seen before.
#[test]
#[cfg(unix)]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_room_switch_keeps_the_replies_already_seen() {
    use crate::{
        ViewRequests, persistence::AccountStore, room_sync::RoomSync,
    };
    use matrix_domain::{RoomId, RoomScope};
    use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
    const PASS: &str = "synthetic-passphrase-room-switch-1";
    let config = fixture();
    let url = config["url"].as_str().unwrap().to_owned();
    let account = AccountId::new(95);
    let stores = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("room-switch");
    let _ = std::fs::remove_dir_all(&stores);
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&stores).unwrap();
    runtime().block_on(async {
        let mut rooms = Vec::new();
        for _ in 0..2 {
            let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
                serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
            let room = created["room_id"].as_str().unwrap().to_owned();
            room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
            rooms.push(room);
        }
        let (a, b) = (rooms[0].clone(), rooms[1].clone());
        let put = |room: String, actor: &'static str, txn: &'static str, body: &'static str, rel: Option<serde_json::Value>| {
            let config = config.clone();
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(rel) = rel {
                    content["m.relates_to"] = rel;
                }
                room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await["event_id"]
                    .as_str().unwrap().to_owned()
            }
        };
        let root = put(a.clone(), "alice", "rsw-root", "root", None).await;
        let question = put(a.clone(), "alice", "rsw-question", "question", None).await;
        put(b.clone(), "bob", "rsw-b1", "b one", None).await;
        let thread_rel = |root: &str| serde_json::json!({"rel_type":"m.thread","event_id":root,
            "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
        let scope = |room: &str| RoomScope { account_id: account, room_id: RoomId::new(room) };
        let (scope_a, scope_b) = (scope(&a), scope(&b));
        let root_id = matrix_domain::EventId::new(root.clone());
        let vis = crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false };
        // The app's own route when a room is selected: saved rows first, then the full list.
        async fn app_open(client: &matrix_sdk::Client, timelines: &crate::room_sync::Timelines, account: AccountId, scope: &RoomScope)
            -> (Vec<crate::history::HistoryEvent>, Vec<crate::history::HistoryEvent>)
        {
            let (mut out, mut results) = crate::bounded(std::num::NonZeroUsize::new(4).unwrap());
            let mut views = ViewRequests::new(); // must outlive the request: dropping it cancels the tag
            let tag = views.start(scope.clone()).unwrap();
            let request = crate::history::HistoryRequest { tag, before: None, open: true };
            assert!(crate::room_sync::serve_history(client, timelines, account, request, &mut out).await);
            drop(out);
            let mut pages = Vec::new();
            while let Some(r) = results.recv().await {
                pages.push(r.page.map(|p| p.events).unwrap_or_default());
            }
            drop(views);
            let full = pages.pop().unwrap_or_default();
            (pages.pop().unwrap_or_default(), full)
        }
        let bodies = |rows: &[crate::history::HistoryEvent]| rows.iter().filter(|r| !r.status).map(|r| r.body.clone()).collect::<Vec<_>>();
        let mut snaps: Vec<(String, serde_json::Value)> = Vec::new();
        // One dump: main rows, thread page, SDK store (event cache) and our receipts, all of room A.
        #[allow(clippy::too_many_arguments)]
        async fn snap(
            step: &str, sync: &RoomSync, client: &matrix_sdk::Client, timelines: &crate::room_sync::Timelines,
            account: AccountId, scope: &RoomScope, root: &matrix_domain::EventId, config: &serde_json::Value,
            snaps: &mut Vec<(String, serde_json::Value)>,
        ) {
            let nonstatus = |rows: &[crate::history::HistoryEvent]| rows.iter().filter(|r| !r.status).map(|r| r.body.clone()).collect::<Vec<_>>();
            let (saved, full) = app_open(client, timelines, account, scope).await;
            let thread = sync.open_thread(scope, root, 25).await;
            let room_id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).unwrap();
            let stored: Vec<String> = crate::room_sync::stored_events(client, &room_id).await.map(|(events, hole, _)| {
                let mut v: Vec<String> = events.iter().filter_map(|e| {
                    let j: serde_json::Value = serde_json::from_str(e.raw().json().get()).ok()?;
                    let rel = j["content"]["m.relates_to"]["rel_type"].as_str().unwrap_or("-");
                    let body = j["content"]["body"].as_str()?;
                    Some(format!("{body}[{rel}]"))
                }).collect();
                v.push(format!("hole={hole}"));
                v
            }).unwrap_or_else(|e| vec![format!("stored error {e:?}")]);
            let room = client.get_room(&room_id).unwrap();
            let me = client.user_id().unwrap().to_owned();
            let mut receipts = serde_json::Map::new();
            for (kind_name, kind) in [("read", ReceiptType::Read), ("private", ReceiptType::ReadPrivate)] {
                for (thread_name, thread) in [("main", ReceiptThread::Main), ("unthreaded", ReceiptThread::Unthreaded), ("thread", ReceiptThread::Thread(matrix_sdk::ruma::EventId::parse(root.as_str()).unwrap()))] {
                    let id = room.load_user_receipt(kind.clone(), &thread, &me).await.ok().flatten().map(|(id, _)| id.to_string());
                    receipts.insert(format!("{kind_name}/{thread_name}"), serde_json::json!(id));
                }
            }
            let root_row = full.iter().find(|r| r.event_id == *root);
            let value = serde_json::json!({
                "server": config["server"], "step": step,
                "main_saved": nonstatus(&saved), "main_full": nonstatus(&full),
                "root_summary": root_row.and_then(|r| r.thread.as_ref()).map(|t| serde_json::json!({"replies": t.num_replies, "unread": t.unread})),
                "thread": thread.as_ref().ok().map(|t| serde_json::json!({
                    "root": t.root.as_ref().map(|r| r.body.clone()),
                    "replies": t.replies.iter().map(|r| r.body.clone()).collect::<Vec<_>>(),
                    "root_unread": t.root.as_ref().and_then(|r| r.thread.as_ref()).map(|i| i.unread),
                })).unwrap_or(serde_json::json!(format!("{:?}", thread.as_ref().err()))),
                "store": stored, "receipts": receipts,
            });
            println!("RSW_SNAP {value}");
            snaps.push((step.to_owned(), value));
        }

        // Run 1, online: room A open, then the replies arrive while it is open.
        let store = AccountStore::create(&stores, account).unwrap();
        let session = store
            .password_login_with_http(HomeserverUrl::parse(&url).unwrap(), "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(), PASS, http(&config))
            .await
            .unwrap();
        let mut sync = RoomSync::new(session);
        sync.sliding_round().await.unwrap();
        let (client, timelines) = sync.live_handles_for_test();
        let _ = app_open(&client, &timelines, account, &scope_a).await;
        let (pa, ra, rb) = (a.clone(), root.clone(), question.clone());
        sync.sliding_round_after(async {
            for (txn, body) in [("rsw-t1", "thread one"), ("rsw-t2", "thread two"), ("rsw-t3", "thread three")] {
                put(pa.clone(), "bob", txn, body, Some(thread_rel(&ra))).await;
            }
            for (txn, body) in [("rsw-q1", "reply one"), ("rsw-q2", "reply two")] {
                put(pa.clone(), "bob", txn, body, Some(serde_json::json!({"m.in_reply_to":{"event_id":rb}}))).await;
            }
        }).await.unwrap();
        // The replies are shown: wait until both kinds are on screen, then read them as the app does (main receipt + thread receipt).
        for _ in 0..20 {
            let (_, full) = app_open(&client, &timelines, account, &scope_a).await;
            let thread = sync.open_thread(&scope_a, &root_id, 25).await;
            if bodies(&full).contains(&"reply two".to_owned()) && thread.as_ref().is_ok_and(|t| t.replies.len() >= 3) { break; }
            let _ = sync.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        sync.open_history(ViewRequests::new().start(scope_a.clone()).unwrap(), vis).await.unwrap();
        let _ = sync.mark_thread_read(&scope_a, &root_id, false).await;
        for _ in 0..2 {
            let _ = sync.sliding_round().await;
        }
        snap("1_before_switch", &sync, &client, &timelines, account, &scope_a, &root_id, &config, &mut snaps).await;
        // Switch to B (opened and read), then back to A.
        let (_, b_rows) = app_open(&client, &timelines, account, &scope_b).await;
        sync.open_history(ViewRequests::new().start(scope_b.clone()).unwrap(), vis).await.unwrap();
        println!("RSW_B {:?}", bodies(&b_rows));
        let _ = sync.sliding_round().await;
        snap("2_back_on_A", &sync, &client, &timelines, account, &scope_a, &root_id, &config, &mut snaps).await;
        snap("2b_back_on_A_again", &sync, &client, &timelines, account, &scope_a, &root_id, &config, &mut snaps).await;
        drop((sync, client, timelines));

        // Restart on the same sealed store: first what the store alone gives, then after one sync round.
        let session = AccountStore::open(&stores, account).unwrap().restore_with_http(PASS, http(&config)).await.unwrap();
        let mut sync = RoomSync::new(session);
        let (client, timelines) = sync.live_handles_for_test();
        snap("3_restart_before_sync", &sync, &client, &timelines, account, &scope_a, &root_id, &config, &mut snaps).await;
        let _ = sync.sliding_round().await;
        snap("4_restart_after_sync", &sync, &client, &timelines, account, &scope_a, &root_id, &config, &mut snaps).await;

        // What was seen at step 1 has to be there at every later step (plus whatever arrived since).
        let mut lost = Vec::new();
        for (step, v) in &snaps {
            for key in ["main_saved", "main_full"] {
                for body in ["root", "question", "reply one", "reply two"] {
                    if !v[key].as_array().unwrap().iter().any(|x| x == body) {
                        lost.push(format!("{step}: {key} lost \"{body}\""));
                    }
                }
            }
            let replies = v["thread"]["replies"].as_array().cloned().unwrap_or_default();
            // What the panel showed before the switch is the baseline: a reply Tuwunel never delivered to the
            // thread timeline is a separate defect, not a reply that disappeared.
            let seen = snaps[0].1["thread"]["replies"].as_array().cloned().unwrap_or_default();
            for body in &seen {
                if !replies.contains(body) {
                    lost.push(format!("{step}: thread lost \"{body}\" (has {replies:?})"));
                }
            }
            if v["thread"]["root"].is_null() {
                lost.push(format!("{step}: thread root missing"));
            }
            let unread = &v["root_summary"]["unread"];
            if unread != &serde_json::json!(0) && !unread.is_null() {
                lost.push(format!("{step}: root unread {unread} after reading"));
            }
        }
        println!("RSW_LOST {}", serde_json::json!({"server": config["server"], "lost": lost}));
        let _ = std::fs::remove_dir_all(&stores);
        assert!(lost.is_empty(), "seen replies lost: {lost:#?}");
    });
}
