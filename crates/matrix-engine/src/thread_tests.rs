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
