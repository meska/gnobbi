// Thread panel as the app drives it: `open_thread(.., 0)` = first paint from the cache alone, then `open_thread(.., 50+)`.
// Run: `python3 tools/auth_smoke.py --filter disposable_thread_panel_ [--server tuwunel]`

/// A room with a root by alice and `count` replies by bob, alice synced and its history open.
struct PanelRoom {
    alice: crate::room_sync::RoomSync,
    scope: matrix_domain::RoomScope,
    root: matrix_domain::EventId,
    peer: matrix_sdk::Client,
    root_ev: matrix_sdk::ruma::OwnedEventId,
    config: serde_json::Value,
}

impl PanelRoom {
    async fn new(encrypted: bool, count: usize, n: u64) -> Self {
        use crate::{ViewRequests, room_sync::RoomSync};
        let config = fixture();
        let mut create = serde_json::json!({"invite":["@bob:matrix-desktop.test"], "preset":"private_chat"});
        if encrypted { create["initial_state"] = serde_json::json!([{"type":"m.room.encryption","state_key":"","content":{"algorithm":"m.megolm.v1.aes-sha2"}}]); }
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
        // The send waits 5 s for the server: an encrypted room on a loaded host (keys to query and share) can miss it. A timed-out
        // attempt is aborted by the engine, so trying again leaves no second root.
        let mut sent = Err(crate::sending::SendError::Transport);
        for attempt in 0..3 {
            let request = crate::sending::SendRequest { scope: scope.clone(), transaction_id: matrix_domain::TransactionId::new(format!("panel-root-{n}-{attempt}")), body: "root".into(), attachment: None, poll: None, mentions: Vec::new() };
            sent = alice.send_text(&request).await;
            if sent.is_ok() { break; }
        }
        let sent = sent.unwrap();
        let root_ev = matrix_sdk::ruma::OwnedEventId::try_from(sent.as_str().to_owned()).unwrap();
        let mut this = PanelRoom { alice, scope, root: matrix_domain::EventId::new(sent.as_str()), peer, root_ev, config };
        this.reply(0..count).await;
        this.alice.sliding_round().await.unwrap();
        this.alice.open_history(ViewRequests::new().start(this.scope.clone()).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false }).await.unwrap();
        this
    }

    /// Bob answers in the thread: "reply 000", "reply 001", ...
    async fn reply(&self, range: std::ops::Range<usize>) {
        use matrix_sdk::ruma::events::room::message::{RoomMessageEventContent as Msg, Relation};
        use matrix_sdk::ruma::events::relation::Thread;
        let room = self.peer.get_room(&matrix_sdk::ruma::RoomId::parse(self.scope.room_id.as_str()).unwrap()).unwrap();
        for i in range {
            let mut m = Msg::text_plain(format!("reply {i:03}"));
            m.relates_to = Some(Relation::Thread(Thread::plain(self.root_ev.clone(), self.root_ev.clone())));
            // A loaded host can time a request out; the same transaction id makes the retry idempotent (no duplicate reply).
            let txn = matrix_sdk::ruma::TransactionId::new();
            let mut sent = room.send(m.clone()).with_transaction_id(txn.clone()).await;
            for _ in 0..3 { if sent.is_ok() { break; } sent = room.send(m.clone()).with_transaction_id(txn.clone()).await; }
            sent.unwrap();
        }
    }

    /// The panel's two steps. Returns (first paint, full page) and the time each took.
    async fn open(&self, want: usize) -> (crate::threads::ThreadPage, std::time::Duration) {
        let started = std::time::Instant::now();
        let page = self.alice.open_thread(&self.scope, &self.root, want).await.unwrap();
        (page, started.elapsed())
    }

    /// Asks for `want` until `replies` are there (the thread arrives with the sync), at most `secs`.
    async fn open_until(&mut self, want: usize, replies: usize, secs: u64) -> crate::threads::ThreadPage {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        loop {
            let (page, _) = self.open(want).await;
            if page.replies.len() >= replies || std::time::Instant::now() > end { return page; }
            let _ = self.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
}

fn panel_bodies(p: &crate::threads::ThreadPage) -> Vec<String> { p.replies.iter().map(|r| r.body.clone()).collect() }

/// Criterion 1 and the «no replies» edge: the root is there from the cache alone (no replies, no network), the full ask
/// of a thread without replies brings the root and nothing else, and a reply that comes later is picked up.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_root_at_first_paint_and_thread_without_replies() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 0, 300).await;
        let (first, took) = room.open(0).await;
        println!("PANEL_NO_REPLIES {}", serde_json::json!({"first_ms": took.as_millis() as u64, "root": first.root.is_some()}));
        assert_eq!(first.root.as_ref().map(|r| r.body.as_str()), Some("root"), "root in the first answer, before any reply");
        assert!(first.replies.is_empty());
        assert!(took.as_millis() < 100, "the first paint waited {took:?}");
        let (full, took) = room.open(50).await;
        println!("PANEL_NO_REPLIES {}", serde_json::json!({"full_ms": took.as_millis() as u64, "at_start": full.at_start}));
        assert_eq!(full.root.as_ref().map(|r| r.body.as_str()), Some("root"));
        assert!(full.replies.is_empty(), "no reply invented");
        room.reply(0..1).await;
        let page = room.open_until(50, 1, 10).await;
        assert_eq!(panel_bodies(&page), ["reply 000"]);
        assert_eq!(page.root.as_ref().map(|r| r.body.as_str()), Some("root"));
    });
}

/// The first reply in a thread that had none (ours, then the other user's) is read at once: the cache holds as many
/// replies as the root declares, so there is nothing to page (the SDK would wait 3 s for a first pagination token,
/// and a 1 s cut of that wait used to cost 2.2 s and then 1.2 s). Measured before the fix: 2154-2157 ms, then 1155-1158 ms.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_first_reply_in_an_empty_thread_reads_at_once() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 0, 335).await;
        assert!(room.open(50).await.0.replies.is_empty());
        room.alice.send_in_thread(&room.scope, &room.root, None, "mine", None).await.unwrap();
        let mut times = Vec::new();
        let mut page = room.open(50).await;
        for _ in 0..50 {
            times.push(page.1.as_millis() as u64);
            if page.0.replies.len() == 1 { break; }
            let _ = room.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            page = room.open(50).await;
        }
        println!("PANEL_FIRST_REPLY {}", serde_json::json!({"own_ms": times}));
        assert_eq!(panel_bodies(&page.0), ["mine"]);
        assert!(times.iter().all(|t| *t < 100), "a read after our first reply took {times:?} ms");
        room.reply(0..1).await;
        let mut times = Vec::new();
        for _ in 0..50 {
            let page = room.open(50).await;
            times.push(page.1.as_millis() as u64);
            if page.0.replies.len() == 2 { break; }
            let _ = room.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        println!("PANEL_FIRST_REPLY {}", serde_json::json!({"other_ms": times}));
        assert!(times.iter().all(|t| *t < 100), "a read after the other user's reply took {times:?} ms");
    });
}

/// The full ask on a thread without replies (a plain message opened as a thread) answers at once: the room shows the root
/// without a thread summary, so there is nothing to page (the SDK would wait 3 s for a first pagination token that a
/// thread cache without events never gets). Measured before the fix: 3010, 3039, 3009 ms.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_empty_thread_answers_at_once() {
    runtime().block_on(async {
        let room = PanelRoom::new(false, 0, 305).await;
        let (full, took) = room.open(50).await;
        println!("PANEL_EMPTY_FULL {}", serde_json::json!({"ms": took.as_millis() as u64, "root": full.root.is_some(), "at_start": full.at_start}));
        assert!(full.root.is_some() && full.replies.is_empty());
        assert!(took.as_millis() < 100, "a thread without replies waited {took:?}");
    });
}

/// Criterion 3: the second opening of the same thread comes from the cache (replies at once, no new full load).
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_second_open_comes_from_the_cache() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 5, 310).await;
        let first = room.open_until(50, 5, 10).await;
        assert_eq!(first.replies.len(), 5, "first open loads the thread");
        // Closed and opened again: the panel's first paint (want 0) already holds the whole content.
        let (cached, took) = room.open(0).await;
        println!("PANEL_CACHE {}", serde_json::json!({"cache_ms": took.as_millis() as u64, "replies": cached.replies.len()}));
        assert_eq!(panel_bodies(&cached), panel_bodies(&first), "same content, immediately");
        assert_eq!(cached.root.as_ref().map(|r| r.body.as_str()), Some("root"));
        assert!(took.as_millis() < 100, "cache-only answer took {took:?}");
        // And the full ask that follows does not start over.
        let (again, took) = room.open(50).await;
        assert_eq!(panel_bodies(&again), panel_bodies(&first));
        assert!(took.as_millis() < 250, "second full open took {took:?}");
    });
}

/// 50 replies in one page, and a thread of 120 read backwards in steps: oldest first, no hole, no duplicate.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_fifty_replies_and_paging_backwards() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 50, 320).await;
        let page = room.open_until(50, 50, 20).await;
        let expected: Vec<String> = (0..50).map(|i| format!("reply {i:03}")).collect();
        assert_eq!(panel_bodies(&page), expected, "50 replies, oldest first");
        assert_eq!(page.root.as_ref().and_then(|r| r.thread.as_ref()).map(|t| t.num_replies), Some(50));
        let (cached, took) = room.open(0).await;
        assert_eq!(panel_bodies(&cached), expected);
        assert!(took.as_millis() < 100, "cache-only answer took {took:?}");
        // 70 more on top: the panel asks 50 more each time it is scrolled to the top.
        room.reply(50..120).await;
        let _ = room.alice.sliding_round().await;
        let (mut steps, mut want, mut last) = (Vec::new(), 50usize, 0usize);
        let page = loop {
            let page = room.open_until(want, (want).min(120), 10).await;
            steps.push((want, page.replies.len()));
            if page.replies.len() >= 120 || page.replies.len() <= last || want > 250 { break page; }
            last = page.replies.len();
            want += 50;
        };
        println!("PANEL_PAGING {}", serde_json::json!({"server": room.config["server"], "steps": steps}));
        let bodies = panel_bodies(&page);
        let mut sorted = bodies.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted, bodies, "oldest first, no duplicate, no root among the replies");
        assert!(steps.windows(2).all(|w| w[1].1 >= w[0].1), "pages only grow: {steps:?}");
        if room.config["server"] == "synapse" {
            assert_eq!(bodies.len(), 120, "whole thread after the backward steps: {steps:?}");
            assert_eq!(bodies.first().map(String::as_str), Some("reply 000"));
            assert_eq!(bodies.last().map(String::as_str), Some("reply 119"));
        } else {
            assert!(bodies.len() >= 50, "at least the first page (Tuwunel may leave holes in big threads)");
        }
    });
}

/// Encrypted room, keys present: the replies are shown decrypted, the first paint does not wait for the network.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_encrypted_room_decrypts() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(true, 5, 330).await;
        let mut page = room.open_until(50, 5, 20).await;
        // The last reply can be in the rows before its key: wait for the decryption, not only for the rows.
        let end = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while page.replies.iter().any(|r| r.utd) && std::time::Instant::now() < end {
            let _ = room.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            page = room.open(50).await.0;
        }
        assert_eq!(panel_bodies(&page), ["reply 000", "reply 001", "reply 002", "reply 003", "reply 004"]);
        assert!(page.replies.iter().all(|r| !r.utd));
        assert_eq!(page.root.as_ref().map(|r| r.body.as_str()), Some("root"));
        let (cached, took) = room.open(0).await;
        assert_eq!(panel_bodies(&cached), panel_bodies(&page));
        assert!(took.as_millis() < 100, "cache-only answer took {took:?}");
    });
}

/// Encrypted room read by a device that holds no key (the messages cannot be decrypted yet): the panel still answers
/// quickly with rows flagged `utd`, never hangs, never fails, and does not show the ciphertext as text.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_encrypted_messages_not_yet_decrypted() {
    use crate::{ViewRequests, room_sync::RoomSync};
    runtime().block_on(async {
        let room = PanelRoom::new(true, 3, 340).await;
        // A second device of alice with a store of its own: it joined after the messages and has no room key.
        let mut late = RoomSync::new(login(&room.config, "alice", 349).await);
        for _ in 0..3 { late.sliding_round().await.unwrap(); }
        late.open_history(ViewRequests::new().start(matrix_domain::RoomScope { account_id: AccountId::new(349), room_id: room.scope.room_id.clone() }).unwrap(), crate::receipts::ReadVisibility { window_focused: true, room_foreground: true, bottom_gap_px: 0.0, public_receipts: false }).await.unwrap();
        let scope = matrix_domain::RoomScope { account_id: AccountId::new(349), room_id: room.scope.room_id.clone() };
        for want in [0usize, 50, 0] {
            let started = std::time::Instant::now();
            let answer = tokio::time::timeout(std::time::Duration::from_secs(5), late.open_thread(&scope, &room.root, want)).await;
            let took = started.elapsed();
            let page = answer.expect("the panel does not hang on undecrypted messages").expect("no error either");
            println!("PANEL_UTD {}", serde_json::json!({"want": want, "ms": took.as_millis() as u64, "root": page.root.as_ref().map(|r| r.utd), "replies": page.replies.iter().map(|r| r.utd).collect::<Vec<_>>()}));
            assert!(page.replies.iter().all(|r| r.utd && r.body != "reply 000" && !r.body.contains("reply")), "undecrypted replies show as such");
            if want == 0 { assert!(took.as_millis() < 100, "cache-only answer took {took:?}"); }
        }
    });
}

/// Closing and reopening the panel quickly: asks abandoned half way (the panel closed) and asks in parallel (reopened
/// at once) leave the thread whole, in order, without a duplicate.
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_close_and_reopen_quickly() {
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 5, 350).await;
        let want: Vec<String> = (0..5).map(|i| format!("reply {i:03}")).collect();
        for round in 0..10u64 {
            // Panel opened and closed before the answer: the pending ask is dropped.
            let _ = tokio::time::timeout(std::time::Duration::from_millis(round), room.alice.open_thread(&room.scope, &room.root, if round % 2 == 0 { 0 } else { 50 })).await;
        }
        // Three openings at the same time (close, open, close, open).
        let (a, b, c) = tokio::join!(
            room.alice.open_thread(&room.scope, &room.root, 0),
            room.alice.open_thread(&room.scope, &room.root, 50),
            room.alice.open_thread(&room.scope, &room.root, 50),
        );
        for page in [a.unwrap(), b.unwrap(), c.unwrap()] {
            assert_eq!(page.root.as_ref().map(|r| r.body.as_str()), Some("root"));
            let bodies = panel_bodies(&page);
            let mut sorted = bodies.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted, bodies, "ordered, no duplicate");
            assert!(bodies.iter().all(|b| want.contains(b)), "only the thread's own replies: {bodies:?}");
        }
        let page = room.open_until(50, 5, 10).await;
        assert_eq!(panel_bodies(&page), want, "whole thread after the churn");
        let (cached, took) = room.open(0).await;
        assert_eq!(panel_bodies(&cached), want);
        assert!(took.as_millis() < 100, "cache-only answer took {took:?}");
    });
}

/// A reply edited or deleted by its author reaches the panel, open (the next read) and reopened (the first paint from the cache).
#[test]
#[ignore = "requires disposable homeserver; needs the external test harness (not in this repo)"]
fn disposable_thread_panel_edit_and_delete_of_a_reply_reach_the_panel() {
    use matrix_sdk::ruma::events::room::message::{ReplacementMetadata, RoomMessageEventContent as Msg};
    runtime().block_on(async {
        let mut room = PanelRoom::new(false, 3, 360).await;
        let page = room.open_until(50, 3, 10).await;
        assert_eq!(panel_bodies(&page), ["reply 000", "reply 001", "reply 002"]);
        let ids: Vec<_> = page.replies.iter().map(|r| matrix_sdk::ruma::OwnedEventId::try_from(r.event_id.as_str().to_owned()).unwrap()).collect();
        let bob = room.peer.get_room(&matrix_sdk::ruma::RoomId::parse(room.scope.room_id.as_str()).unwrap()).unwrap();
        bob.send(Msg::text_plain("reply 001 edited").make_replacement(ReplacementMetadata::new(ids[1].clone(), None))).await.unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let edited = loop {
            let (p, _) = room.open(50).await;
            if panel_bodies(&p).contains(&"reply 001 edited".to_owned()) || std::time::Instant::now() > end { break p; }
            let _ = room.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        };
        assert_eq!(panel_bodies(&edited), ["reply 000", "reply 001 edited", "reply 002"], "edit while the panel is open");
        assert_eq!(panel_bodies(&room.open(0).await.0), panel_bodies(&edited), "edit seen from the cache on reopening");
        bob.redact(&ids[2], None, None).await.unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let deleted = loop {
            let (p, _) = room.open(50).await;
            if p.replies.iter().all(|r| r.body != "reply 002") || std::time::Instant::now() > end { break p; }
            let _ = room.alice.sliding_round().await;
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        };
        let gone = |p: &crate::threads::ThreadPage| p.replies.iter().all(|r| r.body != "reply 002");
        assert!(gone(&deleted), "deleted reply still shows its text while the panel is open: {:?}", panel_bodies(&deleted));
        assert!(gone(&room.open(0).await.0), "deleted reply comes back on reopening");
    });
}
