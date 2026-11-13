//!: the account leaves a room from ANOTHER client (Element, phone) while Gnobbi runs, and while Gnobbi is
//! stopped. Does the row go away, and does it stay away after the restart on the kept store?
//! Driver: the room_mgmt_probe script with PROBE_TEST=leave_elsewhere, PROBE_SERVER=synapse|tuwunel.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    RoomListDiff, RoomListEvent, bounded,
    operations::join_room,
    room_sync::RoomSync,
    sdk::{
        Client, RoomState,
        reqwest::Certificate,
        ruma::{
            RoomId as SdkRoomId,
            api::client::{
                membership::leave_room::v3::Request as LeaveRequest,
                room::{
                    Visibility,
                    create_room::v3::{CreationContent, Request as CreateRoomRequest},
                },
            },
            assign,
            events::room::message::RoomMessageEventContent,
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sliding,
};
use std::{num::NonZeroUsize, path::Path, time::Duration};

async fn build(url: &str, ca: &[u8], store: &Path) -> Client {
    Client::builder()
        .homeserver_url(url)
        .add_root_certificates(vec![Certificate::from_pem(ca).unwrap()])
        .sqlite_store(store, None)
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .build()
        .await
        .unwrap()
}

/// What the row did over a window: how many joined upserts, and whether the LAST word was a Remove.
#[derive(Default, Debug)]
struct Trace {
    upserts: usize,
    removed_last: bool,
}

fn note(t: &mut Trace, ev: &RoomListEvent, scope: &RoomScope) {
    match ev {
        RoomListEvent::Diff(RoomListDiff::Upsert(s)) if s.scope == *scope && !s.info.invited => {
            t.upserts += 1;
            t.removed_last = false;
        }
        RoomListEvent::Diff(RoomListDiff::Remove(r)) if r == scope => t.removed_last = true,
        _ => {}
    }
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn a_room_left_from_another_client_goes_online_and_offline_and_stays_gone_after_restart() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let root = std::path::PathBuf::from(std::env::var("FIXTURE_RUN_ROOT").expect("FIXTURE_RUN_ROOT required"));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let login = |c: &Client, who: &str| {
            let (m, p) = (config[format!("{who}_mxid")].as_str().unwrap().to_owned(), config[format!("{who}_password")].as_str().unwrap().to_owned());
            let c = c.clone();
            async move { c.matrix_auth().login_username(&m, &p).send().await.unwrap() }
        };
        let a = build(&url, &ca, &root.join("alice-store")).await;
        login(&a, "alice").await;
        let b = build(&url, &ca, &root.join("bob-store")).await; // «Gnobbi»
        login(&b, "bob").await;
        let session = b.matrix_auth().session().expect("bob session");
        let other = build(&url, &ca, &root.join("bob-other-store")).await; // «Element / phone»
        login(&other, "bob").await;

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let create = |tag: &'static str| {
            let a = a.clone();
            async move {
                let mut content = CreationContent::new();
                content.federate = false;
                let r = a
                    .create_room(assign!(CreateRoomRequest::new(), {
                        name: Some(format!("gnobbi-test-{tag}-{nanos}")),
                        visibility: Visibility::Public,
                        creation_content: Some(Raw::new(&content).unwrap()),
                    }))
                    .await
                    .unwrap();
                r.send(RoomMessageEventContent::text_plain("prima")).await.unwrap();
                r
            }
        };
        let (online, offline, kept) = (create("online").await, create("offline").await, create("kept").await);
        let account = AccountId::new(2);
        let scope = |r: &matrix_engine::sdk::Room| RoomScope { account_id: account, room_id: RoomId::new(r.room_id().as_str()) };
        let (s_on, s_off, s_kept) = (scope(&online), scope(&offline), scope(&kept));
        let leave_elsewhere = |id: &SdkRoomId| {
            let other = other.clone();
            let id = id.to_owned();
            async move { other.send(LeaveRequest::new(id)).await.expect("leave from the other client") }
        };

        // ---- phase 1: Gnobbi online, the other client leaves `online` ----
        {
            let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
            let mut sync = RoomSync::from_client(account, b.clone());
            let run = sliding::run(&mut sync, &mut tx, || {});
            tokio::pin!(run);
            let steps = async {
                for r in [&online, &offline, &kept] {
                    join_room(&b, account, r.room_id().as_str(), &[]).await.unwrap();
                }
                let mut seen = (false, false, false);
                while !(seen.0 && seen.1 && seen.2) {
                    if let Some(RoomListEvent::Diff(RoomListDiff::Upsert(s))) = rx.recv().await
                        && !s.info.invited
                    {
                        seen.0 |= s.scope == s_on;
                        seen.1 |= s.scope == s_off;
                        seen.2 |= s.scope == s_kept;
                    }
                }
                leave_elsewhere(online.room_id()).await;
                let t0 = tokio::time::Instant::now();
                let mut t = Trace::default();
                let end = t0 + Duration::from_secs(30);
                while !t.removed_last {
                    let ev = tokio::time::timeout_at(end, rx.recv()).await.expect("PHASE1: no Remove 30 s after the leave from another client (row stays)").expect("engine stopped");
                    note(&mut t, &ev, &s_on);
                }
                println!("[leave_elsewhere] PHASE1 online: Remove after {:?}", t0.elapsed());
            };
            tokio::select! {
                _ = &mut run => panic!("engine stopped"),
                r = tokio::time::timeout(Duration::from_secs(120), steps) => r.expect("phase 1 timed out"),
            }
        }
        drop(b); // Gnobbi closed

        // ---- phase 2: Gnobbi off, the other client leaves `offline`; restart on the kept store ----
        leave_elsewhere(offline.room_id()).await;
        kept.send(RoomMessageEventContent::text_plain("mentre Gnobbi era chiuso")).await.unwrap();
        let b2 = build(&url, &ca, &root.join("bob-store")).await;
        b2.restore_session(session).await.unwrap();
        println!("[leave_elsewhere] store before restart sync: online={:?} offline={:?}", b2.get_room(online.room_id()).map(|r| r.state()), b2.get_room(offline.room_id()).map(|r| r.state()));
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(account, b2.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let steps = async {
            let (mut t_on, mut t_off, mut t_kept) = (Trace::default(), Trace::default(), Trace::default());
            let end = tokio::time::Instant::now() + Duration::from_secs(25);
            while let Ok(ev) = tokio::time::timeout_at(end, rx.recv()).await {
                let ev = ev.expect("engine stopped");
                note(&mut t_on, &ev, &s_on);
                note(&mut t_off, &ev, &s_off);
                note(&mut t_kept, &ev, &s_kept);
            }
            println!("[leave_elsewhere] PHASE2 restart: online {t_on:?} offline {t_off:?} kept {t_kept:?}");
            println!("[leave_elsewhere] sdk state after restart: online={:?} offline={:?}", b2.get_room(online.room_id()).map(|r| r.state()), b2.get_room(offline.room_id()).map(|r| r.state()));
            assert!(t_kept.upserts > 0, "the restarted list never showed the kept room: nothing proven");
            assert!(t_on.upserts == 0 || t_on.removed_last, "PHASE2: `online` (left while Gnobbi ran) is back as a row after restart");
            assert!(t_off.upserts == 0 || t_off.removed_last, "PHASE2: `offline` (left while Gnobbi was closed) stays as a row after restart");
            assert_eq!(b2.get_room(offline.room_id()).map(|r| r.state()), Some(RoomState::Left), "sdk never applied the leave");
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped"),
            r = tokio::time::timeout(Duration::from_secs(120), steps) => r.expect("restart run timed out"),
        }
        println!("[leave_elsewhere] ok");
    });
}
