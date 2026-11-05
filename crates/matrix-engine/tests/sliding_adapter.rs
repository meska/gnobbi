//! The sliding-sync adapter shows room previews and activity for live messages, on a real
//! homeserver. Driven by the sliding_adapter_probe script (disposable Synapse/Tuwunel or the owner
//! Tuwunel); the fixture config comes from the environment, no secret lives here.
#![recursion_limit = "512"]
use matrix_domain::AccountId;
use matrix_engine::{
    RoomListDiff, RoomListEvent, RoomSummary, RoomSyncStatus, bounded,
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            api::client::room::create_room::v3::{Request as CreateRoom, RoomPreset},
            assign,
            events::room::message::RoomMessageEventContent,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{collections::BTreeMap, num::NonZeroUsize, time::Duration};

const WAIT: Duration = Duration::from_secs(60);

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let mut builder = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .sliding_sync_version_builder(VersionBuilder::None)
        // Same as the production clients (auth.rs, persistence.rs).
        .with_enable_automatic_back_pagination(true);
    if let Some(ca) = config["ca"].as_str() {
        builder = builder.add_root_certificates(vec![
            Certificate::from_pem(&std::fs::read(ca).unwrap()).unwrap(),
        ]);
    }
    let client = builder.build().await.unwrap();
    client
        .matrix_auth()
        .login_username(
            config[format!("{who}_mxid")].as_str().unwrap(),
            config[format!("{who}_password")].as_str().unwrap(),
        )
        .send()
        .await
        .unwrap();
    client
}

#[test]
#[ignore = "requires the sliding_adapter_probe script"]
fn adapter_matches_the_classic_loop_and_follows_new_messages() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let account = AccountId::new(7);
        step("login");
        let (classic_client, sliding_client, bob) = (
            login(&config, "alice").await,
            login(&config, "alice").await,
            login(&config, "bob").await,
        );
        // The classic client also joins (it is not syncing yet): fewer logins, the owner server throttles them.
        let admin = classic_client.clone();
        step("rooms");
        let room = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("adapter-probe-{}", suffix())),
                invite: vec![admin.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        admin.join_room_by_id(room.room_id()).await.unwrap();
        room.send(RoomMessageEventContent::text_plain("first"))
            .await
            .unwrap();
        let id = room.room_id().to_string();

        step("adapter");
        // Adapter, on a client whose sliding version is unset (an existing profile).
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(64).unwrap());
        let mut adapter = RoomSync::from_client(account, sliding_client.clone());
        let run = sliding::run(&mut adapter, &mut tx, || {});
        tokio::pin!(run);
        let mut seen = Seen::default();
        tokio::select! {
            _ = &mut run => panic!("adapter ended"),
            () = collect(&mut rx, &mut seen, &id, "first", false) => {}
        }
        assert!(seen.ready, "Ready never came");
        let first_ts = seen.rooms[&id].info.last_event_ts;
        assert!(first_ts.is_some(), "no last_event_ts for a room with a message");

        step("second");
        // A new message: preview and activity follow, .
        room.send(RoomMessageEventContent::text_plain("second"))
            .await
            .unwrap();
        tokio::select! {
            _ = &mut run => panic!("adapter ended"),
            () = collect(&mut rx, &mut seen, &id, "second", true) => {}
        }
        assert!(
            seen.activity.iter().any(|s| s.room_id.as_str() == id),
            "no TimelineActivity for the new message"
        );
        assert!(seen.rooms[&id].info.last_event_ts > first_ts, "last_event_ts did not follow the new message");

        step("cleanup");
        // Leave only what this run created, then revoke the devices.
        for client in [&admin, &bob] {
            if let Some(r) = client.get_room(room.room_id()) {
                let _ = r.leave().await;
                let _ = r.forget().await;
            }
        }
        for client in [classic_client, sliding_client, bob] {
            let _ = client.matrix_auth().logout().await;
        }
    });
}

#[test]
#[ignore = "requires the sliding_adapter_probe script"]
// pump! assigns `ready` on every use; only the first pump reads it.
#[allow(unused_assignments)]
fn push_rules_decide_alerts_and_the_room_mute_is_a_server_rule() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let account = AccountId::new(7);
        let (alice, bob) = (login(&config, "alice").await, login(&config, "bob").await);
        let room = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("notify-probe-{}", suffix())),
                invite: vec![alice.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        alice.join_room_by_id(room.room_id()).await.unwrap();
        room.send(RoomMessageEventContent::text_plain("old")).await.unwrap();
        // The cutoff is the adapter's start; the server stamps whole milliseconds on its own clock.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let scope = matrix_domain::RoomScope {
            account_id: account,
            room_id: matrix_domain::RoomId::new(room.room_id().as_str()),
        };

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(64).unwrap());
        let mut adapter = RoomSync::from_client(account, alice.clone());
        let run = sliding::run(&mut adapter, &mut tx, || {});
        tokio::pin!(run);
        // Next event of the adapter's stream; Alerts and the muted flag are collected on the way.
        let mut alerts = 0;
        let mut muted = None;
        let mut ready = false;
        macro_rules! pump {
            ($for:expr, $until:expr) => {{
                let end = tokio::time::Instant::now() + $for;
                loop {
                    tokio::select! {
                        _ = &mut run => panic!("adapter ended"),
                        _ = tokio::time::sleep_until(end) => break,
                        event = rx.recv() => match event.unwrap() {
                            RoomListEvent::Status(RoomSyncStatus::Ready) => ready = true,
                            RoomListEvent::Alert(s) => { assert_eq!(s, scope); alerts += 1 }
                            RoomListEvent::Diff(RoomListDiff::Upsert(r)) if r.scope == scope => muted = Some(r.info.muted),
                            _ => {}
                        },
                    }
                    if $until(&alerts, &muted) { break }
                }
            }};
        }
        pump!(WAIT, |_: &i32, _: &Option<bool>| ready && muted.is_some());
        assert!(ready, "Ready never came");
        assert_eq!(alerts, 0, "history replayed at start must not alert");
        assert_eq!(muted, Some(false));

        step("push rule alerts");
        room.send(RoomMessageEventContent::text_plain("ping")).await.unwrap();
        pump!(WAIT, |a: &i32, _: &Option<bool>| *a >= 1);
        assert_eq!(alerts, 1, "a new message of someone else notifies once");
        let own = alice.get_room(room.room_id()).unwrap();
        own.send(RoomMessageEventContent::text_plain("mine")).await.unwrap();
        pump!(Duration::from_secs(5), |_: &i32, _: &Option<bool>| false);
        assert_eq!(alerts, 1, "own messages do not notify");

        step("mute");
        matrix_engine::notify::set_muted(&alice, account, &scope, true).await.unwrap();
        pump!(WAIT, |_: &i32, m: &Option<bool>| *m == Some(true));
        assert_eq!(muted, Some(true), "the mute rule is projected");
        room.send(RoomMessageEventContent::text_plain("ping muted")).await.unwrap();
        pump!(Duration::from_secs(8), |_: &i32, _: &Option<bool>| false);
        assert_eq!(alerts, 1, "a muted room does not notify");

        step("unmute");
        matrix_engine::notify::set_muted(&alice, account, &scope, false).await.unwrap();
        pump!(WAIT, |_: &i32, m: &Option<bool>| *m == Some(false));
        assert_eq!(muted, Some(false));
        room.send(RoomMessageEventContent::text_plain("ping again")).await.unwrap();
        pump!(WAIT, |a: &i32, _: &Option<bool>| *a >= 2);
        assert_eq!(alerts, 2, "unmuted again: notifies");

        for client in [&alice, &bob] {
            if let Some(r) = client.get_room(room.room_id()) {
                let _ = r.leave().await;
                let _ = r.forget().await;
            }
        }
        for client in [alice, bob] {
            let _ = client.matrix_auth().logout().await;
        }
    });
}

/// Does opening an OLD room reorder the room list? Two rooms, old activity in the first,
/// newer in the second; the adapter runs, the old one is opened through `open_history`, and
/// every `Order`/`Diff` around the open is printed. `REORDER_NO_RECEIPTS=1` opens with a
/// visibility that does not see the last message (no receipts) to isolate the trigger.
/// Asserts the order is unchanged by the open (regression for the dormant-room jump).
#[test]
#[ignore = "requires the sliding_adapter_probe script"]
fn opening_an_old_room_reorders_the_list() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let no_receipts = std::env::var("REORDER_NO_RECEIPTS").is_ok_and(|v| !v.is_empty());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let account = AccountId::new(7);
        step("login");
        let (sliding_client, alice, bob) = (
            login(&config, "alice").await,
            login(&config, "alice").await,
            login(&config, "bob").await,
        );

        step("rooms: old, dormant (last event redacted), then new");
        let sfx = suffix();
        let old = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("zz-old-{sfx}")),
                invite: vec![alice.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        alice.join_room_by_id(old.room_id()).await.unwrap();
        old.send(RoomMessageEventContent::text_plain("old-msg-1")).await.unwrap();
        old.send(RoomMessageEventContent::text_plain("old-msg-2")).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let dormant = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("mm-dormant-{sfx}")),
                invite: vec![alice.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        alice.join_room_by_id(dormant.room_id()).await.unwrap();
        dormant.send(RoomMessageEventContent::text_plain("dormant-msg-1")).await.unwrap();
        let redacted = dormant
            .send(RoomMessageEventContent::text_plain("dormant-msg-2"))
            .await
            .unwrap();
        dormant.redact(&redacted.response.event_id, None, None).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let new = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(format!("aa-new-{sfx}")),
                invite: vec![alice.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        alice.join_room_by_id(new.room_id()).await.unwrap();
        new.send(RoomMessageEventContent::text_plain("new-msg-1")).await.unwrap();
        new.send(RoomMessageEventContent::text_plain("new-msg-2")).await.unwrap();
        let (old_id, dormant_id, new_id) = (
            old.room_id().to_string(),
            dormant.room_id().to_string(),
            new.room_id().to_string(),
        );

        step("adapter");
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(1024).unwrap());
        let mut adapter = RoomSync::from_client(account, sliding_client.clone());
        // The opener shares the client (event cache, sync); only its timeline cache is its own.
        let opener = RoomSync::from_client(account, sliding_client.clone());
        let run = sliding::run(&mut adapter, &mut tx, || {});
        tokio::pin!(run);
        let mut seen = Seen::default();
        let mut reqs = matrix_engine::ViewRequests::new();
        let tag = reqs
            .start(matrix_domain::RoomScope {
                account_id: account,
                room_id: matrix_domain::RoomId::new(dormant_id.clone()),
            })
            .unwrap();
        let visibility = matrix_engine::receipts::ReadVisibility {
            window_focused: !no_receipts,
            room_foreground: true,
            bottom_gap_px: 0.0,
            public_receipts: true,
        };
        tokio::select! {
            _ = &mut run => panic!("adapter ended"),
            () = reorder_probe(&mut rx, &mut seen, &opener, &sliding_client, &old_id, &dormant_id, &new_id, tag, visibility) => {}
        }

        step("cleanup");
        for room_id in [old.room_id(), dormant.room_id(), new.room_id()] {
            for client in [&alice, &bob] {
                if let Some(r) = client.get_room(room_id) {
                    let _ = r.leave().await;
                    let _ = r.forget().await;
                }
            }
        }
        for client in [&alice, &bob] {
            let _ = client.matrix_auth().logout().await;
        }
        let _ = sliding_client.matrix_auth().logout().await;
    });
}

/// Pump the adapter stream through the probe phases, printing every order change.
async fn reorder_probe(
    rx: &mut matrix_engine::Receiver<RoomListEvent>,
    seen: &mut Seen,
    opener: &RoomSync,
    client: &Client,
    old_id: &str,
    dormant_id: &str,
    new_id: &str,
    tag: matrix_engine::RequestTag,
    visibility: matrix_engine::receipts::ReadVisibility,
) {
    let start = std::time::Instant::now();
    let at = |what: &str| format!("[{:>5.1}s] {what}", start.elapsed().as_secs_f32());
    let label_of = |id: &str| {
        if id == old_id { "OLD" }
        else if id == dormant_id { "DORMANT" }
        else if id == new_id { "NEW" }
        else { "other" }
    };
    let report = |order: &[matrix_domain::RoomScope], n: usize| {
        let names: Vec<&str> = order.iter().map(|s| label_of(s.room_id.as_str())).collect();
        eprintln!("{} order #{n}: {:?}", at("order"), names);
    };
    let snapshot = |seen: &Seen, what: &str| {
        for (label, id) in [("OLD", old_id), ("DORMANT", dormant_id), ("NEW", new_id)] {
            let Ok(id) = matrix_engine::sdk::ruma::RoomId::parse(id) else { continue };
            let Some(room) = client.get_room(&id) else {
                eprintln!("{} {what} {label}: room unknown to the client", at("sdk"));
                continue;
            };
            eprintln!(
                "{} {what} {label}: recency_stamp={:?} preview={:?}",
                at("sdk"),
                room.recency_stamp().map(u64::from),
                seen.rooms.get(id.as_str()).and_then(|r| r.info.preview.clone()),
            );
        }
    };
    macro_rules! pump {
        ($for:expr, $until:expr) => {{
            let end = tokio::time::Instant::now() + $for;
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(end) => break,
                    event = rx.recv() => match event.unwrap() {
                        RoomListEvent::Status(RoomSyncStatus::Ready) => seen.ready = true,
                        RoomListEvent::Order(order) => {
                            seen.orders += 1;
                            seen.order_ids = order.iter().map(|s| s.room_id.as_str().to_owned()).collect();
                            report(&order, seen.orders);
                        }
                        RoomListEvent::Diff(RoomListDiff::Upsert(r)) => {
                            if [old_id, dormant_id, new_id].contains(&r.scope.room_id.as_str()) {
                                eprintln!(
                                    "{} diff {}: unread={} preview={:?}",
                                    at("diff"),
                                    label_of(r.scope.room_id.as_str()),
                                    r.unread,
                                    r.info.preview,
                                );
                            }
                            seen.rooms.insert(r.scope.room_id.as_str().to_owned(), r);
                        }
                        RoomListEvent::Diff(RoomListDiff::Remove(s)) => { seen.rooms.remove(s.room_id.as_str()); }
                        _ => {}
                    },
                }
                if $until(seen) { break }
            }
        }};
    }
    // Phase 1: settle (Ready + one order + all three rooms projected).
    pump!(WAIT, |s: &Seen| {
        s.ready
            && s.orders >= 1
            && [old_id, dormant_id, new_id].iter().all(|id| s.rooms.contains_key(*id))
            && s.rooms.get(old_id).and_then(|r| r.info.preview.clone()).is_some()
            && s.rooms.get(new_id).and_then(|r| r.info.preview.clone()).is_some()
    });
    snapshot(seen, "settled");
    // Phase 2: idle control, 8s with no input.
    pump!(Duration::from_secs(8), |_: &Seen| false);
    snapshot(seen, "after-idle");
    let before_open = seen.order_ids.clone();
    // Phase 3: open the DORMANT room exactly like the app does.
    eprintln!(
        "{} opening DORMANT (receipts={})",
        at("open"),
        visibility.sees_last_message()
    );
    match opener.open_history(tag, visibility).await {
        Ok((page, _)) => eprintln!("{} open returned {} events", at("open"), page.events.len()),
        Err(e) => eprintln!("{} open FAILED: {e:?}", at("open")),
    }
    // Phase 4: watch the order for 25s after the open.
    pump!(Duration::from_secs(25), |_: &Seen| false);
    snapshot(seen, "after-open");
    assert_eq!(seen.order_ids, before_open, "opening a room must not reorder the list");
}

fn step(name: &str) {
    eprintln!("[adapter-probe] {name}");
}

fn suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{nanos:x}")
}

#[derive(Default)]
struct Seen {
    ready: bool,
    orders: usize,
    order_ids: Vec<String>,
    rooms: BTreeMap<String, RoomSummary>,
    activity: Vec<matrix_domain::RoomScope>,
}

/// Returns once Ready was seen and the room shows `preview` (and activity followed it).
async fn collect(
    rx: &mut matrix_engine::Receiver<RoomListEvent>,
    seen: &mut Seen,
    id: &str,
    preview: &str,
    need_activity: bool,
) {
    tokio::time::timeout(WAIT, async {
        loop {
            if seen.ready
                && seen
                    .rooms
                    .get(id)
                    .is_some_and(|r| r.info.preview.as_deref() == Some(preview))
                && (!need_activity || seen.activity.iter().any(|s| s.room_id.as_str() == id))
            {
                return;
            }
            match rx.recv().await.expect("adapter stream closed") {
                RoomListEvent::Diff(RoomListDiff::Upsert(r)) => {
                    if r.scope.room_id.as_str() == id && r.info.preview.as_deref() == Some(preview)
                    {
                        // Only activity after the wanted message counts.
                        seen.activity.clear();
                    }
                    seen.rooms.insert(r.scope.room_id.as_str().to_owned(), *r);
                }
                RoomListEvent::Diff(RoomListDiff::Remove(s)) => {
                    seen.rooms.remove(s.room_id.as_str());
                }
                RoomListEvent::Status(RoomSyncStatus::Ready) => seen.ready = true,
                RoomListEvent::Status(RoomSyncStatus::Connecting) => {}
                RoomListEvent::Status(_) => panic!("adapter reported a failure"),
                RoomListEvent::TimelineActivity(s) => seen.activity.push(s),
                RoomListEvent::Order(_) | RoomListEvent::Alert(_) => {}
            }
        }
    })
    .await
    .expect("adapter never showed the message")
}
