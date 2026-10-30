/// P282b: thread view (`TimelineFocus::Thread`) and thread list (`ThreadListService`) through RoomSync.
/// Run: `python3 the auth_smoke script --filter disposable_thread_`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_view_and_list_through_the_timeline() {
    use crate::{ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let send = |actor: &'static str, txn: &'static str, body: &'static str, root: Option<String>| {
            let (config, room) = (config.clone(), room.clone());
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(root) = root {
                    content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,
                        "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
                }
                let sent = room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await;
                sent["event_id"].as_str().unwrap().to_owned()
            }
        };
        let root = send("alice", "thr-root", "root", None).await;
        send("bob", "thr-r1", "reply one", Some(root.clone())).await;
        send("bob", "thr-r2", "reply two", Some(root.clone())).await;
        send("alice", "thr-else", "elsewhere", None).await;
        let mut sync = RoomSync::new(login(&config, "alice", 59).await);
        sync.sliding_round().await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(59), room_id: matrix_domain::RoomId::new(room.clone()) };
        // The room's own timeline first: the root is taken from it.
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let root_id = matrix_domain::EventId::new(root.clone());
        let mut page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        for _ in 0..10 {
            if page.replies.len() >= 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        }
        let replies: Vec<_> = page.replies.iter().map(|r| r.body.as_str()).collect();
        let list = sync.thread_list(&scope).await;
        let summary = list.as_ref().ok().and_then(|l| l.iter().find(|t| t.root == root_id));
        // Probe output first: Tuwunel 1.9.4 limits are reported from here, not hidden.
        println!("P282B_THREAD {}", serde_json::json!({"replies": replies, "root_found": page.root.is_some(),
            "at_start": page.at_start, "list_ok": list.is_ok(), "list_len": list.as_ref().map(|l| l.len()).ok(),
            "num_replies": summary.map(|s| s.num_replies), "latest": summary.and_then(|s| s.latest_body.clone())}));
        assert_eq!(replies, ["reply one", "reply two"], "only the thread's events, oldest first");
        assert_eq!(page.root.as_ref().map(|r| r.body.as_str()), Some("root"), "root from the room's timeline");
        let counted = |page: &crate::threads::ThreadPage| page.root.as_ref().and_then(|r| r.thread.as_ref()).map(|t| t.num_replies);
        println!("P331_COUNT before={:?}", counted(&page));
        // A third reply arrives while the sync runs: the root's summary must follow on its own.
        sync.sliding_round_after(async { send("bob", "thr-r3", "reply three", Some(root.clone())).await; }).await.unwrap();
        let mut live = None;
        for _ in 0..10 {
            let _ = sync.sliding_round().await;
            let again = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            live = counted(&again);
            if live == Some(3) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        println!("P331_COUNT after={live:?}");
        assert_eq!(live, Some(3), "root summary counts the live reply");
        let summary = summary.expect("thread list holds the thread");
        assert_eq!(summary.num_replies, 2);
        assert_eq!(summary.root_body.as_deref(), Some("root"));
        assert_eq!(summary.latest_body.as_deref(), Some("reply two"));
    });
}

/// Thread panel: a thread longer than the sync window is read in steps, the root is always there.
/// Run: `python3 the auth_smoke script --filter disposable_thread_panel [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_long_thread_in_steps() {
    use crate::{ViewRequests, room_sync::RoomSync};
    const REPLIES: usize = 60;
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let send = |actor: &'static str, txn: String, body: String, root: Option<String>| {
            let (config, room) = (config.clone(), room.clone());
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(root) = root {
                    content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,
                        "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
                }
                let sent = room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await;
                sent["event_id"].as_str().unwrap().to_owned()
            }
        };
        let root = send("alice", "pnl-root".into(), "root".into(), None).await;
        for n in 1..=REPLIES {
            send("bob", format!("pnl-r{n}"), format!("reply {n:02}"), Some(root.clone())).await;
        }
        let mut sync = RoomSync::new(login(&config, "alice", 61).await);
        sync.sliding_round().await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(61), room_id: matrix_domain::RoomId::new(room.clone()) };
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let root_id = matrix_domain::EventId::new(root.clone());
        // The panel's loop: ask again with more room until a step brings nothing.
        let (mut want, mut steps, mut last) = (25usize, Vec::new(), 0usize);
        let page = loop {
            let page = sync.open_thread(&scope, &root_id, want).await.unwrap();
            steps.push((want, page.replies.len(), page.at_start));
            if page.replies.len() <= last || page.replies.len() >= REPLIES || want > 200 { break page; }
            last = page.replies.len();
            want += 25;
        };
        let bodies: Vec<_> = page.replies.iter().map(|r| r.body.clone()).collect();
        let mut sorted = bodies.clone();
        sorted.sort();
        sorted.dedup();
        let probe = format!("P332_LONG {}", serde_json::json!({"server": config["server"], "steps": steps,
            "replies": bodies.len(), "root": page.root.as_ref().map(|r| r.body.clone()),
            "num_replies": page.root.as_ref().and_then(|r| r.thread.as_ref()).map(|t| t.num_replies)}));
        println!("{probe}");
        assert_eq!(page.root.as_ref().map(|r| r.body.as_str()), Some("root"), "root always present");
        assert_eq!(sorted, bodies, "oldest first, no duplicates, no root among the replies");
        if config["server"] == "synapse" {
            assert_eq!(bodies.len(), REPLIES, "Synapse returns the whole thread, beyond the sync window");
        } else {
            // Tuwunel 1.9.4 may leave holes in big threads: reported, not worked around.
            assert!(bodies.len() >= 25, "at least the first window arrives");
        }
    });
}

/// Thread composer: a plain text and a reply, sent through the thread's Timeline, reach the other
/// account as `m.thread` with the right fallback. Run: `python3 the auth_smoke script --filter disposable_thread_composer [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_composer_sends_into_the_thread() {
    use crate::{ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let put = |actor: &'static str, txn: &'static str, content: serde_json::Value| {
            let (config, room) = (config.clone(), room.clone());
            async move {
                let sent = room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await;
                sent["event_id"].as_str().unwrap().to_owned()
            }
        };
        let root = put("alice", "cmp-root", serde_json::json!({"msgtype":"m.text","body":"root"})).await;
        let first = put("bob", "cmp-r1", serde_json::json!({"msgtype":"m.text","body":"reply one",
            "m.relates_to":{"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}})).await;
        let mut sync = RoomSync::new(login(&config, "alice", 62).await);
        sync.sliding_round().await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(62), room_id: matrix_domain::RoomId::new(room.clone()) };
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let (root_id, first_id) = (matrix_domain::EventId::new(root.clone()), matrix_domain::EventId::new(first.clone()));
        let mut page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        for _ in 0..10 {
            if page.replies.iter().any(|r| r.event_id == first_id) { break; }
            let _ = sync.sliding_round().await;
            page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        }
        let plain = sync.send_in_thread(&scope, &root_id, None, "plain in thread").await.unwrap();
        let answer = sync.send_in_thread(&scope, &root_id, Some(&first_id), "answer to one").await.unwrap();
        // The other account reads what the server holds.
        let seen = |id: matrix_domain::EventId| {
            let (config, room) = (config.clone(), room.clone());
            async move { room_request(&config, "bob", reqwest::Method::GET, &format!("rooms/{room}/event/{}", id.as_str()), serde_json::json!({})).await }
        };
        let (plain_event, answer_event) = (seen(plain.clone()).await, seen(answer.clone()).await);
        let (p, a) = (&plain_event["content"]["m.relates_to"], &answer_event["content"]["m.relates_to"]);
        println!("P333_RELATIONS {}", serde_json::json!({"server": config["server"], "plain": p, "answer": a}));
        assert_eq!((p["rel_type"].as_str(), p["event_id"].as_str()), (Some("m.thread"), Some(root.as_str())));
        assert_eq!(p["is_falling_back"], true, "a plain thread message carries the fallback");
        assert!(p["m.in_reply_to"]["event_id"].is_string());
        assert_eq!((a["rel_type"].as_str(), a["event_id"].as_str()), (Some("m.thread"), Some(root.as_str())));
        assert_eq!(a["m.in_reply_to"]["event_id"].as_str(), Some(first.as_str()), "send_reply points at the answered event");
        assert!(a.get("is_falling_back").is_none_or(|f| f != true), "a real reply is no fallback");
        // The thread snapshot holds each exactly once, with the ID the server gave.
        let mut page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        for _ in 0..10 {
            if page.replies.iter().any(|r| r.event_id == answer) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        }
        let ids: Vec<_> = page.replies.iter().map(|r| r.event_id.as_str().to_owned()).collect();
        for id in [&first, plain.as_str(), answer.as_str()] {
            assert_eq!(ids.iter().filter(|i| i.as_str() == id).count(), 1, "{id} appears once: {ids:?}");
        }
    });
}
