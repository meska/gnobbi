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
        let plain = sync.send_in_thread(&scope, &root_id, None, "plain in thread", None).await.unwrap();
        let answer = sync.send_in_thread(&scope, &root_id, Some(&first_id), "answer to one", None).await.unwrap();
        // A voice note takes the attachment path: still an event of the thread.
        let note = crate::sending::Attachment { filename: "note.ogg".into(), data: b"OggS not really audio".to_vec(), voice: Some(crate::sending::Voice { duration_ms: 1200, waveform: vec![0.1, 0.9] }) };
        let voice = sync.send_in_thread(&scope, &root_id, None, "", Some(&note)).await.unwrap();
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
        let voice_event = seen(voice.clone()).await;
        let v = &voice_event["content"];
        println!("THREAD_VOICE {}", serde_json::json!({"msgtype": v["msgtype"], "relates": v["m.relates_to"], "voice": v.get("org.matrix.msc3245.voice").is_some()}));
        assert_eq!(v["msgtype"], "m.audio");
        assert_eq!((v["m.relates_to"]["rel_type"].as_str(), v["m.relates_to"]["event_id"].as_str()), (Some("m.thread"), Some(root.as_str())));
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

/// Receipts per thread (D7): what the server counts for a thread and for the main timeline after a
/// `main` receipt and after a thread receipt, in both orders. Run:
/// `python3 the auth_smoke script --filter disposable_thread_receipts [--server tuwunel]`
// Synapse caches identical /sync requests for a while: every call asks with another timeout.
static SYNC_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_receipts_probe() {
    use crate::{ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        // Counts as the server reports them to a client that asks for per-thread counts.
        let fid = room_request(&config, "alice", reqwest::Method::POST, "user/@alice:matrix-desktop.test/filter",
            serde_json::json!({"room":{"timeline":{"limit":1,"unread_thread_notifications":true}}})).await["filter_id"].as_str().unwrap().to_owned();
        let counts = |room: String| {
            let (config, fid) = (config.clone(), fid.clone());
            async move {
                let sync = room_request(&config, "alice", reqwest::Method::GET,
                    &format!("sync?timeout={}&filter={fid}", SYNC_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), serde_json::json!({})).await;
                let r = &sync["rooms"]["join"][room.as_str()];
                serde_json::json!({"main": r["unread_notifications"]["notification_count"],
                    "threads": r["unread_thread_notifications"],
                    "receipts": r["ephemeral"]["events"].as_array().map(|e| e.iter().filter(|e| e["type"] == "m.receipt")
                        .flat_map(|e| e["content"].as_object().unwrap().iter().map(|(k, v)| serde_json::json!([k, v])).collect::<Vec<_>>()).collect::<Vec<_>>())})
            }
        };
        let mut sync = RoomSync::new(login(&config, "alice", 63).await);
        let mut rooms = Vec::new();
        for (n, order) in ["main_first", "thread_first"].into_iter().enumerate() {
            let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
                serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
            let room = created["room_id"].as_str().unwrap().to_owned();
            room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
            let put = |actor: &'static str, txn: String, content: serde_json::Value| {
                let (config, room) = (config.clone(), room.clone());
                async move {
                    let sent = room_request(&config, actor, reqwest::Method::PUT,
                        &format!("rooms/{room}/send/m.room.message/{txn}"), content).await;
                    sent["event_id"].as_str().unwrap().to_owned()
                }
            };
            let root = put("alice", format!("rc{n}-root"), serde_json::json!({"msgtype":"m.text","body":"root"})).await;
            for i in 1..=3 {
                put("bob", format!("rc{n}-r{i}"), serde_json::json!({"msgtype":"m.text","body":format!("reply {i}"),
                    "m.relates_to":{"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}})).await;
            }
            put("bob", format!("rc{n}-main"), serde_json::json!({"msgtype":"m.text","body":"main one"})).await;
            rooms.push((order, room, root));
        }
        sync.sliding_round().await.unwrap();
        for (order, room, root) in rooms {
            let scope = matrix_domain::RoomScope { account_id: AccountId::new(63), room_id: matrix_domain::RoomId::new(room.clone()) };
            let root_id = matrix_domain::EventId::new(root.clone());
            let before = counts(room.clone()).await;
            // Open the room's timeline once so both timelines exist; no receipt yet.
            let vis = crate::receipts::ReadVisibility { window_focused: true, room_foreground: false, bottom_gap_px: 0.0, public_receipts: true };
            sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), vis).await.unwrap();
            let mut page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            for _ in 0..10 {
                if page.replies.len() >= 3 { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            }
            // (main, thread) the server counts: a thread with no entry counts 0.
            let pair = |c: &serde_json::Value| (c["main"].as_u64().unwrap(), c["threads"].as_object()
                .and_then(|t| t.values().next()).map_or(0, |t| t["notification_count"].as_u64().unwrap()));
            assert_eq!(pair(&before), (1, 3), "per-thread counts arrive with the timeline filter");
            let mut log = vec![("start", before)];
            for step in if order == "main_first" { ["main", "thread"] } else { ["thread", "main"] } {
                let sent = if step == "main" { sync.mark_main_read(&scope, true).await } else { sync.mark_thread_read(&scope, &root_id, true).await };
                println!("P334_SENT {step} {sent:?}");
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                log.push((step, counts(room.clone()).await));
                // Each receipt clears its own timeline and nothing else.
                let (main, thread) = pair(&log.last().unwrap().1);
                let main_read = log.iter().skip(1).any(|(s, _)| *s == "main");
                let thread_read = log.iter().skip(1).any(|(s, _)| *s == "thread");
                assert_eq!((main, thread), (if main_read { 0 } else { 1 }, if thread_read { 0 } else { 3 }), "{order} after {step}");
            }
            let ev = room_request(&config, "alice", reqwest::Method::GET, &format!("rooms/{room}/messages?dir=b&limit=1"), serde_json::json!({})).await;
            let last = ev["chunk"][0]["event_id"].as_str().unwrap().to_owned();
            room_request(&config, "alice", reqwest::Method::POST, &format!("rooms/{room}/receipt/m.read/{last}"), serde_json::json!({})).await;
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            log.push(("raw_unthreaded", counts(room.clone()).await));
            println!("P334_COUNTS {}", serde_json::json!({"server": config["server"], "order": order, "steps": log}));
        }
    });
}

/// The counter under the root: replies by others after our read marker of the thread. Opening the
/// room (receipt `main`) leaves it alone, reading the thread drops it, a new reply raises it, and
/// a new session sees the same from the server's receipts. Run:
/// `python3 the auth_smoke script --filter disposable_thread_unread [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_unread_counter_follows_the_thread_receipt() {
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
        let root = put("alice", "un-root", serde_json::json!({"msgtype":"m.text","body":"root"})).await;
        let reply = |body: &str, root: &str| serde_json::json!({"msgtype":"m.text","body":body,
            "m.relates_to":{"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}});
        for (txn, body) in [("un-r1", "one"), ("un-r2", "two"), ("un-r3", "three")] {
            put("bob", txn, reply(body, &root)).await;
        }
        put("bob", "un-main", serde_json::json!({"msgtype":"m.text","body":"main one"})).await;
        let scope = |n| matrix_domain::RoomScope { account_id: AccountId::new(n), room_id: matrix_domain::RoomId::new(room.clone()) };
        let root_id = matrix_domain::EventId::new(root.clone());
        let vis = crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false };
        // The unread number the root row carries when the room is opened.
        async fn unread(sync: &RoomSync, scope: matrix_domain::RoomScope, root_id: &matrix_domain::EventId, vis: crate::receipts::ReadVisibility) -> Option<u32> {
            let page = sync.open_history(ViewRequests::new().start(scope).unwrap(), vis).await.unwrap().0;
            page.events.iter().find(|e| e.event_id == *root_id).and_then(|e| e.thread.as_ref()).map(|t| t.unread)
        }
        let mut sync = RoomSync::new(login(&config, "alice", 64).await);
        sync.sliding_round().await.unwrap();
        // Opening the room reads `main` only.
        let mut first = None;
        for _ in 0..10 {
            first = unread(&sync, scope(64), &root_id, vis).await;
            if first == Some(3) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = sync.sliding_round().await;
        }
        let _ = sync.mark_main_read(&scope(64), false).await;
        let after_main = unread(&sync, scope(64), &root_id, vis).await;
        // Reading the thread drops it (the receipt echo may take a moment).
        let page = sync.open_thread(&scope(64), &root_id, 25).await.unwrap();
        assert_eq!(page.replies.len(), 3);
        sync.mark_thread_read(&scope(64), &root_id, false).await.unwrap();
        let mut after_thread = None;
        for _ in 0..10 {
            let _ = sync.sliding_round().await;
            after_thread = unread(&sync, scope(64), &root_id, vis).await;
            if after_thread == Some(0) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        // A new reply raises it again.
        put("bob", "un-r4", reply("four", &root)).await;
        let mut after_new = None;
        for _ in 0..10 {
            let _ = sync.sliding_round().await;
            after_new = unread(&sync, scope(64), &root_id, vis).await;
            if after_new == Some(1) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        drop(sync);
        // A new session (new login, empty store): the server's receipts give the same number.
        let mut again = RoomSync::new(login(&config, "alice", 65).await);
        again.sliding_round().await.unwrap();
        let mut restarted = None;
        for _ in 0..10 {
            restarted = unread(&again, scope(65), &root_id, vis).await;
            if restarted == Some(1) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = again.sliding_round().await;
        }
        println!("P334_UNREAD {}", serde_json::json!({"server": config["server"], "opened": first, "after_main": after_main,
            "after_thread": after_thread, "after_new_reply": after_new, "new_session": restarted}));
        assert_eq!((first, after_main, after_thread, after_new, restarted),
            (Some(3), Some(3), Some(0), Some(1), Some(1)));
    });
}

/// Main timeline hides thread replies: only the root (with its summary) and non-thread messages stay,
/// every reply is reachable from the thread panel.
/// Run: `python3 the auth_smoke script --filter disposable_thread_replies_leave [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_replies_leave_the_main_timeline() {
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
        let root = put("alice", "hid-root", serde_json::json!({"msgtype":"m.text","body":"root"})).await;
        for (txn, body) in [("hid-r1", "reply one"), ("hid-r2", "reply two"), ("hid-r3", "reply three")] {
            put("bob", txn, serde_json::json!({"msgtype":"m.text","body":body,
                "m.relates_to":{"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}}})).await;
        }
        put("bob", "hid-main", serde_json::json!({"msgtype":"m.text","body":"main one"})).await;
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(66), room_id: matrix_domain::RoomId::new(room.clone()) };
        let root_id = matrix_domain::EventId::new(root.clone());
        let vis = crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false };
        let mut sync = RoomSync::new(login(&config, "alice", 66).await);
        // Room-list counter alice sees: bob's 3 thread replies must not count, only "main one" may.
        let mut badge = None;
        let mut note = |diffs: Vec<crate::RoomListDiff>| for d in diffs {
            if let crate::RoomListDiff::Upsert(r) = d && r.scope.room_id.as_str() == room { badge = Some(r.unread); }
        };
        note(sync.sliding_round().await.unwrap());
        let mut bodies = Vec::new();
        let mut replies = None;
        for _ in 0..10 {
            let page = sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), vis).await.unwrap().0;
            bodies = page.events.iter().map(|e| e.body.clone()).collect();
            replies = page.events.iter().find(|e| e.event_id == root_id).and_then(|e| e.thread.as_ref()).map(|t| t.num_replies);
            if replies == Some(3) && bodies.contains(&"main one".to_owned()) { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Ok(diffs) = sync.sliding_round().await { note(diffs); }
        }
        let panel = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        let panel: Vec<_> = panel.replies.iter().map(|r| r.body.clone()).collect();
        println!("P335_HIDE {}", serde_json::json!({"server": config["server"], "main": bodies, "root_replies": replies, "panel": panel, "room_badge": badge}));
        // Status rows (room creation, joins) precede the messages: only the messages are the contract.
        assert!(bodies.iter().all(|b| !b.starts_with("reply ")) && bodies.ends_with(&["root".to_owned(), "main one".to_owned()]),
            "thread replies stay out of the main timeline: {bodies:?}");
        assert_eq!(replies, Some(3), "the root keeps its summary");
        assert_eq!(panel, ["reply one", "reply two", "reply three"], "every reply is reachable from the panel");
        assert_eq!(badge, Some(1), "main counter counts \"main one\" only, not the 3 thread replies");
    });
}

/// Thread roots that are not text: the panel gets the root with its `media` (image: the bubble's picture is
/// downloadable by event id; file/video: label; sticker: no media). Run: `python3 the auth_smoke script --filter disposable_thread_root_kinds [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_root_kinds_reach_the_panel() {
    use crate::{ViewRequests, history::MediaKind, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let upload = |mime: &'static str, data: Vec<u8>| {
            let config = config.clone();
            async move {
                let response = http(&config).post(format!("{}_matrix/media/v3/upload", config["url"].as_str().unwrap()))
                    .bearer_auth(config["alice_token"].as_str().unwrap()).header("Content-Type", mime).body(data).send().await.unwrap();
                let v: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
                v["content_uri"].as_str().unwrap().to_owned()
            }
        };
        let png = upload("image/png", MEDIA_PNG.to_vec()).await;
        let blob = upload("application/octet-stream", b"synthetic".to_vec()).await;
        let put = |kind: &'static str, txn: &'static str, content: serde_json::Value| {
            let (config, room, actor) = (config.clone(), room.clone(), if txn.ends_with("-r") { "bob" } else { "alice" });
            async move {
                let sent = room_request(&config, actor, reqwest::Method::PUT, &format!("rooms/{room}/send/{kind}/{txn}"), content).await;
                sent["event_id"].as_str().unwrap().to_owned()
            }
        };
        let roots = [
            ("text", put("m.room.message", "k-text", serde_json::json!({"msgtype":"m.text","body":"plain root"})).await, None),
            ("image", put("m.room.message", "k-image", serde_json::json!({"msgtype":"m.image","body":"pic.png","url":png,"info":{"mimetype":"image/png","size":1,"w":8,"h":8}})).await, Some(MediaKind::Image)),
            ("file", put("m.room.message", "k-file", serde_json::json!({"msgtype":"m.file","body":"a.pdf","url":blob,"info":{"mimetype":"application/pdf","size":9}})).await, Some(MediaKind::File)),
            ("video", put("m.room.message", "k-video", serde_json::json!({"msgtype":"m.video","body":"a.mp4","url":blob,"info":{"mimetype":"video/mp4","size":9,"duration":12000}})).await, Some(MediaKind::Video)),
            ("sticker", put("m.sticker", "k-sticker", serde_json::json!({"body":"st","url":png,"info":{"mimetype":"image/png","w":8,"h":8}})).await, None),
        ];
        for (name, id, _) in &roots {
            let txn: &'static str = Box::leak(format!("k-{name}-r").into_boxed_str());
            put("m.room.message", txn, serde_json::json!({"msgtype":"m.text","body":format!("{name} reply"),
                "m.relates_to":{"rel_type":"m.thread","event_id":id,"is_falling_back":true,"m.in_reply_to":{"event_id":id}}})).await;
        }
        let session = login(&config, "alice", 71).await;
        let client = session.client.clone().unwrap();
        let mut sync = RoomSync::new(login(&config, "alice", 71).await);
        sync.sliding_round().await.unwrap();
        crate::room_sync::RoomSync::sliding_catch_up(&client).await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(71), room_id: matrix_domain::RoomId::new(room.clone()) };
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let mut tags = ViewRequests::new();
        let mut seen = Vec::new();
        for (name, id, want) in &roots {
            let root_id = matrix_domain::EventId::new(id.clone());
            let mut page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            for _ in 0..10 {
                if page.root.is_some() && !page.replies.is_empty() { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let _ = sync.sliding_round().await;
                page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            }
            let got = page.root.as_ref().and_then(|r| r.media.as_ref()).map(|m| (m.kind, m.label.clone()));
            seen.push(serde_json::json!({"root": name, "media": got.as_ref().map(|g| format!("{:?} {}", g.0, g.1)),
                "body": page.root.as_ref().map(|r| r.body.clone()), "replies": page.replies.len()}));
            assert!(page.root.is_some(), "{name}: root reaches the panel");
            assert_eq!(page.replies.len(), 1, "{name}: its reply");
            assert_eq!(got.as_ref().map(|g| g.0), *want, "{name}: media kind of the root");
            if *want == Some(MediaKind::Image) {
                // The panel draws the bubble's picture: the same download by event id must work for the root.
                let tag = tags.start(scope.clone()).unwrap();
                let down = session.download_png(&tag, &root_id, false).await;
                seen.push(serde_json::json!({"image_root_download": down.is_ok()}));
                assert!(down.is_ok(), "image root downloads like a bubble");
            }
        }
        println!("T_ROOTS {}", serde_json::json!({"server": config["server"], "roots": seen}));
    });
}

/// Reopen a thread already seen: with the homeserver frozen (SIGSTOP) the second open must not wait for the
/// network and returns what the first showed; a reply sent while it was closed shows up once the server is back.
/// Run: `python3 the auth_smoke script --filter disposable_thread_reopen [--server tuwunel]`
#[test]
#[cfg(unix)]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_reopen_does_not_wait_for_the_network() {
    use crate::{ViewRequests, room_sync::RoomSync};
    let config = fixture();
    runtime().block_on(async {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let put = |actor: &'static str, txn: &'static str, body: &'static str, root: Option<String>| {
            let (config, room) = (config.clone(), room.clone());
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(root) = root {
                    content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,"is_falling_back":true,"m.in_reply_to":{"event_id":root}});
                }
                let sent = room_request(&config, actor, reqwest::Method::PUT, &format!("rooms/{room}/send/m.room.message/{txn}"), content).await;
                sent["event_id"].as_str().unwrap().to_owned()
            }
        };
        let root = put("alice", "ro-root", "root", None).await;
        for (txn, body) in [("ro-r1", "one"), ("ro-r2", "two"), ("ro-r3", "three")] {
            put("bob", txn, body, Some(root.clone())).await;
        }
        let mut sync = RoomSync::new(login(&config, "alice", 72).await);
        sync.sliding_round().await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(72), room_id: matrix_domain::RoomId::new(room.clone()) };
        sync.open_history(ViewRequests::new().start(scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: true }).await.unwrap();
        let root_id = matrix_domain::EventId::new(root.clone());
        let mut first = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        for _ in 0..10 {
            if first.replies.len() >= 3 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            first = sync.open_thread(&scope, &root_id, 25).await.unwrap();
        }
        assert_eq!(first.replies.len(), 3, "first open shows the thread");
        // The page is separate from the receipt: with the server frozen both public values still page.
        let client = sync.client().unwrap();
        let timelines = crate::room_sync::Timelines::default();
        let warm = crate::threads::open_scoped(client, &timelines, AccountId::new(72), &scope, &root_id, 25).await.unwrap();
        assert_eq!(warm.replies.len(), 3);
        let target = warm.replies.last().unwrap().event_id.clone();
        let mut scoped_timeouts = Vec::new();
        let mut receipt_timeouts = Vec::new();
        for public in [true, false] {
            let _paused = PausedServer::new(&config);
            let mut views = ViewRequests::new();
            let tag = views.start(scope.clone()).unwrap();
            let start = std::time::Instant::now();
            let result = tokio::time::timeout(std::time::Duration::from_secs(3), crate::threads::open_scoped(client, &timelines, AccountId::new(72), &scope, &root_id, 25)).await;
            println!("T_SCOPED {}", serde_json::json!({"server":config["server"],"public":public,"elapsed_ms":start.elapsed().as_millis() as u64,"timeout":result.is_err(),"replies":result.as_ref().ok().and_then(|r|r.as_ref().ok()).map(|p|p.replies.iter().map(|r|r.body.clone()).collect::<Vec<_>>())}));
            scoped_timeouts.push((public, result.is_err()));
            // The receipt on the frozen server must stay separate: it hangs, the page above did not.
            let receipt = tokio::time::timeout(std::time::Duration::from_secs(3), crate::threads::mark_read_target(client, &timelines, AccountId::new(72), &tag, &root_id, &target, public)).await;
            println!("T_RECEIPT_SEPARATE {}", serde_json::json!({"server":config["server"],"public":public,"timeout":receipt.is_err(),"result":receipt.as_ref().ok().map(|r|r.as_ref().map_err(|_| "error"))}));
            receipt_timeouts.push((public, receipt.is_err()));
        }
        assert!(scoped_timeouts.iter().all(|(_, timed_out)| !timed_out), "open_scoped waited for frozen server: {scoped_timeouts:?}");
        assert!(receipt_timeouts.iter().all(|(_, timed_out)| *timed_out), "receipt did not wait for the frozen server: {receipt_timeouts:?}");
        let frozen = {
            let _paused = PausedServer::new(&config);
            let t = std::time::Instant::now();
            let again = tokio::time::timeout(std::time::Duration::from_secs(20), sync.open_thread(&scope, &root_id, 25)).await;
            (t.elapsed(), again)
        };
        println!("T_REOPEN {}", serde_json::json!({"server": config["server"], "frozen_ms": frozen.0.as_millis() as u64,
            "timed_out": frozen.1.is_err(), "replies": frozen.1.as_ref().ok().and_then(|r| r.as_ref().ok()).map(|p| p.replies.len())}));
        let again = frozen.1.expect("reopen with the server frozen must not hang").expect("reopen answers from the cache");
        let bodies = |p: &crate::threads::ThreadPage| p.replies.iter().map(|r| r.body.clone()).collect::<Vec<_>>();
        assert_eq!(bodies(&again), bodies(&first), "same content from the cache");
        assert_eq!(again.root.as_ref().map(|r| r.body.as_str()), Some("root"));
        // A reply sent while the thread is closed (and the server was away) shows up after reconnection.
        put("bob", "ro-r4", "four", Some(root.clone())).await;
        let mut after = None;
        for _ in 0..20 {
            let _ = sync.sliding_round().await;
            let page = sync.open_thread(&scope, &root_id, 25).await.unwrap();
            if page.replies.iter().any(|r| r.body == "four") { after = Some(page); break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        let after = after.expect("the reply sent while closed reaches the thread");
        assert_eq!(bodies(&after), ["one", "two", "three", "four"]);
        assert_eq!(after.root.as_ref().and_then(|r| r.thread.as_ref()).map(|t| t.num_replies), Some(4));
        // Last on purpose: the content checks above hold even when this one fails (`fill` paginates over the network).
        assert!(frozen.0 < std::time::Duration::from_secs(3), "reopen waited {:?} for the network", frozen.0);
    });
}


/// Next `Thread` result, skipping the worker's other events (room search, message search).
async fn recv_thread_page(
    results: &mut crate::Receiver<crate::operations::OperationResult>,
    timeout: std::time::Duration,
) -> (
    crate::RequestTag,
    matrix_domain::EventId,
    Result<Box<crate::history::ThreadPage>, crate::operations::OperationError>,
) {
    let end = tokio::time::Instant::now() + timeout;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, results.recv()).await {
            Ok(Some(crate::operations::OperationResult::Thread { tag, root, result })) => {
                return (tag, root, result)
            }
            Ok(Some(_)) => continue,
            _ => panic!("expected a thread page"),
        }
    }
}

/// The real dispatcher: a page is answered while the receipt waits, a local command runs before
/// the ACK, and revoking the receipt's tag stops its send and retries. The server is frozen as the
/// barrier; the receipt already admitted to the transport is reported apart, never as a success.
/// Run: `python3 the auth_smoke script --filter disposable_thread_page_and_receipt [--server tuwunel]`
#[test]
#[cfg(unix)]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_page_and_receipt_are_separate() {
    use crate::{
        ViewRequests,
        operations::{OperationCommand, OperationResult},
    };
    let config = fixture();
    let (mut worker, _rooms, room) = ready_worker(&config);
    let send = |txn: &'static str, body: &'static str, root: Option<String>| {
        let (config, room) = (config.clone(), room.clone());
        async move {
            let mut content = serde_json::json!({"msgtype":"m.text","body":body});
            if let Some(root) = root {
                content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,
                    "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
            }
            room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/{txn}"), content).await["event_id"]
                .as_str().unwrap().to_owned()
        }
    };
    let root = runtime().block_on(send("tpr-root", "root", None));
    for (txn, body) in [("tpr-r1", "one"), ("tpr-r2", "two"), ("tpr-r3", "three")] {
        runtime().block_on(send(txn, body, Some(root.clone())));
    }
    let scope = RoomScope { account_id: AccountId::new(22), room_id: RoomId::new(room.clone()) };
    let root = matrix_domain::EventId::new(root);
    let (mut commands, mut results) = worker.take_operation_route().expect("operation route");
    runtime().block_on(async {
        let mut page_views = ViewRequests::new();
        let mut warm = None;
        for _ in 0..10 {
            let tag = page_views.start(scope.clone()).unwrap();
            assert!(commands.send(OperationCommand::Thread { tag: tag.clone(), root: root.clone(), want: 25 }).await.is_ok());
            let (actual, actual_root, result) = recv_thread_page(&mut results, std::time::Duration::from_secs(8)).await;
            assert!(page_views.accepts(&actual) && actual_root == root);
            let page = result.expect("the thread page loads");
            if page.replies.len() >= 3 { warm = Some(page); break; }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let warm = warm.expect("the thread warms");
        let target = warm.replies.last().unwrap().event_id.clone();
        // Barrier: the server is frozen. The receipt stays in its slot; pages and local commands pass.
        let _paused = PausedServer::new(&config);
        let mut receipt_views = ViewRequests::new();
        let receipt_tag = receipt_views.start(scope.clone()).unwrap();
        assert!(commands.send(OperationCommand::ThreadRead {
            tag: receipt_tag.clone(), root: root.clone(), target: target.clone(), public: true,
        }).await.is_ok());
        // A page from the warm cache: answered while the receipt does not resolve.
        let page_tag = page_views.start(scope.clone()).unwrap();
        assert!(commands.send(OperationCommand::Thread { tag: page_tag.clone(), root: root.clone(), want: 25 }).await.is_ok());
        let (actual, actual_root, result) = recv_thread_page(&mut results, std::time::Duration::from_secs(3)).await;
        assert!(page_views.accepts(&actual) && actual_root == root, "the current page tag is accepted");
        assert!(result.is_ok(), "the page arrives from the cache");
        // A local command answered before the ACK: the user search state needs no thread receipt.
        assert!(commands.send(OperationCommand::SearchUsers { term: "alice".into(), all: false }).await.is_ok());
        let local = tokio::time::timeout(std::time::Duration::from_secs(3), results.recv()).await
            .expect("a local command is handled before the receipt resolves");
        // Either a local search state or no event: what matters is that the channel moved before the ACK.
        let _ = local;
        // Revoke while the HTTP request waits: the UI must not accept a late result.
        receipt_views.cancel();
        // The next command drops the cancelled slot: no retry and no orphan.
        let next_tag = page_views.start(scope.clone()).unwrap();
        assert!(commands.send(OperationCommand::Thread { tag: next_tag.clone(), root: root.clone(), want: 25 }).await.is_ok());
        let (actual, actual_root, result) = recv_thread_page(&mut results, std::time::Duration::from_secs(3)).await;
        assert!(page_views.accepts(&actual) && actual_root == root && result.is_ok(), "the page still loads after revocation");
        // Released server: the already-admitted request may complete, but the dropped slot is gone.
        drop(_paused);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let mut late = 0;
        while let Ok(Some(result)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), results.recv()).await
        {
            if let OperationResult::ThreadRead { tag, .. } = result {
                late += 1;
                assert!(!receipt_views.accepts(&tag), "a revoked receipt is never applied");
            }
        }
        println!("T_PAGE_RECEIPT {}", serde_json::json!({
            "server": config["server"], "page_while_receipt": true, "local_before_ack": true,
            "revoked": true, "late_results": late,
        }));
    });
    worker.shutdown().expect("worker stops");
}

/// The next receipt result for exactly this intent (other worker events are skipped); `None` on timeout.
async fn recv_receipt(
    results: &mut crate::Receiver<crate::operations::OperationResult>,
    tag: &crate::RequestTag,
    timeout: std::time::Duration,
) -> Option<Result<bool, crate::operations::OperationError>> {
    let end = tokio::time::Instant::now() + timeout;
    loop {
        let left = end.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(left, results.recv()).await {
            Ok(Some(crate::operations::OperationResult::ThreadRead { tag: actual, result, .. }))
                if actual.is_same(tag) =>
            {
                return Some(result);
            }
            Ok(Some(_)) => continue,
            _ => return None,
        }
    }
}

/// One page command through the worker, with its own view tag (the call the panel makes), asserted current.
async fn worker_page(
    commands: &mut crate::Sender<crate::operations::OperationCommand>,
    results: &mut crate::Receiver<crate::operations::OperationResult>,
    views: &mut crate::ViewRequests,
    scope: &RoomScope,
    root: &matrix_domain::EventId,
) -> Box<crate::history::ThreadPage> {
    let tag = views.start(scope.clone()).unwrap();
    assert!(
        commands
            .send(crate::operations::OperationCommand::Thread {
                tag: tag.clone(),
                root: root.clone(),
                want: 25,
            })
            .await
            .is_ok()
    );
    let (actual, seen, result) = recv_thread_page(results, std::time::Duration::from_secs(8)).await;
    assert!(views.accepts(&actual) && seen == *root, "the current page tag is accepted");
    result.expect("the page loads")
}

/// Restart on the same sealed store with the homeserver already frozen (SIGSTOP): the page comes from
/// the store with no online round, the receipt is separate (it cannot resolve), the resumed server
/// brings the fourth reply without losing the first three, and only then does a receipt confirm.
/// Run: `python3 the auth_smoke script --filter disposable_thread_restart_offline [--server tuwunel]`
#[test]
#[cfg(unix)]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_restart_offline_on_the_kept_store() {
    use crate::{
        ViewRequests, persistence::AccountStore, room_sync::{RoomSync, Timelines},
    };
    const PASS: &str = "synthetic-passphrase-thread-restart-1";
    let config = fixture();
    let url = config["url"].as_str().unwrap().to_owned();
    let account = AccountId::new(94);
    let stores = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("thread-restart");
    // The sealed store directory must be private, like the app's own.
    use std::os::unix::fs::DirBuilderExt as _;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&stores).unwrap();
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let put = |actor: &'static str, txn: &'static str, body: &'static str, thread: Option<String>| {
            let (config, room) = (config.clone(), room.clone());
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(root) = thread {
                    content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,
                        "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
                }
                room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await["event_id"]
                    .as_str().unwrap().to_owned()
            }
        };
        let root = matrix_domain::EventId::new(put("alice", "rst-root", "root", None).await);
        for (txn, body) in [("rst-r1", "one"), ("rst-r2", "two"), ("rst-r3", "three")] {
            put("bob", txn, body, Some(root.as_str().to_owned())).await;
        }
        let scope = RoomScope { account_id: account, room_id: RoomId::new(room.clone()) };
        let vis = crate::receipts::ReadVisibility {
            window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false,
        };
        async fn unread(
            sync: &RoomSync,
            scope: RoomScope,
            root: &matrix_domain::EventId,
            vis: crate::receipts::ReadVisibility,
        ) -> Option<u32> {
            let page = sync.open_history(ViewRequests::new().start(scope).unwrap(), vis).await.ok()?.0;
            page.events.iter().find(|e| e.event_id == *root).and_then(|e| e.thread.as_ref()).map(|t| t.unread)
        }
        // First run, online: the room and its thread reach the store.
        let store = AccountStore::create(&stores, account).unwrap();
        let session = store
            .password_login_with_http(
                HomeserverUrl::parse(&url).unwrap(),
                "@alice:matrix-desktop.test",
                config["alice_password"].as_str().unwrap(),
                PASS,
                http(&config),
            )
            .await
            .unwrap();
        let client = session.client.clone().unwrap();
        let mut sync = RoomSync::new(session);
        let mut warm = None;
        for _ in 0..20 {
            let _ = sync.sliding_round().await;
            if let Ok(page) = sync.open_thread(&scope, &root, 25).await && page.replies.len() >= 3 {
                warm = Some(page);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let warm = warm.expect("the thread reaches the store online");
        let target = warm.replies.last().unwrap().event_id.clone();
        let before = unread(&sync, scope.clone(), &root, vis).await;
        assert_eq!(before, Some(3), "three replies unread before any receipt");
        drop(sync);
        drop(client);
        // Restart on the same sealed store; the homeserver is frozen before the new session starts.
        let paused = PausedServer::new(&config);
        let session = AccountStore::open(&stores, account)
            .unwrap()
            .restore_with_http(PASS, http(&config))
            .await
            .unwrap();
        let client = session.client.clone().unwrap();
        let mut again = RoomSync::new(session);
        let timelines = Timelines::default();
        let start = std::time::Instant::now();
        let page = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::threads::open_scoped(&client, &timelines, account, &scope, &root, 25),
        )
        .await
        .expect("the page must come from the store, not from the frozen server")
        .unwrap();
        let page_ms = start.elapsed().as_millis() as u64;
        let cached: Vec<String> = page.replies.iter().map(|r| r.body.clone()).collect();
        assert_eq!(cached, ["one", "two", "three"], "every reply seen before the restart comes from the store: {cached:?}");
        assert!(page.root.is_some(), "the root comes from the store");
        // The receipt is separate: on the same frozen server it does not resolve.
        let mut views = ViewRequests::new();
        let tag = views.start(scope.clone()).unwrap();
        let frozen = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            crate::threads::mark_read_target(&client, &timelines, account, &tag, &root, &target, true),
        )
        .await;
        assert!(frozen.is_err(), "the receipt waits for the frozen server: {frozen:?}");
        drop(paused);
        // Resumed: the fourth reply arrives and nothing of the first three is lost.
        let fourth = matrix_domain::EventId::new(put("bob", "rst-r4", "four", Some(root.as_str().to_owned())).await);
        let mut after = None;
        for _ in 0..20 {
            let _ = again.sliding_round().await;
            if let Ok(page) = again.open_thread(&scope, &root, 25).await && page.replies.len() >= 4 {
                after = Some(page);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let after = after.expect("the fourth reply reaches the thread after the resume");
        let bodies: Vec<String> = after.replies.iter().map(|r| r.body.clone()).collect();
        assert_eq!(bodies, ["one", "two", "three", "four"]);
        // Only now an intent exists for the fourth reply: a fresh look at the store holds it too.
        let timelines = Timelines::default();
        let page = crate::threads::open_scoped(&client, &timelines, account, &scope, &root, 25).await.unwrap();
        let stored: Vec<String> = page.replies.iter().map(|r| r.body.clone()).collect();
        let mut views = ViewRequests::new();
        let tag = views.start(scope.clone()).unwrap();
        let ack = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            crate::threads::mark_read_target(&client, &timelines, account, &tag, &root, &fourth, false),
        )
        .await;
        println!(
            "T_RESTART {}",
            serde_json::json!({"server": config["server"], "page_ms": page_ms, "cached": cached,
                "receipt_while_frozen": frozen.is_err(), "after_resume": bodies, "stored_after_resume": stored,
                "unread_before": before, "ack": matches!(&ack, Ok(Ok(true)))})
        );
        assert_eq!(stored, ["one", "two", "three", "four"], "the store holds the fourth reply");
        assert!(matches!(&ack, Ok(Ok(true))), "the receipt confirms after the resume: {ack:?}");
    });
    // The harness fails the whole run if the sandbox keeps private state: the store goes with the test.
    std::fs::remove_dir_all(&stores).unwrap();
}

/// The receipt gate as the fixture's own access lines see it (`the thread_privacy_probe script`): an
/// intent admitted before the shutdown completes and cannot be retracted, a revoked intent produces no
/// further request, the other direction carries the exact target, the homeserver records it under the
/// thread's root (never unthreaded, never the other thread), a no-op sends nothing, and a foreign
/// account, an unknown room or another thread's event are refused before any send.
/// Run: `PROBE_SERVER=synapse python3 the thread_privacy_probe script`
#[test]
#[cfg(unix)]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_receipt_privacy_gate() {
    use crate::{
        ViewRequests,
        operations::{OperationCommand, OperationResult},
        room_sync::{RoomSync, Timelines},
    };
    use matrix_sdk::ruma::events::receipt::{ReceiptThread, ReceiptType};
    let config = fixture();
    let (mut worker, _rooms, _own) = ready_worker(&config);
    runtime().block_on(async {
        let created = room_request(
            &config,
            "alice",
            reqwest::Method::POST,
            "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"]}),
        )
        .await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        let put = |actor: &'static str, txn: &str, body: &str, thread: Option<String>| {
            let (config, room, txn, body) = (config.clone(), room.clone(), txn.to_owned(), body.to_owned());
            async move {
                let mut content = serde_json::json!({"msgtype":"m.text","body":body});
                if let Some(root) = thread {
                    content["m.relates_to"] = serde_json::json!({"rel_type":"m.thread","event_id":root,
                        "is_falling_back":true,"m.in_reply_to":{"event_id":root}});
                }
                room_request(&config, actor, reqwest::Method::PUT,
                    &format!("rooms/{room}/send/m.room.message/{txn}"), content).await["event_id"]
                    .as_str().unwrap().to_owned()
            }
        };
        let root = put("alice", "pg-root", "root", None).await;
        for (txn, body) in [("pg-r1", "one"), ("pg-r2", "two"), ("pg-r3", "three")] {
            put("bob", txn, body, Some(root.clone())).await;
        }
        let root2 = put("alice", "pg-root2", "root2", None).await;
        let reply2 = put("bob", "pg-r2a", "second", Some(root2.clone())).await;
        let scope = RoomScope { account_id: AccountId::new(22), room_id: RoomId::new(room.clone()) };
        let root_id = matrix_domain::EventId::new(root.clone());
        let root2_id = matrix_domain::EventId::new(root2.clone());
        let reply2_id = matrix_domain::EventId::new(reply2.clone());
        let (mut commands, mut results) = worker.take_operation_route().expect("operation route");
        let mut page_views = ViewRequests::new();
        // Warm: the worker's own thread timeline exists and its page shows the three replies.
        let mut warm = None;
        for _ in 0..20 {
            let page = worker_page(&mut commands, &mut results, &mut page_views, &scope, &root_id).await;
            if page.replies.len() >= 3 {
                warm = Some(page);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let warm = warm.expect("the thread warms");
        let mut target = warm.replies.last().unwrap().event_id.clone();
        // A second session of the same account: the counters, the marker state and the isolation checks.
        let mut reader = RoomSync::new(login(&config, "alice", 95).await);
        let vis = crate::receipts::ReadVisibility {
            window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false,
        };
        async fn unread(
            sync: &RoomSync,
            scope: RoomScope,
            root: &matrix_domain::EventId,
            vis: crate::receipts::ReadVisibility,
        ) -> Option<u32> {
            let page = sync.open_history(ViewRequests::new().start(scope).unwrap(), vis).await.ok()?.0;
            page.events.iter().find(|e| e.event_id == *root).and_then(|e| e.thread.as_ref()).map(|t| t.unread)
        }
        let mut before = None;
        for _ in 0..20 {
            before = unread(&reader, scope.clone(), &root_id, vis).await;
            if before == Some(3) {
                break;
            }
            let _ = reader.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        assert_eq!(before, Some(3), "the page is present and the thread is unread before any receipt");
        let other_thread = unread(&reader, scope.clone(), &root2_id, vis).await;
        let client = reader.client().unwrap().clone();
        let room_handle = client.get_room(&matrix_sdk::ruma::RoomId::parse(room.as_str()).unwrap()).expect("room");
        let me = client.user_id().unwrap().to_owned();
        let in_thread = ReceiptThread::Thread(matrix_sdk::ruma::EventId::parse(root.as_str()).unwrap());
        async fn marker(
            room: &matrix_sdk::Room,
            me: &matrix_sdk::ruma::OwnedUserId,
            kind: ReceiptType,
            thread: &ReceiptThread,
        ) -> Option<String> {
            room.load_user_receipt(kind, thread, me).await.ok().flatten().map(|(id, _)| id.to_string())
        }
        /// The marker as the homeserver really holds it: sync until it is the one we expect.
        async fn wait_marker(
            reader: &mut RoomSync,
            room: &matrix_sdk::Room,
            me: &matrix_sdk::ruma::OwnedUserId,
            kind: ReceiptType,
            thread: &ReceiptThread,
            want: &str,
        ) -> Option<String> {
            let mut seen = None;
            for _ in 0..40 {
                let _ = reader.sliding_round().await;
                seen = marker(room, me, kind.clone(), thread).await;
                if seen.as_deref() == Some(want) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            seen
        }
        // 1. Frozen: the intent is admitted and the server comes back before any revocation, so the
        // request completes: an admitted receipt is not retractable. The page passes meanwhile.
        let paused = PausedServer::new(&config);
        let mut read1 = ViewRequests::new();
        let tag1 = read1.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag1.clone(), root: root_id.clone(), target: target.clone(), public: true,
                })
                .await
                .is_ok()
        );
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let page = worker_page(&mut commands, &mut results, &mut page_views, &scope, &root_id).await;
        assert!(page.replies.len() >= 3, "the page is served while the receipt waits");
        let mut premature = false;
        while let Ok(Some(result)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), results.recv()).await
        {
            if matches!(&result, OperationResult::ThreadRead { tag, .. } if tag.is_same(&tag1)) {
                premature = true;
            }
        }
        assert!(!premature, "no ACK can arrive while the server is stopped");
        drop(paused);
        let ack1 = recv_receipt(&mut results, &tag1, std::time::Duration::from_secs(10)).await;
        assert!(matches!(ack1, Some(Ok(true))), "the admitted receipt completes after the resume: {ack1:?}");
        let target1 = target.clone();
        let public_thread = wait_marker(&mut reader, &room_handle, &me, ReceiptType::Read, &in_thread, target1.as_str()).await;
        let public_unthreaded = marker(&room_handle, &me, ReceiptType::Read, &ReceiptThread::Unthreaded).await;
        let private_unthreaded = marker(&room_handle, &me, ReceiptType::ReadPrivate, &ReceiptThread::Unthreaded).await;
        println!("T_STATE after-admitted public_thread={public_thread:?} want {target1:?} unthreaded={public_unthreaded:?}/{private_unthreaded:?}");
        assert_eq!(public_thread.as_deref(), Some(target1.as_str()),
            "a second session sees the admitted receipt under the thread's root, on the exact event");
        assert_ne!(public_unthreaded.as_deref(), Some(target1.as_str()), "a thread receipt never lands unthreaded");
        assert_ne!(private_unthreaded.as_deref(), Some(target1.as_str()), "a thread receipt never lands unthreaded");
        // 2. The other direction. A private intent admitted and revoked while the server is stopped
        // must not start a second request once the preference is public.
        put("bob", "pg-r4", "four", Some(root.clone())).await;
        let mut next = None;
        for _ in 0..20 {
            let page = worker_page(&mut commands, &mut results, &mut page_views, &scope, &root_id).await;
            if page.replies.len() >= 4 {
                next = page.replies.last().map(|r| r.event_id.clone());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        target = next.expect("the new reply reaches the thread");
        let target2 = target.clone();
        let paused = PausedServer::new(&config);
        let mut read2 = ViewRequests::new();
        let tag2 = read2.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag2.clone(), root: root_id.clone(), target: target2.clone(), public: false,
                })
                .await
                .is_ok()
        );
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        read2.cancel();
        let page = worker_page(&mut commands, &mut results, &mut page_views, &scope, &root_id).await;
        assert!(page.replies.len() >= 4, "the page still passes after the revocation");
        drop(paused);
        let mut read3 = ViewRequests::new();
        let tag3 = read3.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag3.clone(), root: root_id.clone(), target: target2.clone(), public: true,
                })
                .await
                .is_ok()
        );
        let ack2 = recv_receipt(&mut results, &tag3, std::time::Duration::from_secs(10)).await;
        assert!(matches!(ack2, Some(Ok(true))), "the public receipt after the toggle is confirmed: {ack2:?}");
        // Synapse moves the public marker to the new reply; Tuwunel keeps the receipt it already holds for
        // that event (the intent admitted before the revocation was private, and a private one is not
        // downgraded), so what must hold on both is: the thread's receipt covers the new reply.
        let public_thread = wait_marker(&mut reader, &room_handle, &me, ReceiptType::Read, &in_thread, target2.as_str()).await;
        let private_thread_then = marker(&room_handle, &me, ReceiptType::ReadPrivate, &in_thread).await;
        let covered = public_thread.as_deref() == Some(target2.as_str()) || private_thread_then.as_deref() == Some(target2.as_str());
        println!("T_STATE after-toggle public_thread={public_thread:?} private_thread={private_thread_then:?} want {target2:?}");
        assert!(covered, "a receipt for this thread covers the new reply on the exact event");
        assert!(
            [target1.as_str(), target2.as_str()].contains(&public_thread.as_deref().unwrap_or("")),
            "the public marker stays where it was or advances, never elsewhere: {public_thread:?}"
        );
        // 3. The private kind, on a newer event: same thread, exact target, and the public marker stays.
        put("bob", "pg-r5", "five", Some(root.clone())).await;
        let mut next = None;
        for _ in 0..20 {
            let page = worker_page(&mut commands, &mut results, &mut page_views, &scope, &root_id).await;
            if page.replies.len() >= 5 {
                next = page.replies.last().map(|r| r.event_id.clone());
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let target3 = next.expect("the fifth reply reaches the thread");
        let mut read4 = ViewRequests::new();
        let tag4 = read4.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag4.clone(), root: root_id.clone(), target: target3.clone(), public: false,
                })
                .await
                .is_ok()
        );
        let ack3 = recv_receipt(&mut results, &tag4, std::time::Duration::from_secs(10)).await;
        assert!(matches!(ack3, Some(Ok(true))), "the private receipt is confirmed: {ack3:?}");
        let private_thread = wait_marker(&mut reader, &room_handle, &me, ReceiptType::ReadPrivate, &in_thread, target3.as_str()).await;
        let public_after_private = marker(&room_handle, &me, ReceiptType::Read, &in_thread).await;
        println!("T_STATE private private_thread={private_thread:?} want {target3:?} public_marker={public_after_private:?}");
        assert_eq!(private_thread.as_deref(), Some(target3.as_str()),
            "off records the private receipt under the thread's root, on the exact event");
        assert_ne!(public_after_private.as_deref(), Some(target3.as_str()), "a private receipt never moves the public marker");
        // 4. No-op: a second intent whose marker already covers the target sends nothing.
        let mut read5 = ViewRequests::new();
        let tag5 = read5.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag5.clone(), root: root_id.clone(), target: target3.clone(), public: false,
                })
                .await
                .is_ok()
        );
        let noop_private = recv_receipt(&mut results, &tag5, std::time::Duration::from_secs(10)).await;
        // The public no-op targets the public marker the server really holds: Synapse moved it to target2 in
        // step 2, Tuwunel kept target1, and a public intent on target2 there is a genuine send, not a no-op.
        let public_held = matrix_domain::EventId::new(public_after_private.clone().expect("a public marker"));
        let mut read6 = ViewRequests::new();
        let tag6 = read6.start(scope.clone()).unwrap();
        assert!(
            commands
                .send(OperationCommand::ThreadRead {
                    tag: tag6.clone(), root: root_id.clone(), target: public_held, public: true,
                })
                .await
                .is_ok()
        );
        let noop_public = recv_receipt(&mut results, &tag6, std::time::Duration::from_secs(10)).await;
        // A late result of the revoked intent is never applicable to the panel that revoked it.
        let mut stale = 0;
        while let Ok(Some(result)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), results.recv()).await
        {
            if let OperationResult::ThreadRead { tag, .. } = result {
                assert!(!read2.accepts(&tag), "a revoked receipt is never applied");
                stale += 1;
            }
        }
        // 5. Isolation: another worker-local account, an unknown room and another thread's event are
        // refused before any send (the probe asserts no request of theirs reaches the homeserver).
        let reader_account = AccountId::new(95);
        let reader_scope = RoomScope { account_id: reader_account, room_id: RoomId::new(room.clone()) };
        let timelines = Timelines::default();
        let _ = crate::threads::open_scoped(&client, &timelines, reader_account, &reader_scope, &root_id, 25).await.unwrap();
        let mut views = ViewRequests::new();
        let foreign_tag = views.start(reader_scope.clone()).unwrap();
        let wrong_scope = crate::threads::mark_read_target(&client, &timelines, AccountId::new(999), &foreign_tag, &root_id, &target3, true).await.is_err();
        let mut views = ViewRequests::new();
        let unknown_tag = views.start(RoomScope { account_id: reader_account, room_id: RoomId::new("!nope:matrix-desktop.test") }).unwrap();
        let unknown_room = crate::threads::mark_read_target(&client, &timelines, reader_account, &unknown_tag, &root_id, &target3, true).await.is_err();
        let mut views = ViewRequests::new();
        let other_tag = views.start(reader_scope.clone()).unwrap();
        let other_thread_event = crate::threads::mark_read_target(&client, &timelines, reader_account, &other_tag, &root_id, &reply2_id, true).await.is_err();
        let other_thread_marker = marker(&room_handle, &me, ReceiptType::Read, &ReceiptThread::Thread(matrix_sdk::ruma::EventId::parse(root2.as_str()).unwrap())).await;
        assert!(other_thread_marker.is_none(), "the other thread never got a receipt: {other_thread_marker:?}");
        // The counters: only the right thread is read, the other one stays unread.
        let mut after = None;
        for _ in 0..20 {
            after = unread(&reader, scope.clone(), &root_id, vis).await;
            if after == Some(0) {
                break;
            }
            let _ = reader.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        let still_unread = unread(&reader, scope.clone(), &root2_id, vis).await;
        println!(
            "T_PRIVACY {}",
            serde_json::json!({"server": config["server"], "room": room, "root": root,
                "target1": target1.as_str(), "target2": target2.as_str(), "target3": target3.as_str(),
                "root2": root2, "reply2": reply2_id.as_str(),
                "page_while_frozen": true, "premature_ack": premature, "revoked_slot_dropped": true,
                "ack_admitted": matches!(ack1, Some(Ok(true))), "ack_after_toggle": matches!(ack2, Some(Ok(true))),
                "ack_private": matches!(ack3, Some(Ok(true))),
                "public_thread_marker": public_thread, "private_thread_marker": private_thread,
                "public_unthreaded": public_unthreaded, "private_unthreaded": private_unthreaded,
                "other_thread_marker": other_thread_marker,
                "noop_public": noop_public.is_some_and(|r| matches!(r, Ok(false))),
                "noop_private": noop_private.is_some_and(|r| matches!(r, Ok(false))),
                "late_of_revoked": stale, "wrong_scope": wrong_scope, "unknown_room": unknown_room,
                "other_thread_event": other_thread_event, "unread_before": before, "unread_after": after,
                "other_thread_before": other_thread, "other_thread_after": still_unread})
        );
        assert!(matches!(noop_public, Some(Ok(false))), "a no-op sends nothing: {noop_public:?}");
        assert!(matches!(noop_private, Some(Ok(false))), "a no-op sends nothing: {noop_private:?}");
        assert!(wrong_scope && unknown_room && other_thread_event, "invalid inputs are refused");
        assert_eq!(after, Some(0), "only the thread that was read drops");
        assert_eq!(still_unread, other_thread, "the other thread stays unread");
    });
    worker.shutdown().expect("worker stops");
}
/// Room-list badge = main-timeline messages only, plain and encrypted room: thread replies before and
/// after the main message, a read of `main`, then a new reply (no change) and a new main message (+1).
/// Bob sends through the SDK so the encrypted room really carries megolm events.
/// Run: `python3 the auth_smoke script --filter disposable_thread_badge [--server tuwunel]`
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_badge_counts_main_messages_only_plain_and_encrypted() {
    use crate::room_sync::RoomSync;
    use matrix_sdk::ruma::events::room::message::{RoomMessageEventContent as Msg, Relation};
    use matrix_sdk::ruma::events::relation::Thread;
    let config = fixture();
    runtime().block_on(async {
        for (encrypted, n) in [(false, 70u64), (true, 72)] {
            let mut create = serde_json::json!({"invite":["@bob:matrix-desktop.test"], "preset":"private_chat"});
            if encrypted {
                create["initial_state"] = serde_json::json!([{"type":"m.room.encryption","state_key":"",
                    "content":{"algorithm":"m.megolm.v1.aes-sha2"}}]);
            }
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
            let peer_room = peer.get_room(&matrix_sdk::ruma::RoomId::parse(&room).unwrap()).unwrap();
            let root = alice.send_text(&crate::sending::SendRequest { scope: scope.clone(),
                transaction_id: matrix_domain::TransactionId::new(format!("badge-root-{n}")), body: "root".into(),
                attachment: None, poll: None, mentions: Vec::new() }).await.unwrap();
            let root_ev = matrix_sdk::ruma::OwnedEventId::try_from(root.as_str().to_owned()).unwrap();
            let thread_reply = |body: &str| { let mut m = Msg::text_plain(body); m.relates_to = Some(Relation::Thread(Thread::plain(root_ev.clone(), root_ev.clone()))); m };
            let mut badge = None;
            // Every await is bounded: an unbounded one hung the Tuwunel run once in ~5. Waits for the badge to reach `want`; the last value seen is returned either way.
            macro_rules! settle { ($want:expr) => {{
                for _ in 0..12 {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(20), peer.sync_once(matrix_sdk::config::SyncSettings::default().timeout(std::time::Duration::from_millis(300)))).await;
                    if let Ok(Ok(diffs)) = tokio::time::timeout(std::time::Duration::from_secs(30), alice.sliding_round()).await { for d in diffs {
                        if let crate::RoomListDiff::Upsert(r) = d && r.scope.room_id.as_str() == room { badge = Some(r.unread); }
                    } }
                    if badge == Some($want) { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                badge
            }}; }
            let mut seen = Vec::new();
            for b in ["reply one", "reply two"] { {tokio::time::timeout(std::time::Duration::from_secs(40), peer_room.send(thread_reply(b))).await.expect("peer send hung (bounded, see Tuwunel note)").unwrap()}; }
            seen.push(("only_thread_replies", settle!(0)));
            {tokio::time::timeout(std::time::Duration::from_secs(40), peer_room.send(Msg::text_plain("main one"))).await.expect("peer send hung (bounded, see Tuwunel note)").unwrap()};
            {tokio::time::timeout(std::time::Duration::from_secs(40), peer_room.send(thread_reply("reply three"))).await.expect("peer send hung (bounded, see Tuwunel note)").unwrap()};
            seen.push(("main_then_reply", settle!(1)));
            let _ = tokio::time::timeout(std::time::Duration::from_secs(30), alice.mark_main_read(&scope, false)).await;
            seen.push(("main_read", settle!(0)));
            {tokio::time::timeout(std::time::Duration::from_secs(40), peer_room.send(thread_reply("reply four"))).await.expect("peer send hung (bounded, see Tuwunel note)").unwrap()};
            seen.push(("new_reply_after_read", settle!(0)));
            {tokio::time::timeout(std::time::Duration::from_secs(40), peer_room.send(Msg::text_plain("main two"))).await.expect("peer send hung (bounded, see Tuwunel note)").unwrap()};
            seen.push(("new_main_after_read", settle!(1)));
            println!("P336_BADGE {}", serde_json::json!({"server": config["server"], "encrypted": encrypted, "steps": seen}));
            let want = [("only_thread_replies", Some(0)), ("main_then_reply", Some(1)), ("main_read", Some(0)),
                ("new_reply_after_read", Some(0)), ("new_main_after_read", Some(1))];
            assert_eq!(seen, want, "badge counts main messages only (encrypted: {encrypted})");
        }
    });
}

