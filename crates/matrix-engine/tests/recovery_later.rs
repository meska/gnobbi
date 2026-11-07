//! «Recupera chiavi» after sign-in, on a disposable homeserver (Synapse or Tuwunel), synthetic accounts only.
//! Device A (with backup) and O send; device B signs in later and DEFERS the recovery: old rows are UTD
//! (`HistoryEvent::utd`) and the recovery is not complete. B then runs the post-sign-in recovery with the
//! key: the rows become readable on the SAME timeline (no restart, no `clear_all_rooms`), the recovery is
//! complete and B's own device is signed by its owner. A session that is in no backup stays UTD.
//! Driver: PROBE_TARGET=recovery_later PROBE_TEST=recovery_later_reads_old_rows PROBE_SERVER=synapse|tuwunel \
//!   PROBE_FEATURES=sdk PROBE_EVIDENCE=artifacts/recovery-later python3 the timeline_probe script
#![cfg(feature = "sdk")]
#![recursion_limit = "512"]
use matrix_engine::{
    history::HistoryEvent,
    recovery::{keys_recovered, recover_all_with},
    room_sync::rows_for_test,
    sdk::{
        Client, Room,
        encryption::{BackupDownloadStrategy, EncryptionSettings},
        reqwest::Certificate,
        ruma::{
            OwnedRoomId,
            api::client::room::create_room::v3::{CreationContent, Request as CreateRoomRequest, RoomPreset},
            assign,
            events::{
                InitialStateEvent,
                room::{
                    encryption::RoomEncryptionEventContent,
                    history_visibility::{HistoryVisibility, RoomHistoryVisibilityEventContent},
                    message::RoomMessageEventContent,
                },
            },
            serde::Raw,
        },
        sliding_sync::VersionBuilder,
    },
    sdk_ui::{sync_service::SyncService, timeline::{RoomExt, Timeline}},
};
use std::time::{Duration, Instant};

async fn until<T>(secs: u64, mut f: impl AsyncFnMut() -> Option<T>) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = f().await {
            return Some(v);
        }
        if start.elapsed() > Duration::from_secs(secs) {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn login(config: &serde_json::Value, who: &str, name: &str, dir: &std::path::Path, strategy: BackupDownloadStrategy) -> Client {
    let ca = std::fs::read(config["ca"].as_str().unwrap()).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let c = Client::builder()
        .homeserver_url(config["url"].as_str().unwrap())
        .add_root_certificates(vec![Certificate::from_pem(&ca).unwrap()])
        .sliding_sync_version_builder(VersionBuilder::DiscoverNative)
        .sqlite_store(dir, None)
        .with_encryption_settings(EncryptionSettings { backup_download_strategy: strategy, ..Default::default() })
        .build()
        .await
        .unwrap();
    c.matrix_auth()
        .login_username(config[format!("{who}_mxid")].as_str().unwrap(), config[format!("{who}_password")].as_str().unwrap())
        .initial_device_display_name(name)
        .send()
        .await
        .unwrap();
    c.event_cache().subscribe().unwrap();
    c
}

async fn open(room: &Room) -> Timeline {
    let tl = room.timeline_builder().build().await.unwrap();
    for _ in 0..10 {
        if tl.paginate_backwards(50).await.unwrap() {
            break;
        }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    tl
}

fn row<'a>(rows: &'a [HistoryEvent], body: &str) -> Option<&'a HistoryEvent> {
    rows.iter().find(|r| r.body == body)
}

/// Rows of the live timeline now; waits until `done` holds or `secs` pass.
async fn rows_until(room: &Room, tl: &Timeline, secs: u64, done: impl Fn(&[HistoryEvent]) -> bool) -> Vec<HistoryEvent> {
    let start = Instant::now();
    loop {
        let rows = rows_for_test(room, tl).await;
        if done(&rows) || start.elapsed() > Duration::from_secs(secs) {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn creation(invite: &Client, shared_history: bool) -> CreateRoomRequest {
    let mut content = CreationContent::new();
    content.federate = false;
    let mut initial = vec![InitialStateEvent::with_empty_state_key(RoomEncryptionEventContent::with_recommended_defaults()).to_raw_any()];
    if shared_history {
        initial.push(InitialStateEvent::with_empty_state_key(RoomHistoryVisibilityEventContent::new(HistoryVisibility::Shared)).to_raw_any());
    }
    assign!(CreateRoomRequest::new(), {
        preset: Some(RoomPreset::PrivateChat),
        invite: if shared_history { vec![] } else { vec![invite.user_id().unwrap().to_owned()] },
        creation_content: Some(Raw::new(&content).unwrap()),
        initial_state: initial,
    })
}

async fn run(config: &serde_json::Value) {
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("stores-recovery-later");
    let a = login(config, "alice", "A", &home.join("a"), BackupDownloadStrategy::Manual).await;
    let o = login(config, "bob", "O", &home.join("o"), BackupDownloadStrategy::Manual).await;
    for c in [&a, &o] {
        c.encryption().bootstrap_cross_signing_if_needed(None).await.expect("bootstrap cross-signing");
    }
    let key = a.encryption().recovery().enable().wait_for_backups_to_upload().await.expect("recovery enable");
    let (sa, so) = (SyncService::builder(a.clone()).build().await.unwrap(), SyncService::builder(o.clone()).build().await.unwrap());
    sa.start().await;
    so.start().await;

    // R1: A and O both in the room, A's backup holds the sessions.
    let room_o = o.create_room(creation(&a, false)).await.unwrap();
    let r1: OwnedRoomId = room_o.room_id().to_owned();
    a.join_room_by_id(&r1).await.unwrap();
    let ra = until(40, async || a.get_room(&r1)).await.expect("room on A");
    until(40, async || (room_o.joined_user_ids().await.map(|m| m.len()).unwrap_or(0) == 2).then_some(())).await.expect("2 members");
    room_o.send(RoomMessageEventContent::text_plain("old-O")).await.unwrap();
    ra.send(RoomMessageEventContent::text_plain("old-A")).await.unwrap();
    a.encryption().backups().wait_for_steady_state().await.unwrap();

    // R2: same members, no message yet. Then A and O stop syncing: what O writes next reaches only A's
    // device (never synced, never backed up) and not B, whom O has not heard of.
    let room_o2 = o.create_room(creation(&a, false)).await.unwrap();
    let r2: OwnedRoomId = room_o2.room_id().to_owned();
    a.join_room_by_id(&r2).await.unwrap();
    until(40, async || a.get_room(&r2)).await.expect("R2 on A");
    until(40, async || (room_o2.joined_user_ids().await.map(|m| m.len()).unwrap_or(0) == 2).then_some(())).await.expect("2 members in R2");
    sa.stop().await;
    so.stop().await;

    // B: fresh device, signs in and defers the recovery (no key typed).
    let b = login(config, "alice", "B", &home.join("b"), BackupDownloadStrategy::AfterDecryptionFailure).await;
    let sb = SyncService::builder(b.clone()).build().await.unwrap();
    sb.start().await;
    let (rb1, rb2) = (until(40, async || b.get_room(&r1)).await.expect("R1 on B"), until(40, async || b.get_room(&r2)).await.expect("R2 on B"));
    room_o2.send(RoomMessageEventContent::text_plain("blind-O")).await.unwrap();
    rb1.send(RoomMessageEventContent::text_plain("own-B")).await.unwrap();
    let (t1, t2) = (open(&rb1).await, open(&rb2).await);

    let rows = rows_until(&rb1, &t1, 20, |r| row(r, "own-B").is_some()).await;
    for label in ["old-O", "old-A"] {
        assert!(row(&rows, label).is_none(), "{label} must not be readable before the recovery");
    }
    let utd: Vec<_> = rows.iter().filter(|r| r.utd).collect();
    assert_eq!(utd.len(), 2, "the two old messages are UTD rows");
    assert!(utd.iter().all(|r| r.body == "Cifrato · chiavi non disponibili" && !r.status));
    assert!(row(&rows, "own-B").is_some_and(|r| !r.utd), "a readable message is not UTD");
    assert!(rows.iter().any(|r| r.status), "room has status rows");
    assert!(rows.iter().filter(|r| r.status).all(|r| !r.utd), "status rows are not UTD");
    assert!(!keys_recovered(&b).await, "deferred recovery is not complete");

    // The new command path: recovery key, B already syncing.
    recover_all_with(b.clone(), zeroize::Zeroizing::new(key), false).await.expect("recovery with the key");
    let rows = rows_until(&rb1, &t1, 40, |r| r.iter().all(|x| !x.utd)).await;
    assert!(rows.iter().all(|r| !r.utd), "no UTD row left in R1 after the recovery, same timeline, no restart");
    for label in ["old-O", "old-A"] {
        assert!(row(&rows, label).is_some(), "{label} readable after the recovery");
    }
    assert!(keys_recovered(&b).await, "recovery complete");
    let own = b.encryption().get_own_device().await.unwrap().expect("own device");
    assert!(own.is_cross_signed_by_owner(), "own device signed by its owner");

    // Complete recovery, session in no backup: the row stays UTD (so no action is offered).
    let rows2 = rows_until(&rb2, &t2, 10, |r| r.iter().any(|x| x.utd)).await;
    assert!(rows2.iter().any(|r| r.utd), "a session outside the backup stays UTD");
    assert!(keys_recovered(&b).await);
    sb.stop().await;
}

#[test]
#[ignore = "requires the timeline_probe script"]
fn recovery_later_reads_old_rows() {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(std::env::var("FIXTURE_AUTH_CONFIG").unwrap()).unwrap()).unwrap();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(run(&config));
}
