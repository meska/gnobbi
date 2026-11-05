// RED tests: opening / reading an OLD chat must not move it in the room list.
// Order comes from the SDK (RoomListService sorter); the app only projects `RoomListEvent::Order`.
// Fixture: three rooms where bob is the user; OLD got its messages first, then MID, then NEW, so the
// SDK order is [NEW, MID, OLD]. Each scenario acts on OLD only and the order must stay the same.
// Limit: `sliding::run` borrows the RoomSync mutably, so actions that need it (open, mark read)
// run between two runs; the second run reads the order back from the store plus a fresh server sync.
// Receipts go over HTTP while the run is live. Not distinguished: desktop/mobile (engine is shared).
// Run one scenario at a time (a failing one aborts the runner), per server:
//   the ct script 'python3 the auth_smoke script --filter disposable_old_room_live_open --server tuwunel'
// (needs CARGO_TARGET_DIR=$PWD/target when the tree is a CT_SRC worktree). `disposable_old_room_live_*`
// act while the list is live (the app's case); the others act between two runs (read back after restart).

struct OldRoom {
    config: serde_json::Value,
    sync: crate::room_sync::RoomSync,
    root: std::path::PathBuf,
    /// Room ids: OLD (oldest last message), MID, NEW.
    ids: [String; 3],
    old_last_event: String,
    old_scope: matrix_domain::RoomScope,
    before: Vec<String>,
}

/// Order (most recent first, limited to the fixture rooms) after one live run; `action` fires at `Ready`.
async fn order_in_run(
    sync: &mut crate::room_sync::RoomSync,
    ids: &[String; 3],
    action: impl std::future::Future<Output = ()>,
) -> Vec<String> {
    use crate::{RoomListEvent, RoomSyncStatus};
    use std::time::Duration;
    let (mut tx, mut rx) = crate::bounded(std::num::NonZeroUsize::new(1024).unwrap());
    let collect = async {
        let (mut ready, mut action, mut order) = (false, Some(action), Vec::new());
        loop {
            let wait = Duration::from_secs(if ready { 3 } else { 30 });
            match tokio::time::timeout(wait, rx.recv()).await {
                Ok(Some(RoomListEvent::Status(RoomSyncStatus::Ready))) => {
                    ready = true;
                    if let Some(action) = action.take() {
                        action.await;
                    }
                }
                Ok(Some(RoomListEvent::Order(o))) => {
                    order = o.iter().map(|s| s.room_id.as_str().to_owned()).collect::<Vec<_>>();
                }
                Ok(Some(_)) => {}
                _ => return order,
            }
        }
    };
    let order = tokio::select! {
        () = crate::sliding::run(sync, &mut tx, || {}) => panic!("sliding run ended early"),
        order = collect => order,
    };
    order.into_iter().filter(|id| ids.contains(id)).collect()
}

async fn old_room_fixture(account: u64) -> OldRoom {
    old_room_fixture_with(account, false).await
}

/// `redact_mid`: MID gets two messages and the last one is redacted, so the SDK has no valid
/// latest event for it until it back-paginates (the option set in auth.rs / persistence.rs).
async fn old_room_fixture_with(account: u64, redact_mid: bool) -> OldRoom {
    use crate::room_sync::RoomSync;
    use std::time::Duration;
    const OLD_BURST: usize = 30; // two history pages of 25
    let config = fixture();
    let mut ids = Vec::new();
    let mut old_last_event = String::new();
    for (n, burst) in [(0, OLD_BURST), (1, if redact_mid { 2 } else { 1 }), (2, 1)] {
        let created = room_request(&config, "alice", reqwest::Method::POST, "createRoom",
            serde_json::json!({"invite":["@bob:matrix-desktop.test"],"name":format!("old-room-order-{n}")})).await;
        let room = created["room_id"].as_str().unwrap().to_owned();
        room_request(&config, "bob", reqwest::Method::POST, &format!("join/{room}"), serde_json::json!({})).await;
        for i in 0..burst {
            let sent = room_request(&config, "alice", reqwest::Method::PUT,
                &format!("rooms/{room}/send/m.room.message/order-{n}-{i}"),
                serde_json::json!({"msgtype":"m.text","body":format!("m{n}-{i}")})).await;
            if n == 0 {
                old_last_event = sent["event_id"].as_str().unwrap().to_owned();
            }
            if redact_mid && n == 1 && i == burst - 1 {
                let event = sent["event_id"].as_str().unwrap();
                room_request(&config, "alice", reqwest::Method::PUT, &format!("rooms/{room}/redact/{event}/redact-mid"), serde_json::json!({})).await;
            }
        }
        ids.push(room);
        tokio::time::sleep(Duration::from_millis(1200)).await; // recency stamps must differ
    }
    let ids: [String; 3] = ids.try_into().unwrap();
    let root = std::env::temp_dir().join(format!("gnobbi-test-oldorder-{account}-{}", std::process::id()));
    crate::fsx::create_private_dir(&root).unwrap();
    let bob = AccountStore::create(&root, AccountId::new(account)).unwrap()
        .password_login_with_http(
            HomeserverUrl::parse(config["url"].as_str().unwrap()).unwrap(),
            "@bob:matrix-desktop.test",
            config["bob_password"].as_str().unwrap(),
            "long disposable fixture passphrase",
            http(&config),
        )
        .await
        .expect("fixture login accepted");
    let mut sync = RoomSync::new(bob);
    let before = order_in_run(&mut sync, &ids, async {}).await;
    let old_scope = matrix_domain::RoomScope {
        account_id: AccountId::new(account),
        room_id: matrix_domain::RoomId::new(ids[0].clone()),
    };
    OldRoom { config, sync, root, ids, old_last_event, old_scope, before }
}

impl OldRoom {
    /// Open OLD like the app (`open_history`); `seen` = window focused, so it also sends receipts.
    async fn open(&self, seen: bool) -> crate::history::HistoryPage {
        let visibility = crate::receipts::ReadVisibility { window_focused: seen, room_foreground: seen, bottom_gap_px: 0.0, public_receipts: true };
        self.sync.open_history(crate::ViewRequests::new().start(self.old_scope.clone()).unwrap(), visibility).await.unwrap().0
    }

    /// Back on the list: read the order again and compare with the starting one.
    async fn back_to_list(mut self, action: impl std::future::Future<Output = ()>, what: &str) {
        let after = order_in_run(&mut self.sync, &self.ids, action).await;
        let [old, mid, new] = self.ids.clone();
        let (before, root) = (self.before.clone(), self.root.clone());
        drop(self);
        let _ = std::fs::remove_dir_all(&root); // before the assert: a RED panic must not leave state
        assert_eq!(before, [new, mid, old], "precondition: SDK order is newest first");
        assert_eq!(after, before, "{what} moved the old chat in the room list (before {before:?}, after {after:?})");
    }
}

/// The user flow: open the old chat (history + automatic receipts), go back to the list.
#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_open_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(71).await;
        assert!(!f.open(true).await.events.is_empty());
        f.back_to_list(async {}, "opening the old chat").await;
    });
}

/// (a) «Segna come letto» on the old chat.
#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_mark_read_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(72).await;
        f.open(false).await; // fills the event cache the mark takes the newest event from
        f.sync.mark_room_for_test(&f.old_scope, true).await.unwrap();
        f.back_to_list(async {}, "«Segna come letto»").await;
    });
}

/// (b) Timeline opening with history loading: first page, then older pages, no receipts.
#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_history_pagination_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(73).await;
        let mut next = f.open(false).await.next;
        for _ in 0..5 {
            let Some(token) = next else { break };
            next = f.sync.history_page(crate::ViewRequests::new().start(f.old_scope.clone()).unwrap(), Some(token)).await.unwrap().next;
        }
        f.back_to_list(async {}, "loading the old chat's history").await;
    });
}

/// (c) A receipt for the old chat's last event, sent while the list is live.
async fn receipt_keeps_position(account: u64, path: impl Fn(&str, &str) -> String, what: &str, body: impl Fn(&str) -> serde_json::Value) {
    let f = old_room_fixture(account).await;
    let (config, room, event) = (f.config.clone(), f.ids[0].clone(), f.old_last_event.clone());
    let send = async { room_request(&config, "bob", reqwest::Method::POST, &path(&room, &event), body(&event)).await; };
    f.back_to_list(send, what).await;
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_public_receipt_keeps_position() {
    runtime().block_on(receipt_keeps_position(74, |r, e| format!("rooms/{r}/receipt/m.read/{e}"), "a public read receipt", |_| serde_json::json!({})));
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_private_receipt_keeps_position() {
    runtime().block_on(receipt_keeps_position(75, |r, e| format!("rooms/{r}/receipt/m.read.private/{e}"), "a private read receipt", |_| serde_json::json!({})));
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_fully_read_marker_keeps_position() {
    runtime().block_on(receipt_keeps_position(76, |r, _| format!("rooms/{r}/read_markers"), "a fully-read marker", |e| serde_json::json!({"m.fully_read": e})));
}

// Live variants: the same app calls, made while `sliding::run` is live (the app's situation),
// through owned handles to the same client and timeline cache.
impl OldRoom {
    /// `actions` gets the client, the timeline cache and OLD's scope.
    async fn live(self, what: &str, actions: impl AsyncFnOnce(matrix_sdk::Client, crate::room_sync::Timelines, matrix_domain::RoomScope)) {
        let (client, timelines) = self.sync.live_handles_for_test();
        let scope = self.old_scope.clone();
        let run = actions(client, timelines, scope);
        self.back_to_list(run, what).await;
    }
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_live_open_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(77).await;
        f.live("opening the old chat while the list is live", async |client, timelines, scope| {
            let tag = crate::ViewRequests::new().start(scope.clone()).unwrap();
            let (page, _) = crate::room_sync::open_page(&client, &timelines, tag).await.unwrap();
            assert!(!page.events.is_empty());
            let id = matrix_sdk::ruma::RoomId::parse(scope.room_id.as_str()).unwrap();
            let timeline = timelines.get(&client.get_room(&id).unwrap()).await.unwrap();
            use matrix_sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
            timeline.mark_as_read(ReceiptType::Read).await.unwrap();
            timeline.mark_as_read(ReceiptType::FullyRead).await.unwrap();
        }).await;
    });
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_live_mark_read_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(78).await;
        let account = f.old_scope.account_id;
        f.live("«Segna come letto» while the list is live", async |client, timelines, scope| {
            let tag = crate::ViewRequests::new().start(scope.clone()).unwrap();
            crate::room_sync::open_page(&client, &timelines, tag).await.unwrap();
            crate::operations::mark_room(&client, &timelines, account, &scope, true, true).await.unwrap();
        }).await;
    });
}

#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_live_history_pagination_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture(79).await;
        let account = f.old_scope.account_id;
        f.live("loading the old chat's history while the list is live", async |client, timelines, scope| {
            let mut views = crate::ViewRequests::new(); // dropping it cancels its tags
            let tag = views.start(scope.clone()).unwrap();
            let mut next = crate::room_sync::open_page(&client, &timelines, tag).await.unwrap().0.next;
            for _ in 0..5 {
                let Some(token) = next else { break };
                let tag = views.start(scope.clone()).unwrap();
                next = crate::room_sync::fetch_history(&client, &timelines, account, tag, Some(token)).await.unwrap().next;
            }
        }).await;
    });
}

/// Control: the harness does see a move. A new message in OLD puts it first; if this fails the
/// green scenarios above prove nothing.
#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_control_new_message_moves_it_to_top() {
    runtime().block_on(async {
        let mut f = old_room_fixture(80).await;
        let (config, ids) = (f.config.clone(), f.ids.clone());
        let send = async { room_request(&config, "alice", reqwest::Method::PUT, &format!("rooms/{}/send/m.room.message/control", ids[0]), serde_json::json!({"msgtype":"m.text","body":"fresh"})).await; };
        let after = order_in_run(&mut f.sync, &ids, send).await;
        let _ = std::fs::remove_dir_all(&f.root);
        assert_eq!(after, [ids[0].clone(), ids[2].clone(), ids[1].clone()], "a new message must move OLD to the top (before {:?})", f.before);
    });
}

/// Production clients only (auth.rs login, persistence.rs restore): MID's last message is redacted,
/// MID must keep its place, and opening OLD must not move anything. Removing the SDK option from
/// either Client builder turns this RED; the test builds no Client of its own.
#[test]
#[ignore = "requires disposable homeserver; run via the auth_smoke script"]
fn disposable_old_room_redacted_last_event_keeps_position() {
    runtime().block_on(async {
        let f = old_room_fixture_with(81, true).await;
        let [old, mid, new] = f.ids.clone();
        assert_eq!(f.before, [new, mid, old], "redacted last event pushed MID out of place");
        assert!(!f.open(true).await.events.is_empty());
        f.back_to_list(async {}, "opening the old chat with a redacted MID").await;
    });
}
