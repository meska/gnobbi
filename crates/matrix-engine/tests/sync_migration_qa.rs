//! Exploratory QA of the live sliding-sync path (`sliding::run`) on a disposable
//! homeserver: recency/order, previews, unread, spaces, invites, leaving. Never uses the
//! classic loop. Driven by the sync_migration_qa script (FIXTURE_AUTH_CONFIG, no secret here).
//! Each scenario prints `QA <name> PASS|FAIL detail` and the test fails if any scenario failed.
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

const WAIT: Duration = Duration::from_secs(45);

async fn login(config: &serde_json::Value, who: &str) -> Client {
    let mut builder = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .sliding_sync_version_builder(VersionBuilder::None);
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

#[derive(Default)]
struct State {
    ready: bool,
    failed: bool,
    rooms: BTreeMap<String, RoomSummary>,
    removed: Vec<String>,
    /// Last `RoomListEvent::Order`: room ids, most recent first.
    order: Vec<String>,
}

fn pos(st: &State, id: &str) -> Option<usize> {
    st.order.iter().position(|o| o == id)
}

/// Drives the adapter until `done(state)` holds or WAIT elapses; false on timeout.
async fn until(
    run: &mut (impl std::future::Future<Output = ()> + Unpin),
    rx: &mut matrix_engine::Receiver<RoomListEvent>,
    st: &mut State,
    done: impl Fn(&State) -> bool,
) -> bool {
    let waiting = async {
        while !done(st) {
            match rx.recv().await {
                Some(RoomListEvent::Diff(RoomListDiff::Upsert(r))) => {
                    st.rooms.insert(r.scope.room_id.as_str().to_owned(), r);
                }
                Some(RoomListEvent::Diff(RoomListDiff::Remove(s))) => {
                    st.removed.push(s.room_id.as_str().to_owned());
                    st.rooms.remove(s.room_id.as_str());
                }
                Some(RoomListEvent::Status(RoomSyncStatus::Ready)) => st.ready = true,
                Some(RoomListEvent::Status(RoomSyncStatus::Connecting)) => {}
                Some(RoomListEvent::Status(_)) => st.failed = true,
                Some(RoomListEvent::TimelineActivity(_) | RoomListEvent::Alert(_)) => {}
                Some(RoomListEvent::Order(order)) => {
                    st.order = order
                        .iter()
                        .map(|s| s.room_id.as_str().to_owned())
                        .collect();
                }
                None => return,
            }
        }
    };
    tokio::select! {
        _ = &mut *run => { eprintln!("adapter ended"); false }
        r = tokio::time::timeout(WAIT, waiting) => r.is_ok() && done(st),
    }
}

fn report(failed: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    eprintln!("QA {name} {} {detail}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        failed.push(name.to_owned());
    }
}

fn preview<'a>(st: &'a State, id: &str) -> Option<&'a str> {
    st.rooms.get(id).and_then(|r| r.info.preview.as_deref())
}

#[test]
#[ignore = "requires the sync_migration_qa script"]
fn qa_sliding_path_scenarios() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut failed = Vec::new();
    rt.block_on(async {
        let (alice, bob) = (login(&config, "alice").await, login(&config, "bob").await);
        let mk = |name: &str, invite: bool| {
            let mut req = assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some(name.to_owned()),
            });
            if invite {
                req.invite = vec![alice.user_id().unwrap().to_owned()];
            }
            req
        };
        // Two rooms alice has joined before the app starts (an "existing profile" state).
        let a = bob.create_room(mk("qa-a", true)).await.unwrap();
        let b = bob.create_room(mk("qa-b", true)).await.unwrap();
        alice.join_room_by_id(a.room_id()).await.unwrap();
        alice.join_room_by_id(b.room_id()).await.unwrap();
        let (ida, idb) = (a.room_id().to_string(), b.room_id().to_string());
        a.send(RoomMessageEventContent::text_plain("a1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        b.send(RoomMessageEventContent::text_plain("b1")).await.unwrap();

        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(AccountId::new(9), alice.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let mut st = State::default();

        // S1 first load: both rooms, previews, recency order b newer than a.
        let ok = until(&mut run, &mut rx, &mut st, |s| {
            s.ready && preview(s, &ida) == Some("a1") && preview(s, &idb) == Some("b1")
        })
        .await;
        report(&mut failed, "first_load_previews", ok, format!("ready={} a={:?} b={:?}", st.ready, preview(&st, &ida), preview(&st, &idb)));
        let ok = until(&mut run, &mut rx, &mut st, |s| matches!((pos(s, &idb), pos(s, &ida)), (Some(y), Some(x)) if y < x)).await;
        report(&mut failed, "recency_b_before_a", ok, format!("order={:?}", st.order));

        // S2 a new message in the older room moves it to the top (recency).
        tokio::time::sleep(Duration::from_millis(1100)).await;
        a.send(RoomMessageEventContent::text_plain("a2")).await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| preview(s, &ida) == Some("a2")).await;
        let ok = ok && until(&mut run, &mut rx, &mut st, |s| matches!((pos(s, &ida), pos(s, &idb)), (Some(x), Some(y)) if x < y)).await;
        report(&mut failed, "new_message_preview_and_recency", ok, format!("order={:?}", st.order));

        // S3 unread counter from the sliding delivery: 3 peer messages -> 3.
        let mut last_id = None;
        for t in ["u1", "u2", "u3"] {
            last_id = Some(b.send(RoomMessageEventContent::text_plain(t)).await.unwrap().response.event_id);
        }
        let ok = until(&mut run, &mut rx, &mut st, |s| s.rooms.get(&idb).is_some_and(|r| r.unread == 3)).await;
        report(&mut failed, "unread_counts_three", ok, format!("unread={:?}", st.rooms.get(&idb).map(|r| r.unread)));

        // S4 a read receipt from the user drops the counter to 0 through the same stream.
        if let (Some(room), Some(ev)) = (alice.get_room(b.room_id()), last_id) {
            use matrix_engine::sdk::ruma::api::client::receipt::create_receipt::v3::ReceiptType;
            use matrix_engine::sdk::ruma::events::receipt::ReceiptThread;
            room.send_single_receipt(ReceiptType::Read, ReceiptThread::Unthreaded, ev).await.unwrap();
        }
        let ok = until(&mut run, &mut rx, &mut st, |s| s.rooms.get(&idb).is_some_and(|r| r.unread == 0)).await;
        report(&mut failed, "unread_cleared_after_receipt", ok, format!("unread={:?}", st.rooms.get(&idb).map(|r| r.unread)));

        // S5 space with a child: is_space and the child link are projected.
        let space = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some("qa-space".to_owned()),
                creation_content: Some(
                    matrix_engine::sdk::ruma::serde::Raw::new(&serde_json::json!({"type":"m.space"}))
                        .unwrap()
                        .cast_unchecked(),
                ),
                invite: vec![alice.user_id().unwrap().to_owned()],
            }))
            .await
            .unwrap();
        let ids = space.room_id().to_string();
        let child = serde_json::json!({"via": ["matrix-desktop.test"]});
        space.send_state_event_raw("m.space.child", &ida, child).await.unwrap();
        alice.join_room_by_id(space.room_id()).await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| {
            s.rooms.get(&ids).is_some_and(|r| r.info.is_space && r.info.children.iter().any(|c| c.as_str() == ida))
        })
        .await;
        report(&mut failed, "space_with_child", ok, format!("space={:?}", st.rooms.get(&ids).map(|r| (r.info.is_space, r.info.children.len()))));

        // S6 invites from the SDK filter: received (listed as invited), accepted (joined), declined (removed).
        let inv = bob.create_room(mk("qa-invite", true)).await.unwrap();
        let idi = inv.room_id().to_string();
        let ok = until(&mut run, &mut rx, &mut st, |s| {
            s.rooms.get(&idi).is_some_and(|r| r.info.invited) && pos(s, &idi).is_some()
        })
        .await;
        report(&mut failed, "invite_received_listed_as_invited", ok, format!("room={:?}", st.rooms.get(&idi).map(|r| (&r.name, r.info.invited))));
        alice.get_room(inv.room_id()).unwrap().join().await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| s.rooms.get(&idi).is_some_and(|r| !r.info.invited)).await;
        report(&mut failed, "invite_accepted_becomes_joined", ok, format!("invited={:?}", st.rooms.get(&idi).map(|r| r.info.invited)));
        let no = bob.create_room(mk("qa-invite-no", true)).await.unwrap();
        let idn = no.room_id().to_string();
        let ok = until(&mut run, &mut rx, &mut st, |s| s.rooms.get(&idn).is_some_and(|r| r.info.invited)).await;
        alice.get_room(no.room_id()).unwrap().leave().await.unwrap();
        let ok = ok && until(&mut run, &mut rx, &mut st, |s| !s.rooms.contains_key(&idn) && pos(s, &idn).is_none()).await;
        report(&mut failed, "invite_declined_removed", ok, format!("listed={}", st.rooms.contains_key(&idn)));

        // S7 leaving removes the room.
        alice.get_room(a.room_id()).unwrap().leave().await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| !s.rooms.contains_key(&ida)).await;
        report(&mut failed, "leave_removes_room", ok, String::new());

        // S8 reconnection: freeze the server, time until the adapter says SyncFailed, then resume
        // and time until Ready again (the SDK detects a mute server after ~181-211 s).
        let pid = config["server_pid"].to_string();
        let kill = |sig: &str| std::process::Command::new("/usr/bin/kill").args([sig, &pid]).status().unwrap().success();
        assert!(kill("-STOP"));
        let t0 = std::time::Instant::now();
        let mut failed_after = None;
        st.failed = false;
        let wait_fail = tokio::time::timeout(Duration::from_secs(240), async {
            while !st.failed {
                tokio::select! {
                    _ = &mut run => return,
                    ev = rx.recv() => match ev {
                        Some(RoomListEvent::Status(RoomSyncStatus::SyncFailed)) => st.failed = true,
                        None => return,
                        _ => {}
                    },
                }
            }
        })
        .await;
        if wait_fail.is_ok() && st.failed {
            failed_after = Some(t0.elapsed().as_secs());
        }
        assert!(kill("-CONT"));
        let t1 = std::time::Instant::now();
        st.ready = false;
        let ready = until(&mut run, &mut rx, &mut st, |s| s.ready).await;
        b.send(RoomMessageEventContent::text_plain("post-resume")).await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| preview(s, &idb) == Some("post-resume")).await;
        eprintln!("QA reconnect_ready_event_after_resume OBSERVED {ready} (Ready is only re-emitted if the service left Running)");
        report(&mut failed, "reconnect_failure_visible_within_240s", matches!(failed_after, Some(n) if n <= 240), format!("syncfailed_after_s={failed_after:?}"));
        report(&mut failed, "reconnect_live_message_after_resume", ok, format!("ready_after_resume_s={}", t1.elapsed().as_secs()));

        // cleanup of what this run created
        for c in [&alice, &bob] {
            for r in c.rooms() {
                let _ = r.leave().await;
                let _ = r.forget().await;
            }
            let _ = c.matrix_auth().logout().await;
        }
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}

/// A profile whose store was filled by the old classic loop (plain `sync_once` rounds, the
/// removed loop) opens on the sliding path: saved rooms/previews come back and live
/// messages follow, with the sliding version unset as on a pre-migration client.
#[test]
#[ignore = "requires the sync_migration_qa script"]
fn qa_profile_filled_by_classic_loop_opens_on_sliding() {
    let config: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut failed = Vec::new();
    rt.block_on(async {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("old-profile");
        let build = || async {
            let mut b = Client::builder()
                .homeserver_url(config["url"].as_str().unwrap())
                .sliding_sync_version_builder(VersionBuilder::None)
                .sqlite_store(&dir, None);
            if let Some(ca) = config["ca"].as_str() {
                b = b.add_root_certificates(vec![Certificate::from_pem(&std::fs::read(ca).unwrap()).unwrap()]);
            }
            let c = b.build().await.unwrap();
            c.event_cache().subscribe().unwrap(); // as AccountStore::client does
            c
        };
        let bob = login(&config, "bob").await;
        let room = bob
            .create_room(assign!(CreateRoom::new(), {
                preset: Some(RoomPreset::PrivateChat),
                name: Some("old-profile-room".to_owned()),
                invite: vec![config["alice_mxid"].as_str().unwrap().to_string().try_into().unwrap()],
            }))
            .await
            .unwrap();
        let id = room.room_id().to_string();
        room.send(RoomMessageEventContent::text_plain("old message")).await.unwrap();

        // Old life of the profile: login + classic rounds until the preview is stored.
        let session = {
            let old = build().await;
            old.matrix_auth()
                .login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap())
                .send()
                .await
                .unwrap();
            old.join_room_by_id(room.room_id()).await.unwrap();
            let seen = tokio::time::timeout(WAIT, async {
                loop {
                    old.sync_once(matrix_sdk_sync_settings()).await.unwrap();
                    if old.get_room(room.room_id()).is_some_and(|r| !r.latest_event().is_none()) {
                        return;
                    }
                }
            })
            .await;
            report(&mut failed, "old_profile_filled_by_classic", seen.is_ok(), String::new());
            old.matrix_auth().session().unwrap()
        };

        // New life: same store directory, fresh client, sliding path.
        let new = build().await;
        new.restore_session(session).await.unwrap();
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(AccountId::new(11), new.clone());
        let mut st = State::default();
        // Like RoomListWorker: saved rooms from the store first, then the live path.
        for d in sync.saved_batch().await.unwrap() {
            if let RoomListDiff::Upsert(r) = d {
                st.rooms.insert(r.scope.room_id.as_str().to_owned(), r);
            }
        }
        // The preview is the SDK latest event: a store written by the old loop has none until sliding sync computes it.
        report(&mut failed, "old_profile_saved_rooms_before_sync", st.rooms.contains_key(&id), format!("preview={:?}", preview(&st, &id)));
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        // The SDK latest event of this room is alice's own join (it came after the message): no preview until a message arrives.
        let ok = until(&mut run, &mut rx, &mut st, |s| s.ready && s.rooms.contains_key(&id)).await;
        report(&mut failed, "old_profile_rooms_and_preview_on_sliding", ok, format!("ready={} preview={:?} name={:?} base_latest={:?} recency={:?}", st.ready, preview(&st, &id), st.rooms.get(&id).map(|r| r.name.clone()), new.get_room(room.room_id()).map(|r| (*r).latest_event()), new.get_room(room.room_id()).and_then(|r| r.recency_stamp())));
        room.send(RoomMessageEventContent::text_plain("after migration")).await.unwrap();
        let ok = until(&mut run, &mut rx, &mut st, |s| preview(s, &id) == Some("after migration")).await;
        report(&mut failed, "old_profile_follows_live_message", ok, format!("preview={:?}", preview(&st, &id)));
        let ok = pos(&st, &id).is_some();
        report(&mut failed, "old_profile_room_in_sdk_order", ok, format!("order={:?}", st.order));

        for c in [&new, &bob] {
            for r in c.rooms() {
                let _ = r.leave().await;
                let _ = r.forget().await;
            }
            let _ = c.matrix_auth().logout().await;
        }
    });
    assert!(failed.is_empty(), "failed scenarios: {failed:?}");
}

fn matrix_sdk_sync_settings() -> matrix_engine::sdk::config::SyncSettings {
    matrix_engine::sdk::config::SyncSettings::default().timeout(Duration::from_millis(200))
}
