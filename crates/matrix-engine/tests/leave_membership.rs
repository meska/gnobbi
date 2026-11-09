//! «Esci» on a room: server membership, the room list right after and in late syncs, and a restart on the
//! kept store. Same disposable-homeserver driver as room_join (the room_mgmt_probe script, PROBE_TEST=leave_membership,
//! PROBE_SERVER=synapse|tuwunel). Two synthetic accounts: A creates two public rooms, B joins both, leaves one.
#![recursion_limit = "512"]
#![cfg(feature = "sdk")]
use matrix_domain::{AccountId, RoomId, RoomScope};
use matrix_engine::{
    RoomListDiff, RoomListEvent, bounded,
    operations::{OperationError, join_room, leave_room},
    room_sync::RoomSync,
    sdk::{
        Client,
        reqwest::Certificate,
        ruma::{
            RoomId as SdkRoomId,
            api::client::{
                room::{
                    Visibility,
                    create_room::v3::{CreationContent, Request as CreateRoomRequest},
                },
                state::get_state_event_for_key::v3::Request as StateRequest,
            },
            assign,
            events::{StateEventType, room::message::RoomMessageEventContent},
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

/// Our own `m.room.member` content, read from the server (not from the SDK's local state).
async fn server_membership(client: &Client, room: &SdkRoomId) -> String {
    let me = client.user_id().unwrap().to_string();
    let r = client.send(StateRequest::new(room.to_owned(), StateEventType::RoomMember, me)).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(r.event_or_content.get()).unwrap();
    json["membership"].as_str().unwrap_or("?").to_owned()
}

#[test]
#[ignore = "requires the room_mgmt_probe script"]
fn a_confirmed_leave_stays_gone_in_late_syncs_and_after_a_restart_with_the_kept_store() {
    let path = std::env::var("FIXTURE_AUTH_CONFIG").expect("FIXTURE_AUTH_CONFIG required");
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    let url = config["url"].as_str().unwrap().to_owned();
    let store = std::path::PathBuf::from(std::env::var("FIXTURE_RUN_ROOT").expect("FIXTURE_RUN_ROOT required")).join("bob-store");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let a = {
            let c = build(&url, &ca, &store.with_file_name("alice-store")).await;
            c.matrix_auth().login_username(config["alice_mxid"].as_str().unwrap(), config["alice_password"].as_str().unwrap()).send().await.unwrap();
            c
        };
        let b = build(&url, &ca, &store).await;
        b.matrix_auth().login_username(config["bob_mxid"].as_str().unwrap(), config["bob_password"].as_str().unwrap()).send().await.unwrap();
        let session = b.matrix_auth().session().expect("bob session");

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let create = |name: String| {
            let a = a.clone();
            async move {
                let mut content = CreationContent::new();
                content.federate = false;
                a.create_room(assign!(CreateRoomRequest::new(), {
                    name: Some(name),
                    visibility: Visibility::Public,
                    creation_content: Some(Raw::new(&content).unwrap()),
                }))
                .await
                .unwrap()
            }
        };
        let (gone, kept) = (create(format!("gnobbi-test-leave-{nanos}")).await, create(format!("gnobbi-test-keep-{nanos}")).await);
        let account = AccountId::new(2);
        let scope = |id: &str| RoomScope { account_id: account, room_id: RoomId::new(id) };
        let (gone_scope, kept_scope) = (scope(gone.room_id().as_str()), scope(kept.room_id().as_str()));
        gone.send(RoomMessageEventContent::text_plain("prima del leave")).await.unwrap();

        // ---- first run: join both, leave one, watch the list for a while ----
        {
            let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
            let mut sync = RoomSync::from_client(account, b.clone());
            let run = sliding::run(&mut sync, &mut tx, || {});
            tokio::pin!(run);
            let steps = async {
                assert_eq!(join_room(&b, account, gone.room_id().as_str(), &[]).await.unwrap(), gone_scope);
                assert_eq!(join_room(&b, account, kept.room_id().as_str(), &[]).await.unwrap(), kept_scope);
                let (mut saw_gone, mut saw_kept) = (false, false);
                while !(saw_gone && saw_kept) {
                    if let Some(RoomListEvent::Diff(RoomListDiff::Upsert(s))) = rx.recv().await
                        && !s.info.invited
                    {
                        saw_gone |= s.scope == gone_scope;
                        saw_kept |= s.scope == kept_scope;
                    }
                }

                // The engine answers Ok only when the server itself says «leave».
                leave_room(&b, account, &gone_scope).await.expect("leave");
                assert_eq!(server_membership(&b, gone.room_id()).await, "leave", "Ok without the server having left");
                assert_eq!(server_membership(&b, kept.room_id()).await, "join");

                // Updates already queued before the leave may still be delivered; the projection then
                // drops the room with a `Remove`. Count the stale ones (the shell applies them after
                // `RoomLeft`) and wait for that `Remove`.
                let mut stale = 0;
                let removed = tokio::time::Instant::now();
                loop {
                    match tokio::time::timeout(Duration::from_secs(30), rx.recv()).await.expect("no Remove for the left room in 30 s").expect("engine stopped") {
                        RoomListEvent::Diff(RoomListDiff::Remove(r)) if r == gone_scope => break,
                        RoomListEvent::Diff(RoomListDiff::Upsert(s)) if s.scope == gone_scope && !s.info.invited => stale += 1,
                        _ => {}
                    }
                }
                println!("[leave_membership] stale upserts before Remove: {stale}; Remove after {:?}", removed.elapsed());

                // Late syncs: both rooms get traffic; the left room must not come back as a joined row,
                // the other one must (proof that the sync is alive and delivering).
                let mut kept_updates = 0;
                let end = tokio::time::Instant::now() + Duration::from_secs(12);
                let mut n = 0;
                loop {
                    n += 1;
                    if tokio::time::Instant::now() >= end {
                        break;
                    }
                    gone.send(RoomMessageEventContent::text_plain(format!("dopo il leave {n}"))).await.unwrap();
                    kept.send(RoomMessageEventContent::text_plain(format!("altra stanza {n}"))).await.unwrap();
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let Ok(event) = tokio::time::timeout_at(end, rx.recv()).await else { break };
                    match event.expect("engine stopped") {
                        RoomListEvent::Diff(RoomListDiff::Upsert(s)) if !s.info.invited => {
                            assert_ne!(s.scope, gone_scope, "the left room came back in a late sync");
                            kept_updates += usize::from(s.scope == kept_scope);
                        }
                        _ => {}
                    }
                }
                println!("[leave_membership] late loop done, kept updates {kept_updates}");
                assert!(kept_updates > 0, "no late sync reached the list: the check above proves nothing");
                // The SDK keeps the room as Left (history stays in the store); a second leave is refused.
                assert_eq!(b.get_room(gone.room_id()).map(|r| r.state()), Some(matrix_engine::sdk::RoomState::Left));
                assert_eq!(leave_room(&b, account, &gone_scope).await, Err(OperationError::Unavailable));
            };
            tokio::select! {
                _ = &mut run => panic!("engine stopped before the steps ended"),
                r = tokio::time::timeout(Duration::from_secs(180), steps) => r.expect("first run did not finish in 180 s"),
            }
        }
        drop(b);

        // ---- restart: new client on the same store, rebuilt list ----
        let b2 = build(&url, &ca, &store).await;
        b2.restore_session(session).await.unwrap();
        let (mut tx, mut rx) = bounded(NonZeroUsize::new(256).unwrap());
        let mut sync = RoomSync::from_client(account, b2.clone());
        let run = sliding::run(&mut sync, &mut tx, || {});
        tokio::pin!(run);
        let steps = async {
            let mut kept_seen = false;
            let end = tokio::time::Instant::now() + Duration::from_secs(15);
            while let Ok(event) = tokio::time::timeout_at(end, rx.recv()).await {
                if tokio::time::Instant::now() >= end {
                    break;
                }
                if let RoomListEvent::Diff(RoomListDiff::Upsert(s)) = event.expect("engine stopped")
                    && !s.info.invited
                {
                    assert_ne!(s.scope, gone_scope, "the left room is back after a restart");
                    kept_seen |= s.scope == kept_scope;
                }
            }
            assert!(kept_seen, "the restarted list never showed the room that was not left");
            assert_eq!(b2.get_room(gone.room_id()).map(|r| r.state()), Some(matrix_engine::sdk::RoomState::Left), "store lost the left room");
            assert_eq!(server_membership(&b2, gone.room_id()).await, "leave");
        };
        tokio::select! {
            _ = &mut run => panic!("engine stopped before the steps ended"),
            r = tokio::time::timeout(Duration::from_secs(120), steps) => r.expect("restart run did not finish in 120 s"),
        }
        println!("[leave_membership] left {} kept {} ok", gone_scope.room_id.as_str(), kept_scope.room_id.as_str());
    });
}
